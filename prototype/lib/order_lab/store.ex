defmodule OrderLab.Store do
  use GenServer
  alias Exqlite.Sqlite3, as: SQL

  def start_link(_), do: GenServer.start_link(__MODULE__, nil, name: __MODULE__)
  def snapshot, do: GenServer.call(__MODULE__, :snapshot)
  def detail(id), do: GenServer.call(__MODULE__, {:detail, id})
  def order(id), do: GenServer.call(__MODULE__, {:order, id})
  def create(input, key), do: GenServer.call(__MODULE__, {:create, input, key}, 30_000)
  def deliver, do: GenServer.call(__MODULE__, :deliver, 30_000)
  def retry_email(id), do: GenServer.call(__MODULE__, {:retry_email, id})

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

    Enum.each(
      [
        ["p1", "Studio sluchátka", 249_000, 12],
        ["p2", "Mechanická klávesnice", 329_000, 5],
        ["p3", "USB-C rozbočovač", 129_000, 2],
        ["p4", "Webkamera", 189_000, 0]
      ],
      &write!(db, "INSERT OR IGNORE INTO products VALUES (?, ?, ?, ?)", &1)
    )

    Enum.each(
      [
        ["u1", "Petra Nováková", "petra@example.test", "CZ"],
        ["u2", "David Miller", "david@example.test", "US"],
        ["u3", "Nora Silva", "nora@example.test", "BR"]
      ],
      &write!(db, "INSERT OR IGNORE INTO users VALUES (?, ?, ?, ?)", &1)
    )

    {:ok, %{db: db, steps: OrderLab.Workflow.compile!()}}
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
        query!(db, "SELECT * FROM email_jobs ORDER BY rowid DESC")
        |> Enum.map(&decode(&1, ["input_json", "error_json", "result_json"])),
      "workflow" => OrderLab.Workflow.source()
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

  def handle_call({:create, input, key}, _, %{db: db} = state) do
    started = System.monotonic_time(:millisecond)
    request_id = id("req")
    hash = :crypto.hash(:sha256, :erlang.term_to_binary(input)) |> Base.encode16(case: :lower)

    previous =
      if key,
        do:
          query!(
            db,
            "SELECT * FROM requests WHERE idempotency_key=? AND replayed_from IS NULL ORDER BY rowid LIMIT 1",
            [key]
          ),
        else: []

    write!(
      db,
      "INSERT INTO requests(id,created_at,method,path,input_json,status,idempotency_key,input_hash) VALUES (?,?,'POST','/api/orders',?,'running',?,?)",
      [request_id, now(), json(input), key, hash]
    )

    {status, response, context} =
      case previous do
        [old] ->
          write!(db, "UPDATE requests SET replayed_from=? WHERE id=?", [old["id"], request_id])

          cond do
            old["input_hash"] != hash ->
              {409, error("idempotency_conflict", "Stejný klíč už patří jinému požadavku."), %{}}

            old["status"] == "running" ->
              {409, error("outcome_unknown", "Původní operace vyžaduje ověření výsledku."), %{}}

            true ->
              {old["http_status"],
               Jason.decode!(old["response_json"]) |> Map.put("replayed", true), %{}}
          end

        [] ->
          execute_order(state, input, request_id)
      end

    Enum.each(Map.get(context, :calls, []), &save_call!(db, &1))
    duration = System.monotonic_time(:millisecond) - started
    response = Map.put(response, "request_id", request_id)
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

    {:reply, {status, response}, state}
  end

  def handle_call(:deliver, _, %{db: db} = state) do
    jobs =
      query!(
        db,
        "SELECT * FROM email_jobs WHERE state IN ('queued','retrying') AND next_at<=? ORDER BY rowid LIMIT 1",
        [System.system_time(:millisecond)]
      )

    case jobs do
      [job] ->
        attempt = job["attempts"] + 1
        input = Jason.decode!(job["input_json"])

        {result, call} =
          invoke(
            OrderLab.Plugins.Email,
            "Email",
            "send_confirmation",
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
              "UPDATE email_jobs SET state='sent',attempts=?,result_json=?,error_json=NULL WHERE id=?",
              [attempt, json(response), job["id"]]
            )

          {:error, error} ->
            next_state = if attempt >= job["max_attempts"], do: "failed", else: "retrying"

            write!(
              db,
              "UPDATE email_jobs SET state=?,attempts=?,error_json=?,next_at=? WHERE id=?",
              [
                next_state,
                attempt,
                json(error),
                System.system_time(:millisecond) + job["retry_delay_ms"],
                job["id"]
              ]
            )
        end

        exec!(db, "COMMIT")

      [] ->
        :ok
    end

    {:reply, :ok, state}
  end

  def handle_call({:retry_email, id}, _, %{db: db} = state) do
    result =
      case query!(db, "SELECT * FROM email_jobs WHERE id=? AND state='failed'", [id]) do
        [job] ->
          input = Jason.decode!(job["input_json"]) |> Map.put("simulate_failure", false)

          write!(
            db,
            "UPDATE email_jobs SET state='queued',attempts=0,input_json=?,error_json=NULL,next_at=? WHERE id=?",
            [json(input), System.system_time(:millisecond), id]
          )

          :ok

        [] ->
          :not_found
      end

    {:reply, result, state}
  end

  defp execute_order(%{db: db, steps: steps}, input, request_id) do
    with :ok <- validate(input),
         [user] <- query!(db, "SELECT * FROM users WHERE id=?", [input["user_id"]]) do
      context = %{db: db, input: input, user: user, request_id: request_id, calls: []}
      exec!(db, "BEGIN IMMEDIATE")

      try do
        case OrderLab.Workflow.run(steps, context, &step/2) do
          {:ok, result} ->
            {201,
             %{
               "order" => result.order,
               "payment" => result.payment,
               "email" => %{"state" => "queued"}
             }, result}

          {:error, {status, code, message}, result} ->
            exec!(db, "ROLLBACK")
            {status, error(code, message), result}
        end
      rescue
        exception ->
          SQL.execute(db, "ROLLBACK")
          require Logger
          Logger.error(Exception.format(:error, exception, __STACKTRACE__))
          {500, error("internal_error", "Operace selhala; změny byly vráceny."), %{}}
      end
    else
      {:error, message} -> {422, error("invalid_input", message), %{}}
      [] -> {404, error("user_not_found", "Uživatel neexistuje."), %{}}
    end
  end

  defp validate(input) when is_map(input) do
    items = input["items"]

    cond do
      Map.keys(input) -- ["user_id", "items", "payment_method", "email_failure"] != [] ->
        {:error, "Požadavek obsahuje neznámá pole."}

      not is_binary(input["user_id"]) ->
        {:error, "user_id musí být identifikátor uživatele."}

      input["payment_method"] not in ["card", "bank"] ->
        {:error, "payment_method musí být card nebo bank."}

      not is_boolean(Map.get(input, "email_failure", false)) ->
        {:error, "email_failure musí být boolean."}

      not is_list(items) or length(items) not in 1..50 ->
        {:error, "Objednávka musí obsahovat 1 až 50 položek."}

      not Enum.all?(items, fn item ->
        is_map(item) and Map.keys(item) -- ["product_id", "quantity"] == [] and
          is_binary(item["product_id"]) and is_integer(item["quantity"]) and
            item["quantity"] in 1..100
      end) ->
        {:error, "Položka potřebuje product_id a celočíselné quantity v rozsahu 1 až 100."}

      true ->
        :ok
    end
  end

  defp validate(_), do: {:error, "Vstup musí být JSON objekt."}

  defp step(:snapshot, context) do
    quantities =
      Enum.reduce(context.input["items"], %{}, fn item, acc ->
        Map.update(acc, item["product_id"], item["quantity"], &(&1 + item["quantity"]))
      end)

    items =
      Enum.map(Enum.sort(quantities), fn {product_id, quantity} ->
        case query!(context.db, "SELECT * FROM products WHERE id=?", [product_id]) do
          [product] ->
            Map.take(product, ["id", "name", "price_cents"]) |> Map.put("quantity", quantity)

          [] ->
            nil
        end
      end)

    if Enum.any?(items, &is_nil/1) do
      {:error, {404, "product_not_found", "Některý produkt neexistuje."}, context}
    else
      order = %{
        "id" => id("ord"),
        "user_id" => context.user["id"],
        "items" => items,
        "total_cents" => Enum.reduce(items, 0, &(&1["price_cents"] * &1["quantity"] + &2)),
        "currency" => "CZK",
        "payment_method" => context.input["payment_method"],
        "status" => "awaiting_payment",
        "created_at" => now()
      }

      write!(
        context.db,
        "INSERT INTO orders(id,user_id,request_id,created_at,items_json,total_cents,payment_method) VALUES (?,?,?,?,?,?,?)",
        [
          order["id"],
          order["user_id"],
          context.request_id,
          order["created_at"],
          json(items),
          order["total_cents"],
          order["payment_method"]
        ]
      )

      {:ok, Map.put(context, :order, order)}
    end
  end

  defp step(:decrease, context) do
    Enum.reduce_while(context.order["items"], {:ok, context}, fn item, {:ok, context} ->
      write!(context.db, "UPDATE products SET stock=stock-? WHERE id=? AND stock>=?", [
        item["quantity"],
        item["id"],
        item["quantity"]
      ])

      [%{"changed" => changed}] = query!(context.db, "SELECT changes() changed")

      if changed == 1,
        do: {:cont, {:ok, context}},
        else:
          {:halt,
           {:error, {409, "insufficient_stock", "Nedostatek kusů: #{item["name"]}."}, context}}
    end)
  end

  defp step(:payment, context) do
    input = %{
      "order_id" => context.order["id"],
      "amount_cents" => context.order["total_cents"],
      "currency" => "CZK",
      "country" => context.user["country"],
      "method" => context.input["payment_method"]
    }

    {result, call} =
      invoke(
        OrderLab.Plugins.Payment,
        "Payment",
        "create_url",
        input,
        context.request_id,
        context.order["id"],
        1
      )

    context = %{context | calls: context.calls ++ [call]}

    case result do
      {:ok, payment} ->
        write!(context.db, "UPDATE orders SET payment_url=? WHERE id=?", [
          payment["url"],
          context.order["id"]
        ])

        {:ok, Map.put(context, :payment, payment)}

      {:error, error} ->
        {:error, {422, error["code"], error["message"]}, context}
    end
  end

  defp step(:email, context) do
    input = %{
      "order_id" => context.order["id"],
      "to" => context.user["email"],
      "subject" => "Objednávka #{context.order["id"]}",
      "payment_url" => context.payment["url"],
      "total_cents" => context.order["total_cents"],
      "items" => context.order["items"],
      "simulate_failure" => Map.get(context.input, "email_failure", false)
    }

    write!(
      context.db,
      "INSERT INTO email_jobs(id,order_id,request_id,input_json,state,next_at) VALUES (?,?,?,?,'queued',?)",
      [
        id("job"),
        context.order["id"],
        context.request_id,
        json(input),
        System.system_time(:millisecond)
      ]
    )

    {:ok, context}
  end

  defp step(:commit, context) do
    exec!(context.db, "COMMIT")
    {:ok, context}
  end

  defp invoke(module, plugin, operation, input, request_id, order_id, attempt) do
    started_at = now()
    started = System.monotonic_time(:millisecond)
    result = module.call(input)

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
