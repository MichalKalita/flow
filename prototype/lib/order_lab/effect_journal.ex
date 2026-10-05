defmodule OrderLab.ExternalUnknown do
  defexception [:message]
end

defmodule OrderLab.EffectJournal do
  alias Exqlite.Sqlite3, as: SQL
  alias OrderLab.Language.Failure

  def open!(path) do
    {:ok, db} = SQL.open(path)
    :ok = SQL.execute(db, "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")

    :ok =
      SQL.execute(db, """
      CREATE TABLE IF NOT EXISTS journal_meta(key TEXT PRIMARY KEY,value TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS values_log(request_id TEXT NOT NULL,position INTEGER NOT NULL,signature TEXT NOT NULL,value_json TEXT NOT NULL,PRIMARY KEY(request_id,position));
      CREATE TABLE IF NOT EXISTS effects(request_id TEXT NOT NULL,position INTEGER NOT NULL,id TEXT NOT NULL UNIQUE,signature TEXT NOT NULL,operation TEXT NOT NULL,input_json TEXT NOT NULL,state TEXT NOT NULL,attempts INTEGER NOT NULL DEFAULT 0,result TEXT,updated_at TEXT NOT NULL,PRIMARY KEY(request_id,position));
      """)

    query!(db, "INSERT OR IGNORE INTO journal_meta(key,value) VALUES ('identity',?)", [
      Base.encode16(:crypto.strong_rand_bytes(24), case: :lower)
    ])

    db
  end

  def identity(db),
    do:
      query!(db, "SELECT value FROM journal_meta WHERE key='identity'")
      |> hd()
      |> Map.fetch!("value")

  def value!(db, request, position, name, args, generate) do
    signature = hash({name, args})

    case query!(db, "SELECT * FROM values_log WHERE request_id=? AND position=?", [
           request,
           position
         ]) do
      [row] ->
        check!(row["signature"], signature)
        value = Jason.decode!(row["value_json"])

        if name == "source" and generate.() != value,
          do:
            raise(Failure,
              status: 409,
              code: "journal_source_conflict",
              message: "Source data changed since the interrupted execution"
            )

        value

      [] ->
        value = generate.()

        query!(db, "INSERT INTO values_log VALUES (?,?,?,?)", [
          request,
          position,
          signature,
          Jason.encode!(value)
        ])

        value
    end
  end

  def invoke!(db, request, position, site, operation, input, perform) do
    signature = hash({site, operation, input})
    scope = Map.get(request, "journal_scope", request["id"])
    id = "effect_" <> hash({scope, position})

    row =
      case query!(db, "SELECT * FROM effects WHERE request_id=? AND position=?", [
             scope,
             position
           ]) do
        [row] ->
          check!(row["signature"], signature)
          row

        [] ->
          query!(
            db,
            "INSERT INTO effects(request_id,position,id,signature,operation,input_json,state,updated_at) VALUES (?,?,?,?,?,?,'pending',?)",
            [scope, position, id, signature, operation, Jason.encode!(input), now()]
          )

          %{"state" => "pending", "id" => id}
      end

    if row["state"] == "completed" do
      OrderLab.Checkpoint.decode(row["result"])
    else
      query!(db, "UPDATE effects SET attempts=attempts+1,updated_at=? WHERE id=?", [now(), id])
      result = perform.(id)
      OrderLab.Checkpoint.probe(request, :after_external_effect)

      try do
        query!(db, "UPDATE effects SET state='completed',result=?,updated_at=? WHERE id=?", [
          OrderLab.Checkpoint.encode(result),
          now(),
          id
        ])
      rescue
        _ -> raise OrderLab.ExternalUnknown, message: "External result could not be persisted"
      end

      OrderLab.Checkpoint.probe(request, :after_external_record)
      result
    end
  end

  def list(db) do
    query!(
      db,
      "SELECT id,request_id,operation,input_json,state,attempts,updated_at FROM effects ORDER BY rowid DESC LIMIT 100"
    )
    |> Enum.map(fn row ->
      row = row |> Map.put("input", Jason.decode!(row["input_json"])) |> Map.delete("input_json")

      case String.split(row["request_id"], ":", parts: 3) do
        ["queue", request_id, job_id] ->
          row |> Map.put("request_id", request_id) |> Map.put("job_id", job_id)

        _ ->
          row
      end
    end)
  end

  defp check!(signature, expected) do
    unless signature == expected,
      do:
        raise(Failure,
          status: 409,
          code: "journal_conflict",
          message: "Recovered execution diverged from its durable operation log"
        )
  end

  defp hash(value),
    do:
      :crypto.hash(:sha256, :erlang.term_to_binary(value, [:deterministic]))
      |> Base.encode16(case: :lower)

  defp now, do: DateTime.utc_now() |> DateTime.to_iso8601()

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
end
