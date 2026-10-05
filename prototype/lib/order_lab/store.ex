defmodule OrderLab.Store do
  use GenServer
  alias Exqlite.Sqlite3, as: SQL

  def start_link(_), do: GenServer.start_link(__MODULE__, nil, name: __MODULE__)
  def snapshot, do: GenServer.call(__MODULE__, :snapshot)
  def detail(id), do: GenServer.call(__MODULE__, {:detail, id})
  def order(id), do: GenServer.call(__MODULE__, {:order, id})

  def reject(input, status, code, message, method, path),
    do: GenServer.call(__MODULE__, {:reject, input, status, code, message, method, path})

  def deliver, do: GenServer.call(__MODULE__, :deliver, 30_000)
  def retry_email(id), do: GenServer.call(__MODULE__, {:retry_email, id})

  def route(method, path), do: GenServer.call(__MODULE__, {:route, method, path})

  def execute(endpoint, input, path, key \\ nil),
    do: GenServer.call(__MODULE__, {:execute, endpoint, input, path, key}, 30_000)

  def mqtt_publish(topic, payload, retain),
    do: GenServer.call(__MODULE__, {:mqtt_publish, topic, payload, retain})

  def websocket(path), do: GenServer.call(__MODULE__, {:websocket, path})

  def subscription(endpoint, credentials, source, params, pid, action, latest, keys),
    do:
      GenServer.call(
        __MODULE__,
        {:subscription, endpoint, credentials, source, params, pid, action, latest, keys}
      )

  def authenticate_websocket(endpoint, input),
    do: GenServer.call(__MODULE__, {:ws_authenticate, endpoint, input})

  def authorize_websocket(endpoint, credentials, source, params),
    do: GenServer.call(__MODULE__, {:ws_authorize, endpoint, credentials, source, params})

  def file(id), do: GenServer.call(__MODULE__, {:file, id})
  def resume_request(id), do: GenServer.call(__MODULE__, {:resume_request, id}, 30_000)
  def retry_mqtt(id), do: GenServer.call(__MODULE__, {:retry_mqtt, id})
  def deliver_mqtt, do: GenServer.call(__MODULE__, :deliver_mqtt, 30_000)
  def mqtt_retained, do: GenServer.call(__MODULE__, :mqtt_retained)

  def init(_) do
    path = System.get_env("DATABASE_PATH", "data/order_lab.sqlite3")
    File.mkdir_p!(Path.dirname(path))
    {:ok, db} = SQL.open(path)
    exec!(db, "PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")

    exec!(db, """
    CREATE TABLE IF NOT EXISTS products (
      id TEXT PRIMARY KEY, name TEXT NOT NULL, price_cents INTEGER NOT NULL CHECK(price_cents >= 0),
      stock INTEGER NOT NULL CHECK(stock >= 0));
    CREATE TABLE IF NOT EXISTS users (
      id TEXT PRIMARY KEY, name TEXT NOT NULL, email TEXT NOT NULL, country TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS requests (
      id TEXT PRIMARY KEY, created_at TEXT NOT NULL, method TEXT NOT NULL, path TEXT NOT NULL,
      input_json TEXT NOT NULL, status TEXT NOT NULL, http_status INTEGER,
      response_json TEXT, order_id TEXT, error_code TEXT, duration_ms INTEGER,
      idempotency_key TEXT, input_hash TEXT, replayed_from TEXT);
    CREATE TABLE IF NOT EXISTS flow_checkpoints(request_id TEXT PRIMARY KEY REFERENCES requests(id),fingerprint TEXT NOT NULL,value TEXT NOT NULL,created_at TEXT NOT NULL);
    CREATE INDEX IF NOT EXISTS requests_idempotency ON requests(idempotency_key);
    CREATE TABLE IF NOT EXISTS orders (
      id TEXT PRIMARY KEY, user_id TEXT NOT NULL REFERENCES users(id),
      request_id TEXT NOT NULL REFERENCES requests(id), created_at TEXT NOT NULL,
      items_json TEXT NOT NULL, total_cents INTEGER NOT NULL CHECK(total_cents > 0),
      payment_method TEXT NOT NULL, payment_url TEXT NOT NULL DEFAULT '',
      status TEXT NOT NULL DEFAULT 'awaiting_payment');
    CREATE TABLE IF NOT EXISTS plugin_calls (
      id TEXT PRIMARY KEY, request_id TEXT NOT NULL REFERENCES requests(id), order_id TEXT,
      plugin TEXT NOT NULL, operation TEXT NOT NULL, created_at TEXT NOT NULL,
      duration_ms INTEGER NOT NULL, status TEXT NOT NULL, input_json TEXT NOT NULL,
      output_json TEXT NOT NULL, attempt INTEGER NOT NULL);
    CREATE TABLE IF NOT EXISTS email_jobs (
      id TEXT PRIMARY KEY, order_id TEXT NOT NULL REFERENCES orders(id),
      request_id TEXT NOT NULL REFERENCES requests(id), input_json TEXT NOT NULL,
      state TEXT NOT NULL, attempts INTEGER NOT NULL DEFAULT 0,
      max_attempts INTEGER NOT NULL DEFAULT 3, retry_delay_ms INTEGER NOT NULL DEFAULT 1000,
      final_disposition TEXT NOT NULL DEFAULT 'retain', next_at INTEGER NOT NULL,
      error_json TEXT, result_json TEXT);
    """)

    exec!(db, """
    CREATE TABLE IF NOT EXISTS runtime_meta(key TEXT PRIMARY KEY,value TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS flow_seed_keys(table_name TEXT NOT NULL,id TEXT NOT NULL,PRIMARY KEY(table_name,id));
    CREATE TABLE IF NOT EXISTS flow_records(table_name TEXT NOT NULL,id TEXT NOT NULL,value_json TEXT NOT NULL,PRIMARY KEY(table_name,id));
    CREATE TABLE IF NOT EXISTS flow_jobs(
      id TEXT PRIMARY KEY,order_id TEXT,request_id TEXT NOT NULL REFERENCES requests(id),operation TEXT NOT NULL,
      input_json TEXT NOT NULL,state TEXT NOT NULL,attempts INTEGER NOT NULL DEFAULT 0,
      max_attempts INTEGER NOT NULL,retry_delay_ms INTEGER NOT NULL,final_disposition TEXT NOT NULL,
      next_at INTEGER NOT NULL,error_json TEXT,result_json TEXT);
    INSERT OR IGNORE INTO flow_jobs SELECT id,order_id,request_id,'Email.send_confirmation',input_json,state,attempts,max_attempts,retry_delay_ms,final_disposition,next_at,error_json,result_json FROM email_jobs;
    CREATE TABLE IF NOT EXISTS flow_files(id TEXT PRIMARY KEY,value_json TEXT NOT NULL,request_id TEXT NOT NULL REFERENCES requests(id),created_at TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS mqtt_outbox(id TEXT PRIMARY KEY,request_id TEXT NOT NULL REFERENCES requests(id),source TEXT NOT NULL,params_json TEXT NOT NULL,topic TEXT NOT NULL,payload_json TEXT NOT NULL,retained INTEGER NOT NULL,state TEXT NOT NULL,created_at TEXT NOT NULL,accepted_at TEXT,sent_at TEXT,error TEXT);
    CREATE TABLE IF NOT EXISTS mqtt_messages(id INTEGER PRIMARY KEY AUTOINCREMENT,source TEXT NOT NULL,topic TEXT NOT NULL,payload_json TEXT NOT NULL,received_at TEXT NOT NULL,retained INTEGER NOT NULL);
    CREATE INDEX IF NOT EXISTS mqtt_source_topic ON mqtt_messages(source,topic,id);
    """)

    effects = OrderLab.EffectJournal.open!(path <> ".effects")
    journal_identity = OrderLab.EffectJournal.identity(effects)

    case query!(db, "SELECT value FROM runtime_meta WHERE key='effect_journal'") do
      [] ->
        write!(db, "INSERT INTO runtime_meta(key,value) VALUES ('effect_journal',?)", [
          journal_identity
        ])

      [%{"value" => ^journal_identity}] ->
        :ok

      _ ->
        SQL.close(effects)
        SQL.close(db)
        raise "External operation journal does not match the main database"
    end

    program = OrderLab.Workflow.compile!()
    exec!(db, "BEGIN IMMEDIATE")

    try do
      host =
        OrderLab.Language.Native.host(db, program, %{}, fn _, _, _, _ ->
          raise("SEED cannot invoke plugins")
        end)

      Enum.each(program.seeds, fn seed ->
        existing =
          OrderLab.Language.Native.source(db, program, seed.table, now())
          |> MapSet.new(& &1["id"])

        Enum.each(seed.value, fn row ->
          seeded =
            query!(db, "SELECT id FROM flow_seed_keys WHERE table_name=? AND id=?", [
              seed.table,
              row["id"]
            ])

          if seeded == [] do
            unless MapSet.member?(existing, row["id"]),
              do: host.(:insert, %{table: seed.table, value: row})

            write!(db, "INSERT INTO flow_seed_keys(table_name,id) VALUES (?,?)", [
              seed.table,
              row["id"]
            ])
          end
        end)
      end)

      exec!(db, "COMMIT")
    rescue
      error ->
        SQL.execute(db, "ROLLBACK")
        reraise error, __STACKTRACE__
    end

    {:ok, %{db: db, effects: effects, steps: program}, {:continue, :recover}}
  end

  def handle_continue(:recover, state) do
    query!(
      state.db,
      "SELECT c.request_id FROM flow_checkpoints c JOIN requests r ON r.id=c.request_id WHERE r.status='running' ORDER BY r.rowid"
    )
    |> Enum.each(fn row -> recover_request(state, row["request_id"]) end)

    {:noreply, state}
  end

  def handle_call({:resume_request, id}, _, state),
    do: {:reply, recover_request(state, id), state}

  def handle_call({:websocket, path}, _, state),
    do: {:reply, Enum.find(state.steps.websockets, &(&1.path == path)), state}

  def handle_call({:ws_authenticate, endpoint, input}, _, state) do
    result =
      websocket_result(fn ->
        OrderLab.WebSocketPolicy.authenticate!(state.db, state.steps, endpoint, input)
      end)

    {:reply, result, state}
  end

  def handle_call({:ws_authorize, endpoint, credentials, source, params}, _, state) do
    result =
      websocket_result(fn ->
        OrderLab.WebSocketPolicy.subscription!(
          state.db,
          state.steps,
          endpoint,
          credentials,
          source,
          params
        )
      end)

    {:reply, result, state}
  end

  def handle_call(
        {:subscription, endpoint, credentials, name, params, pid, action, latest, keys},
        _,
        state
      ) do
    result =
      try do
        source = Map.fetch!(state.steps.mqtt, name)
        type = {:record, Map.new(source.params, &{&1.name, &1.type})}
        params = OrderLab.Language.Types.validate!(params, type)
        values = Enum.map(source.params, &params[&1.name])

        if Enum.any?(values, &String.contains?(&1, ["/", "+", "#", "\0"])),
          do: raise("Invalid topic parameter")

        key = {name, values}

        if action == "subscribe",
          do:
            OrderLab.WebSocketPolicy.subscription!(
              state.db,
              state.steps,
              endpoint,
              credentials,
              name,
              params
            )

        last =
          if latest and action == "subscribe" do
            case OrderLab.Language.Native.source(state.db, state.steps, key, now())
                 |> List.last() do
              nil ->
                nil

              row ->
                %{
                  "type" => "message",
                  "source" => name,
                  "params" => params,
                  "payload" => Map.delete(row, "received_at"),
                  "received_at" => row["received_at"]
                }
            end
          end

        if action == "subscribe" do
          if MapSet.size(keys) >= 16 and not MapSet.member?(keys, key),
            do: raise("At most 16 subscriptions are allowed")

          OrderLab.PubSub.subscribe(pid, key)
        else
          OrderLab.PubSub.unsubscribe(pid, key)
        end

        {:ok, key, params, last}
      rescue
        e in OrderLab.Language.Failure -> {:error, e.code, e.message}
        _ -> {:error, "invalid_subscription", "Invalid subscription parameters"}
      end

    {:reply, result, state}
  end

  def handle_call({:file, id}, _, state) do
    result =
      try do
        OrderLab.Files.read(state.db, %{"id" => id}, %{})
      rescue
        error ->
          {:error, %{"code" => "invalid_stored_file", "message" => Exception.message(error)}}
      end

    {:reply, result, state}
  end

  def handle_call({:route, method, path}, _, state) do
    route =
      Enum.find_value(state.steps.endpoints, fn endpoint ->
        if endpoint.method == method do
          expected = String.split(endpoint.path, "/", trim: true)
          actual = String.split(path, "/", trim: true)

          if length(expected) == length(actual) do
            params =
              Enum.zip(expected, actual)
              |> Enum.reduce_while(%{}, fn
                {":" <> name, value}, params -> {:cont, Map.put(params, name, URI.decode(value))}
                {same, same}, params -> {:cont, params}
                _, _ -> {:halt, nil}
              end)

            if params, do: {endpoint, params}
          end
        end
      end)

    {:reply, route, state}
  end

  def handle_call({:mqtt_publish, topic, payload, retain}, _, state) do
    result =
      try do
        definition =
          Enum.find_value(state.steps.mqtt, fn {_name, source} ->
            template = String.split(source.topic, "/")
            parts = String.split(topic, "/")

            if length(template) == length(parts) do
              values =
                Enum.zip(template, parts)
                |> Enum.reduce_while(%{}, fn {pattern, part}, acc ->
                  case Regex.run(~r/^\{([A-Za-z_][A-Za-z_0-9]*)\}$/, pattern) do
                    [_, param] -> {:cont, Map.put(acc, param, part)}
                    nil -> if pattern == part, do: {:cont, acc}, else: {:halt, nil}
                  end
                end)

              if values, do: {source, values}
            end
          end)

        unless definition,
          do: raise(OrderLab.Language.Error, message: "Topic is not declared", stage: :validation)

        {source, values} = definition

        Enum.each(source.params, fn param ->
          OrderLab.Language.Types.validate!(values[param.name], param.type)
        end)

        if retain and payload == "" do
          write!(state.db, "UPDATE mqtt_messages SET retained=0 WHERE topic=?", [topic])
        else
          value = Jason.decode!(payload)
          value = OrderLab.Language.Types.validate!(value, source.payload)

          exec!(state.db, "BEGIN IMMEDIATE")

          if retain,
            do: write!(state.db, "UPDATE mqtt_messages SET retained=0 WHERE topic=?", [topic])

          timestamp = now()

          write!(
            state.db,
            "INSERT INTO mqtt_messages(source,topic,payload_json,received_at,retained) VALUES (?,?,?,?,?)",
            [source.name, topic, json(value), timestamp, if(retain, do: 1, else: 0)]
          )

          exec!(state.db, "COMMIT")
          key = {source.name, Enum.map(source.params, &values[&1.name])}

          OrderLab.PubSub.broadcast(key, %{
            "type" => "message",
            "source" => source.name,
            "params" => values,
            "payload" => value,
            "received_at" => timestamp
          })

          Enum.filter(state.steps.endpoints, &(&1.method == "MQTT" and &1.path == source.name))
          |> Enum.each(fn endpoint ->
            # Transport acceptance records the message even if the business scenario fails.
            handle_call(
              {:execute, endpoint, Map.put(values, "message", value), topic, nil},
              nil,
              state
            )
          end)

          # Preserve the latest value even when it has aged beyond history retention.
          threshold =
            DateTime.utc_now()
            |> DateTime.add(-source.history_ms, :millisecond)
            |> DateTime.to_iso8601()

          write!(
            state.db,
            "DELETE FROM mqtt_messages WHERE source=? AND topic=? AND received_at<? AND retained=0 AND id!=(SELECT MAX(id) FROM mqtt_messages WHERE source=? AND topic=?)",
            [source.name, topic, threshold, source.name, topic]
          )
        end

        :ok
      rescue
        e ->
          SQL.execute(state.db, "ROLLBACK")
          {:error, Exception.message(e)}
      end

    {:reply, result, state}
  end

  def handle_call(:deliver_mqtt, _, state) do
    if Process.whereis(OrderLab.MQTT) do
      case query!(
             state.db,
             "SELECT * FROM mqtt_outbox WHERE state IN ('queued','accepted') ORDER BY rowid LIMIT 1"
           ) do
        [job] -> deliver_mqtt_job(state, job)
        [] -> :ok
      end
    end

    {:reply, :ok, state}
  end

  def handle_call({:retry_mqtt, id}, _, state) do
    result =
      case query!(state.db, "SELECT accepted_at FROM mqtt_outbox WHERE id=? AND state='failed'", [
             id
           ]) do
        [job] ->
          status = if job["accepted_at"], do: "accepted", else: "queued"
          write!(state.db, "UPDATE mqtt_outbox SET state=?,error=NULL WHERE id=?", [status, id])
          {:ok, status}

        [] ->
          :not_found
      end

    {:reply, result, state}
  end

  def handle_call(:mqtt_retained, _, state) do
    rows =
      query!(
        state.db,
        "SELECT source,topic,payload_json FROM mqtt_messages WHERE retained=1 ORDER BY id"
      )
      |> Enum.filter(fn row ->
        try do
          source = Map.fetch!(state.steps.mqtt, row["source"])
          template = String.split(source.topic, "/")
          parts = String.split(row["topic"], "/")
          unless length(template) == length(parts), do: raise("Topic contract changed")

          values =
            Enum.zip(template, parts)
            |> Enum.reduce(%{}, fn {pattern, value}, values ->
              case Regex.run(~r/^\{([A-Za-z_][A-Za-z_0-9]*)\}$/, pattern) do
                [_, name] -> Map.put(values, name, value)
                nil -> if pattern == value, do: values, else: raise("Topic contract changed")
              end
            end)

          parameters = Enum.map(source.params, &Map.fetch!(values, &1.name))
          OrderLab.Language.Native.mqtt_target!(state.steps, source.name, parameters)
          OrderLab.Language.Types.validate!(Jason.decode!(row["payload_json"]), source.payload)
          true
        rescue
          _ -> false
        end
      end)
      |> Enum.map(&Map.take(&1, ["topic", "payload_json"]))

    {:reply, rows, state}
  end

  def handle_call({:reject, input, status, code, message, method, path}, _, %{db: db} = state) do
    request_id = id("req")
    response = error(code, message) |> Map.put("request_id", request_id)

    write!(
      db,
      "INSERT INTO requests(id,created_at,method,path,input_json,status,http_status,response_json,error_code,duration_ms) VALUES (?,?,?,?,?,'failed',?,?,?,0)",
      [request_id, now(), method, path, json(input), status, json(response), code]
    )

    {:reply, {status, response}, state}
  end

  def handle_call(:snapshot, _, %{db: db} = state) do
    result = %{
      "products" => query!(db, "SELECT * FROM products ORDER BY id"),
      "users" => query!(db, "SELECT * FROM users ORDER BY id"),
      "orders" =>
        query!(
          db,
          "SELECT o.*, u.name user_name FROM orders o JOIN users u ON u.id=o.user_id ORDER BY o.created_at DESC"
        )
        |> Enum.map(&decode(&1, ["items_json"])),
      "requests" =>
        query!(db, "SELECT * FROM requests ORDER BY created_at DESC LIMIT 100")
        |> Enum.map(&decode(&1, ["input_json", "response_json"])),
      "plugin_calls" =>
        query!(db, "SELECT * FROM plugin_calls ORDER BY created_at DESC LIMIT 200")
        |> Enum.map(&decode(&1, ["input_json", "output_json"])),
      "email_jobs" =>
        query!(db, "SELECT * FROM flow_jobs ORDER BY rowid DESC")
        |> Enum.map(&decode(&1, ["input_json", "error_json", "result_json"])),
      "workflow" => state.steps.source,
      "workflow_path" => state.steps.file,
      "websocket" => OrderLab.PubSub.stats(),
      "external_operations" => OrderLab.EffectJournal.list(state.effects),
      "mqtt_outbox" =>
        query!(db, "SELECT * FROM mqtt_outbox ORDER BY rowid DESC LIMIT 100")
        |> Enum.map(&decode(&1, ["params_json", "payload_json"]))
    }

    {:reply, result, state}
  end

  def handle_call({:detail, id}, _, %{db: db} = state) do
    result =
      case query!(db, "SELECT * FROM requests WHERE id=?", [id]) do
        [request] ->
          %{
            "request" => decode(request, ["input_json", "response_json"]),
            "plugin_calls" =>
              query!(db, "SELECT * FROM plugin_calls WHERE request_id=? ORDER BY rowid", [id])
              |> Enum.map(&decode(&1, ["input_json", "output_json"]))
          }

        [] ->
          nil
      end

    {:reply, result, state}
  end

  def handle_call({:order, id}, _, %{db: db} = state) do
    result = query!(db, "SELECT * FROM orders WHERE id=?", [id]) |> List.first()
    {:reply, if(result, do: decode(result, ["items_json"])), state}
  end

  def handle_call({:execute, endpoint, input, path, key}, _, %{db: db} = state) do
    started = System.monotonic_time(:millisecond)
    request_id = id("req")
    hash = :crypto.hash(:sha256, :erlang.term_to_binary(input)) |> Base.encode16(case: :lower)

    previous =
      if key,
        do:
          query!(
            db,
            "SELECT * FROM requests WHERE idempotency_key=? AND method=? AND path=? AND replayed_from IS NULL ORDER BY rowid LIMIT 1",
            [key, endpoint.method, path]
          ),
        else: []

    write!(
      db,
      "INSERT INTO requests(id,created_at,method,path,input_json,status,idempotency_key,input_hash) VALUES (?,?,?,?,?,'running',?,?)",
      [request_id, now(), endpoint.method, path, json(input), key, hash]
    )

    {status, response, context} =
      case previous do
        [old] ->
          write!(db, "UPDATE requests SET replayed_from=? WHERE id=?", [old["id"], request_id])

          cond do
            old["input_hash"] != hash ->
              {409, error("idempotency_conflict", "Stejný klíč už patří jinému požadavku."), %{}}

            old["status"] == "running" ->
              case recover_request(state, old["id"]) do
                {:ok, status, response} ->
                  {status, Map.put(response, "replayed", true), %{}}

                _ ->
                  {409, error("outcome_unknown", "Původní operace vyžaduje ověření výsledku."),
                   %{}}
              end

            true ->
              {old["http_status"],
               Jason.decode!(old["response_json"]) |> Map.put("replayed", true), %{}}
          end

        [] ->
          execute_scenario(db, state.effects, state.steps, endpoint, input, request_id)
      end

    response = complete_request!(db, request_id, status, response, context, started)

    {:reply, {status, response}, state}
  end

  def handle_call(:deliver, _, %{db: db} = state) do
    jobs =
      query!(
        db,
        "SELECT * FROM flow_jobs WHERE state IN ('queued','retrying') AND next_at<=? ORDER BY rowid LIMIT 1",
        [System.system_time(:millisecond)]
      )

    case jobs do
      [job] ->
        attempt = job["attempts"] + 1
        input = Jason.decode!(job["input_json"])

        {result, call} =
          invoke(
            Map.fetch!(OrderLab.Language.Native.operations(), job["operation"]).module,
            List.first(String.split(job["operation"], ".")),
            List.last(String.split(job["operation"], ".")),
            input,
            job["request_id"],
            job["order_id"],
            attempt
          )

        exec!(db, "BEGIN IMMEDIATE")
        save_call!(db, call)

        case result do
          {:ok, response} ->
            write!(
              db,
              "UPDATE flow_jobs SET state='sent',attempts=?,result_json=?,error_json=NULL WHERE id=?",
              [attempt, json(response), job["id"]]
            )

          {:error, error} ->
            next_state = if attempt >= job["max_attempts"], do: "failed", else: "retrying"

            write!(
              db,
              "UPDATE flow_jobs SET state=?,attempts=?,error_json=?,next_at=? WHERE id=?",
              [
                next_state,
                attempt,
                json(error),
                System.system_time(:millisecond) + job["retry_delay_ms"],
                job["id"]
              ]
            )
        end

        if job["final_disposition"] == "delete" and attempt >= job["max_attempts"] and
             match?({:error, _}, result) do
          write!(db, "DELETE FROM flow_jobs WHERE id=?", [job["id"]])
        end

        exec!(db, "COMMIT")

      [] ->
        :ok
    end

    {:reply, :ok, state}
  end

  def handle_call({:retry_email, id}, _, %{db: db} = state) do
    result =
      case query!(db, "SELECT * FROM flow_jobs WHERE id=? AND state='failed'", [id]) do
        [job] ->
          input = Jason.decode!(job["input_json"]) |> Map.put("simulate_failure", false)

          write!(
            db,
            "UPDATE flow_jobs SET state='queued',attempts=0,input_json=?,error_json=NULL,next_at=? WHERE id=?",
            [json(input), System.system_time(:millisecond), id]
          )

          :ok

        [] ->
          :not_found
      end

    {:reply, result, state}
  end

  defp recover_request(state, id) do
    case query!(
           state.db,
           "SELECT r.status,r.http_status,r.response_json,c.fingerprint,c.value FROM requests r LEFT JOIN flow_checkpoints c ON c.request_id=r.id WHERE r.id=?",
           [id]
         ) do
      [] ->
        :not_found

      [%{"status" => status} = row] when status != "running" ->
        {:ok, row["http_status"], Jason.decode!(row["response_json"])}

      [row] ->
        if row["value"] && row["fingerprint"] == OrderLab.Checkpoint.fingerprint(state.steps) do
          checkpoint = OrderLab.Checkpoint.decode(row["value"])

          if OrderLab.Checkpoint.safe?(checkpoint) do
            started = System.monotonic_time(:millisecond)

            {status, response, context} =
              execute_scenario(state.db, state.effects, state.steps, nil, nil, id, checkpoint)

            {:ok, status, complete_request!(state.db, id, status, response, context, started)}
          else
            :unsafe
          end
        else
          :incompatible
        end
    end
  rescue
    error ->
      require Logger
      Logger.error("Checkpoint recovery failed for #{id}: #{Exception.message(error)}")
      :invalid_checkpoint
  end

  defp prepare_mqtt_job(state, job) do
    source = Map.fetch!(state.steps.mqtt, job["source"])
    params = Jason.decode!(job["params_json"])
    parameters = Enum.map(source.params, &params[&1.name])

    {_, checked, topic} =
      OrderLab.Language.Native.mqtt_target!(state.steps, source.name, parameters)

    unless checked == params and topic == job["topic"],
      do: raise("Outbox source contract changed")

    value =
      Jason.decode!(job["payload_json"]) |> OrderLab.Language.Types.validate!(source.payload)

    {:ok, source, params, parameters, topic, value}
  rescue
    error -> {:error, Exception.message(error)}
  end

  defp deliver_mqtt_job(state, job) do
    case prepare_mqtt_job(state, job) do
      {:error, message} ->
        write!(state.db, "UPDATE mqtt_outbox SET state='failed',error=? WHERE id=?", [
          message,
          job["id"]
        ])

      {:ok, source, params, parameters, topic, value} ->
        deliver_prepared_mqtt_job(state, job, source, params, parameters, topic, value)
    end
  end

  defp deliver_prepared_mqtt_job(state, job, source, params, parameters, topic, value) do
    db = state.db

    try do
      timestamp = job["accepted_at"] || now()

      if job["state"] == "queued" do
        exec!(db, "BEGIN IMMEDIATE")

        if job["retained"] == 1,
          do: write!(db, "UPDATE mqtt_messages SET retained=0 WHERE topic=?", [topic])

        write!(
          db,
          "INSERT INTO mqtt_messages(source,topic,payload_json,received_at,retained) VALUES (?,?,?,?,?)",
          [source.name, topic, json(value), timestamp, job["retained"]]
        )

        write!(db, "UPDATE mqtt_outbox SET state='accepted',accepted_at=? WHERE id=?", [
          timestamp,
          job["id"]
        ])

        exec!(db, "COMMIT")
      end

      OrderLab.Checkpoint.probe(
        %{"id" => job["id"], "time" => timestamp, "path" => "mqtt_outbox"},
        :mqtt_accepted
      )

      Enum.filter(state.steps.endpoints, &(&1.method == "MQTT" and &1.path == source.name))
      |> Enum.each(fn endpoint ->
        handle_call(
          {:execute, endpoint, Map.put(params, "message", value), topic,
           "mqtt-outbox:" <> job["id"]},
          nil,
          state
        )
      end)

      OrderLab.PubSub.broadcast({source.name, parameters}, %{
        "type" => "message",
        "source" => source.name,
        "params" => params,
        "payload" => value,
        "received_at" => timestamp
      })

      :ok = OrderLab.MQTT.forward(topic, job["payload_json"])

      write!(db, "UPDATE mqtt_outbox SET state='sent',sent_at=?,error=NULL WHERE id=?", [
        now(),
        job["id"]
      ])
    rescue
      error ->
        SQL.execute(db, "ROLLBACK")
        # Keep transport failures retryable; a contract failure needs inspection.
        row = query!(db, "SELECT state FROM mqtt_outbox WHERE id=?", [job["id"]]) |> List.first()
        status = if row["state"] == "queued", do: "failed", else: "accepted"

        write!(db, "UPDATE mqtt_outbox SET state=?,error=? WHERE id=?", [
          status,
          Exception.message(error),
          job["id"]
        ])
    catch
      :exit, reason ->
        write!(db, "UPDATE mqtt_outbox SET error=? WHERE id=?", [inspect(reason), job["id"]])
    end
  end

  defp websocket_result(fun) do
    {:ok, fun.()}
  rescue
    e in OrderLab.Language.Failure -> {:error, e.code, e.message}
    _ -> {:error, "invalid_input", "Invalid WebSocket input"}
  end

  defp complete_request!(db, request_id, status, response, context, started) do
    if Map.get(context, :pending, false) do
      if is_map(response),
        do: Map.put(response, "request_id", request_id),
        else: %{"result" => response, "request_id" => request_id}
    else
      exec!(db, "BEGIN IMMEDIATE")

      try do
        Enum.each(Map.get(context, :calls, []), &save_call!(db, &1))
        duration = System.monotonic_time(:millisecond) - started

        response =
          if is_map(response),
            do: Map.put(response, "request_id", request_id),
            else: %{"result" => response, "request_id" => request_id}

        order_id = get_in(response, ["order", "id"])
        code = get_in(response, ["error", "code"])

        write!(
          db,
          "UPDATE requests SET status=?,http_status=?,response_json=?,order_id=?,error_code=?,duration_ms=? WHERE id=?",
          [
            if(status < 400, do: "committed", else: "failed"),
            status,
            json(response),
            order_id,
            code,
            duration,
            request_id
          ]
        )

        write!(db, "DELETE FROM flow_checkpoints WHERE request_id=?", [request_id])
        exec!(db, "COMMIT")
        response
      rescue
        error ->
          SQL.execute(db, "ROLLBACK")
          reraise error, __STACKTRACE__
      end
    end
  end

  defp execute_scenario(db, effects, program, endpoint, input, request_id, checkpoint \\ nil) do
    key = {:flow_calls, request_id}
    Process.put(key, [])
    cursor = {:flow_cursor, request_id}

    Process.put(
      cursor,
      if(checkpoint,
        do: Map.get(checkpoint, :journal_cursor, %{value: 0, call: 0}),
        else: %{value: 0, call: 0}
      )
    )

    request =
      if checkpoint,
        do: checkpoint.env["request"],
        else: %{"id" => request_id, "time" => now(), "path" => endpoint.path}

    journal_value = fn name, args, generate ->
      position = Process.get(cursor).value
      Process.put(cursor, %{Process.get(cursor) | value: position + 1})
      OrderLab.EffectJournal.value!(effects, request_id, position, name, args, generate)
    end

    invoke = fn operation, module, input, site ->
      [plugin, method] = String.split(operation, ".", parts: 2)
      contract = Map.fetch!(OrderLab.Language.Native.operations(), operation)

      {result, call} =
        if Map.get(contract, :retry) == :idempotent do
          position = Process.get(cursor).call
          Process.put(cursor, %{Process.get(cursor) | call: position + 1})

          OrderLab.EffectJournal.invoke!(
            effects,
            request,
            position,
            site,
            operation,
            input,
            fn id ->
              invoke(
                fn value -> module.call(value, %{"idempotency_key" => id}) end,
                plugin,
                method,
                input,
                request_id,
                input["order_id"],
                1
              )
            end
          )
        else
          invoke(module, plugin, method, input, request_id, input["order_id"], 1)
        end

      Process.put(key, Process.get(key) ++ [call])
      result
    end

    native =
      OrderLab.Language.Native.host(db, program, request, invoke, fn prefix ->
        journal_value.("uuid", [prefix], fn ->
          OrderLab.Language.Evaluator.builtin("uuid", [prefix])
        end)
      end)

    save_checkpoint = fn value ->
      write!(
        db,
        "INSERT INTO flow_checkpoints(request_id,fingerprint,value,created_at) VALUES (?,?,?,?) ON CONFLICT(request_id) DO UPDATE SET fingerprint=excluded.fingerprint,value=excluded.value,created_at=excluded.created_at",
        [
          request_id,
          OrderLab.Checkpoint.fingerprint(program),
          OrderLab.Checkpoint.encode(Map.put(value, :journal_cursor, Process.get(cursor))),
          now()
        ]
      )
    end

    host = fn action, args ->
      case action do
        :nondeterministic ->
          journal_value.(args.name, args.args, fn ->
            OrderLab.Language.Evaluator.builtin(args.name, args.args)
          end)

        :source ->
          journal_value.("source", [args.name], fn -> native.(:source, args) end)

        :checkpoint ->
          save_checkpoint.(args.checkpoint)

        :commit ->
          save_checkpoint.(args.checkpoint)
          Enum.each(Process.get(key), &save_call!(db, &1))
          OrderLab.Checkpoint.probe(request, :before_commit)
          native.(:commit, args)
          Process.put(key, [])
          OrderLab.Checkpoint.probe(request, :after_commit)

        other ->
          native.(other, args)
      end
    end

    try do
      {status, result} =
        if checkpoint,
          do: OrderLab.Language.Runtime.resume(checkpoint, host),
          else: OrderLab.Language.Runtime.run(endpoint, input, program.types, host, request)

      {status, result, %{calls: Process.get(key)}}
    rescue
      e in OrderLab.ExternalUnknown ->
        SQL.execute(db, "ROLLBACK")
        {503, error("outcome_unknown", e.message), %{calls: Process.get(key), pending: true}}

      e in OrderLab.Language.Failure ->
        {e.status, error(e.code, e.message), %{calls: Process.get(key)}}

      e in OrderLab.Language.Error ->
        code = if e.stage == :validation, do: "invalid_input", else: "language_error"

        {if(e.stage == :validation, do: 422, else: 500), error(code, e.message),
         %{calls: Process.get(key)}}

      e ->
        SQL.execute(db, "ROLLBACK")
        require Logger
        Logger.error(Exception.format(:error, e, __STACKTRACE__))
        {500, error("internal_error", "Operace selhala."), %{calls: Process.get(key)}}
    after
      Process.delete(key)
      Process.delete(cursor)
    end
  end

  defp invoke(module, plugin, operation, input, request_id, order_id, attempt) do
    started_at = now()
    started = System.monotonic_time(:millisecond)
    result = if is_function(module, 1), do: module.(input), else: module.call(input)

    {status, output} =
      case result do
        {:ok, output} -> {"success", output}
        {:error, output} -> {"error", output}
      end

    call = %{
      id: id("call"),
      request_id: request_id,
      order_id: order_id,
      plugin: plugin,
      operation: operation,
      input: input,
      output: output,
      status: status,
      created_at: started_at,
      duration_ms: System.monotonic_time(:millisecond) - started,
      attempt: attempt
    }

    {result, call}
  end

  defp save_call!(db, call) do
    write!(db, "INSERT INTO plugin_calls VALUES (?,?,?,?,?,?,?,?,?,?,?)", [
      call.id,
      call.request_id,
      call.order_id,
      call.plugin,
      call.operation,
      call.created_at,
      call.duration_ms,
      call.status,
      json(call.input),
      json(call.output),
      call.attempt
    ])
  end

  defp query!(db, sql, args \\ []) do
    {:ok, statement} = SQL.prepare(db, sql)

    try do
      :ok = SQL.bind(statement, args)
      {:ok, columns} = SQL.columns(db, statement)
      {:ok, rows} = SQL.fetch_all(db, statement)
      Enum.map(rows, &Map.new(Enum.zip(columns, &1)))
    after
      SQL.release(db, statement)
    end
  end

  defp write!(db, sql, args) do
    query!(db, sql, args)
    :ok
  end

  defp exec!(db, sql) do
    case SQL.execute(db, sql) do
      :ok -> :ok
      {:error, reason} -> raise("SQLite: #{inspect(reason)}")
    end
  end

  defp decode(row, fields) do
    Enum.reduce(fields, row, fn field, row ->
      value = row[field]

      row
      |> Map.delete(field)
      |> Map.put(String.replace_suffix(field, "_json", ""), if(value, do: Jason.decode!(value)))
    end)
  end

  defp id(prefix), do: prefix <> "_" <> Base.encode16(:crypto.strong_rand_bytes(8), case: :lower)
  defp now, do: DateTime.utc_now() |> DateTime.to_iso8601()
  defp json(value), do: Jason.encode!(value)
  defp error(code, message), do: %{"error" => %{"code" => code, "message" => message}}
end
