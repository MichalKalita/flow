defmodule OrderLab.Files do
  @moduledoc "SQLite-backed native files participating in the scenario transaction."
  alias OrderLab.Language.{Native, Types, Evaluator}

  def put(db, input, request) do
    file = Types.validate!(input["file"], {:named, "File"})
    id = Evaluator.builtin("uuid", ["file"])

    Native.query!(
      db,
      "INSERT INTO flow_files(id,value_json,request_id,created_at) VALUES (?,?,?,?)",
      [id, Jason.encode!(file), request["id"], request["time"]]
    )

    {:ok,
     Map.take(file, ~w(name media_type byte_size sha256))
     |> Map.merge(%{"id" => id, "url" => "/files/" <> id})}
  end

  def read(db, input, _) do
    case Native.query!(db, "SELECT value_json FROM flow_files WHERE id=?", [input["id"]]) do
      [row] -> {:ok, Types.validate!(Jason.decode!(row["value_json"]), {:named, "File"})}
      [] -> {:error, %{"code" => "file_not_found", "message" => "File does not exist"}}
    end
  end

  def delete(db, input, request) do
    case read(db, input, request) do
      {:ok, _} ->
        Native.query!(db, "DELETE FROM flow_files WHERE id=?", [input["id"]])
        {:ok, %{"id" => input["id"], "deleted" => true}}

      error ->
        error
    end
  end
end
