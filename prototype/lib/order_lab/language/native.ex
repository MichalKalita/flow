defmodule OrderLab.Language.Native do
  alias Exqlite.Sqlite3, as: SQL
  alias OrderLab.Language.{Types, Failure, Evaluator}

  @physical %{
    "Products" => {"products", %{}},
    "Users" => {"users", %{}},
    "Orders" => {"orders", %{"items" => "items_json"}}
  }
  def operations do
    string = {:named, "String"}
    int = {:named, "Int"}
    item = {:record, %{"id" => string, "name" => string, "price_cents" => int, "quantity" => int}}

    dimension =
      {:refined, int,
       {:binary, "AND", {:binary, ">=", {:variable, "value"}, {:literal, 1}},
        {:binary, "<=", {:variable, "value"}, {:literal, 8192}}}}

    file_ref =
      {:record,
       Map.merge(
         Map.take(OrderLab.FileValue.fields("File"), ~w(name media_type byte_size sha256)),
         %{"id" => string, "url" => string}
       )}

    %{
      "Image.resize" => %{
        module: OrderLab.Plugins.ImageResize,
        effect: :pure,
        queueable: false,
        input:
          {:record, %{"image" => {:named, "Image"}, "width" => dimension, "height" => dimension}},
        output: {:named, "Image"}
      },
      "Image.decode" => %{
        module: OrderLab.Plugins.ImageDecode,
        effect: :pure,
        queueable: false,
        input: {:record, %{"file" => {:named, "File"}}},
        output: {:named, "Image"}
      },
      "Files.put" => %{
        module: OrderLab.Files,
        native: :put,
        effect: :write,
        queueable: false,
        input: {:record, %{"file" => {:named, "File"}}},
        output: file_ref
      },
      "Files.read" => %{
        module: OrderLab.Files,
        native: :read,
        effect: :read,
        queueable: false,
        input: {:record, %{"id" => string}},
        output: {:named, "File"}
      },
      "Files.delete" => %{
        module: OrderLab.Files,
        native: :delete,
        effect: :write,
        queueable: false,
        input: {:record, %{"id" => string}},
        output: {:record, %{"id" => string, "deleted" => {:named, "Bool"}}}
      },
      "Payment.create_url" => %{
        module: OrderLab.Plugins.Payment,
        input:
          {:record,
           %{
             "order_id" => string,
             "amount_cents" => int,
             "currency" => string,
             "country" => string,
             "method" => string
           }},
        output: {:record, %{"url" => string, "provider" => string, "method" => string}}
      },
      "Email.send_confirmation" => %{
        module: OrderLab.Plugins.Email,
        input:
          {:record,
           %{
             "order_id" => string,
             "to" => string,
             "subject" => string,
             "payment_url" => string,
             "total_cents" => int,
             "items" => {:list, item},
             "simulate_failure" => {:named, "Bool"}
           }},
        output: {:record, %{"message_id" => string, "delivery" => string, "to" => string}}
      }
    }
  end

  def host(db, program, request, invoke) do
    fn action, args ->
      case action do
        :source ->
          source(db, program, args.name, request["time"])

        :checkpoint ->
          :ok

        :begin ->
          exec!(db, "BEGIN IMMEDIATE")

        :commit ->
          exec!(db, "COMMIT")

        :rollback ->
          SQL.execute(db, "ROLLBACK")

        :savepoint ->
          exec!(db, "SAVEPOINT " <> args.name)

        :release ->
          exec!(db, "RELEASE " <> args.name)

        :rollback_to ->
          exec!(db, "ROLLBACK TO " <> args.name)

        :call ->
          op = Map.fetch!(operations(), args.operation)
          input = Types.validate!(args.input, op.input)

          implementation =
            if Map.has_key?(op, :native),
              do: fn input -> apply(op.module, op.native, [db, input, request]) end,
              else: op.module

          case invoke.(args.operation, implementation, input) do
            {:ok, result} ->
              Types.validate!(result, op.output)

            {:error, error} ->
              raise Failure, status: 422, code: error["code"], message: error["message"]
          end

        :publish ->
          {definition, params, topic} = mqtt_target!(program, args.source, args.parameters)
          value = Types.validate!(args.value, definition.payload)
          payload = Jason.encode!(value)

          if byte_size(topic) + byte_size(payload) + 4 > 64_000,
            do:
              raise(Failure,
                status: 422,
                code: "mqtt_message_too_large",
                message: "MQTT message exceeds 64 kB"
              )

          id = Evaluator.builtin("uuid", ["publish"])

          query!(
            db,
            "INSERT INTO mqtt_outbox(id,request_id,source,params_json,topic,payload_json,retained,state,created_at) VALUES (?,?,?,?,?,?,?,'queued',?)",
            [
              id,
              request["id"],
              args.source,
              Jason.encode!(params),
              topic,
              payload,
              if(args.retain, do: 1, else: 0),
              request["time"]
            ]
          )

          %{"id" => id, "state" => "queued"}

        :queue ->
          op = Map.fetch!(operations(), args.operation)
          input = Types.validate!(args.input, op.input)
          job = Evaluator.builtin("uuid", ["job"])

          query!(
            db,
            "INSERT INTO flow_jobs(id,order_id,request_id,operation,input_json,state,next_at,max_attempts,retry_delay_ms,final_disposition) VALUES (?,?,?,?,?,'queued',?,?,?,?)",
            [
              job,
              args.input["order_id"],
              request["id"],
              args.operation,
              Jason.encode!(input),
              System.system_time(:millisecond),
              args.attempts,
              args.delay,
              String.downcase(args.disposition)
            ]
          )

          %{"id" => job, "state" => "queued"}

        :insert ->
          insert(db, program, args.table, args.value)

        :update ->
          update(db, program, args.table, args.row, args.value)

        :delete ->
          delete(db, args.table, args.row)
      end
    end
  end

  def mqtt_target!(program, name, parameters) do
    definition = Map.fetch!(program.mqtt, name)
    unless length(parameters) == length(definition.params), do: raise("Wrong source arity")

    topic =
      Enum.zip(definition.params, parameters)
      |> Enum.reduce(definition.topic, fn {param, value}, topic ->
        Types.validate!(value, param.type)
        encoded = to_string(value)

        if String.contains?(encoded, ["/", "#", "+", "\0"]),
          do:
            raise(Failure,
              status: 422,
              code: "invalid_topic_parameter",
              message: "Invalid topic parameter"
            )

        String.replace(topic, "{#{param.name}}", encoded)
      end)

    values =
      Enum.zip(definition.params, parameters)
      |> Map.new(fn {param, value} -> {param.name, value} end)

    {definition, values, topic}
  end

  def source(db, program, {name, parameters}, cutoff) do
    {definition, _, topic} = mqtt_target!(program, name, parameters)

    threshold =
      DateTime.from_iso8601(cutoff)
      |> elem(1)
      |> DateTime.add(-definition.history_ms, :millisecond)
      |> DateTime.to_iso8601()

    query!(
      db,
      "SELECT payload_json,received_at FROM mqtt_messages WHERE source=? AND topic=? AND received_at<=? AND (received_at>=? OR id=(SELECT MAX(id) FROM mqtt_messages WHERE source=? AND topic=? AND received_at<=?)) ORDER BY id",
      [name, topic, cutoff, threshold, name, topic, cutoff]
    )
    |> Enum.map(fn row ->
      validate_source!(Jason.decode!(row["payload_json"]), definition.payload)
      |> Map.put("received_at", row["received_at"])
    end)
  end

  def source(db, program, name, _) do
    _ = Map.fetch!(program.tables, name)

    case Map.get(@physical, name) do
      {table, json_fields} ->
        query!(db, "SELECT * FROM #{table} ORDER BY rowid")
        |> Enum.map(fn row ->
          Enum.reduce(json_fields, row, fn {field, column}, row ->
            Map.put(Map.delete(row, column), field, Jason.decode!(row[column]))
          end)
          |> validate_source!(Map.fetch!(program.tables, name))
        end)

      nil ->
        query!(db, "SELECT value_json FROM flow_records WHERE table_name=? ORDER BY rowid", [name])
        |> Enum.map(
          &validate_source!(Jason.decode!(&1["value_json"]), Map.fetch!(program.tables, name))
        )
    end
  end

  defp validate_source!(value, type) do
    Types.validate!(value, type)
  rescue
    error in OrderLab.Language.Error -> reraise %{error | stage: :runtime}, __STACKTRACE__
  end

  defp insert(db, program, name, value) do
    Types.validate!(value, Map.fetch!(program.tables, name))

    case Map.get(@physical, name) do
      {table, mapping} ->
        fields = Enum.sort(Map.keys(value))
        columns = Enum.map(fields, &Map.get(mapping, &1, &1))
        # Field names were checked against the declared contract and the physical schema.
        allowed = query!(db, "PRAGMA table_info(#{table})") |> Enum.map(& &1["name"])
        unless Enum.all?(columns, &(&1 in allowed)), do: raise("Physical table schema mismatch")

        args =
          Enum.map(fields, fn field ->
            if Map.has_key?(mapping, field), do: Jason.encode!(value[field]), else: value[field]
          end)

        query!(
          db,
          "INSERT INTO #{table}(#{Enum.join(columns, ",")}) VALUES (#{Enum.map_join(fields, ",", fn _ -> "?" end)})",
          args
        )

      nil ->
        query!(db, "INSERT INTO flow_records(table_name,id,value_json) VALUES (?,?,?)", [
          name,
          value["id"],
          Jason.encode!(value)
        ])
    end

    value
  end

  defp update(db, program, name, row, changed) do
    value = Map.merge(row, changed)
    Types.validate!(value, Map.fetch!(program.tables, name))

    case Map.get(@physical, name) do
      {table, mapping} ->
        fields = Enum.sort(Map.keys(changed))

        if fields != [] do
          allowed = query!(db, "PRAGMA table_info(#{table})") |> Enum.map(& &1["name"])
          columns = Enum.map(fields, &Map.get(mapping, &1, &1))
          unless Enum.all?(columns, &(&1 in allowed)), do: raise("Physical table schema mismatch")

          args =
            Enum.map(fields, fn field ->
              if Map.has_key?(mapping, field), do: Jason.encode!(value[field]), else: value[field]
            end)

          query!(
            db,
            "UPDATE #{table} SET #{Enum.map_join(columns, ",", &(&1 <> "=?"))} WHERE id=?",
            args ++ [row["id"]]
          )
        end

      nil ->
        query!(db, "UPDATE flow_records SET value_json=? WHERE table_name=? AND id=?", [
          Jason.encode!(value),
          name,
          row["id"]
        ])
    end

    value
  end

  defp delete(db, name, row) do
    case Map.get(@physical, name) do
      {table, _} -> query!(db, "DELETE FROM #{table} WHERE id=?", [row["id"]])
      nil -> query!(db, "DELETE FROM flow_records WHERE table_name=? AND id=?", [name, row["id"]])
    end

    row
  end

  def query!(db, sql, args \\ []) do
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

  def exec!(db, sql) do
    case SQL.execute(db, sql) do
      :ok -> :ok
      {:error, reason} -> raise("SQLite: #{inspect(reason)}")
    end
  end
end
