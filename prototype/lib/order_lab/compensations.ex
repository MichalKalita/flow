defmodule OrderLab.Compensations do
  alias OrderLab.Language.Native
  alias OrderLab.Language.Native, as: DB

  def setup!(db) do
    DB.exec!(db, """
    CREATE TABLE IF NOT EXISTS compensations(
      effect_id TEXT PRIMARY KEY,request_id TEXT NOT NULL,operation TEXT NOT NULL,
      input_json TEXT NOT NULL,fingerprint TEXT NOT NULL,state TEXT NOT NULL,
      attempts INTEGER NOT NULL DEFAULT 0,next_at INTEGER NOT NULL DEFAULT 0,
      error_json TEXT,result_json TEXT);
    """)
  end

  def register!(db, effect_id, request_id, operation, input) do
    DB.query!(
      db,
      "INSERT OR IGNORE INTO compensations(effect_id,request_id,operation,input_json,fingerprint,state) VALUES (?,?,?,?,?,'waiting')",
      [effect_id, request_id, operation, Jason.encode!(input), fingerprint(operation)]
    )
  end

  def candidates(db) do
    DB.query!(
      db,
      "SELECT * FROM compensations WHERE state IN ('waiting','retrying') AND next_at<=? ORDER BY rowid",
      [System.system_time(:millisecond)]
    )
  end

  def compatible?(row), do: row["fingerprint"] == fingerprint(row["operation"])

  def finish!(db, row, state, result \\ nil, error \\ nil) do
    DB.query!(
      db,
      "UPDATE compensations SET state=?,result_json=?,error_json=? WHERE effect_id=?",
      [state, encode(result), encode(error), row["effect_id"]]
    )
  end

  def attempt!(db, row) do
    DB.query!(db, "UPDATE compensations SET attempts=attempts+1 WHERE effect_id=?", [
      row["effect_id"]
    ])

    row["attempts"] + 1
  end

  def fail!(db, row, error, attempt) do
    DB.query!(db, "UPDATE compensations SET state=?,error_json=?,next_at=? WHERE effect_id=?", [
      if(attempt >= 3, do: "failed", else: "retrying"),
      Jason.encode!(error),
      System.system_time(:millisecond) + 1000,
      row["effect_id"]
    ])
  end

  def retry!(db, id) do
    case DB.query!(
           db,
           "SELECT effect_id FROM compensations WHERE effect_id=? AND state='failed'",
           [id]
         ) do
      [] ->
        :not_found

      [_] ->
        DB.query!(
          db,
          "UPDATE compensations SET state='retrying',attempts=0,next_at=0,error_json=NULL WHERE effect_id=?",
          [id]
        )

        :ok
    end
  end

  def list(db) do
    DB.query!(db, "SELECT * FROM compensations ORDER BY rowid DESC LIMIT 100")
    |> Enum.map(fn row ->
      Enum.reduce(~w(input_json error_json result_json), row, fn field, acc ->
        acc
        |> Map.delete(field)
        |> Map.put(
          String.replace_suffix(field, "_json", ""),
          if(row[field], do: Jason.decode!(row[field]))
        )
      end)
      |> Map.delete("fingerprint")
    end)
  end

  defp fingerprint(operation) do
    :crypto.hash(
      :sha256,
      :erlang.term_to_binary({Map.get(Native.operations(), operation), Native.configuration()}, [
        :deterministic
      ])
    )
    |> Base.encode16(case: :lower)
  end

  defp encode(nil), do: nil
  defp encode(value), do: Jason.encode!(value)
end
