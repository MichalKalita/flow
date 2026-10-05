defmodule Flow.Store do
  @moduledoc "Serialized SQLite access. Only trusted host code receives a connection."
  use GenServer
  alias Exqlite.Sqlite3, as: SQL
  alias Flow.{Ref, ID}

  defstruct [:connection, :schema, :trace]

  def start_link(options), do: GenServer.start_link(__MODULE__, options)

  def transaction(store, function), do: GenServer.call(store, {:transaction, function}, :infinity)
  def read(store, function), do: GenServer.call(store, {:read, function}, :infinity)

  @impl true
  def init(options) do
    schema = Keyword.fetch!(options, :schema)

    with {:ok, connection} <- SQL.open(Keyword.get(options, :path, ":memory:")) do
      db = %__MODULE__{connection: connection, schema: schema, trace: options[:trace]}

      try do
        execute(db, "PRAGMA foreign_keys = ON")
        execute(db, "PRAGMA journal_mode = WAL")
        execute(db, "PRAGMA busy_timeout = 5000")
        migrate(db)
        {:ok, db}
      rescue
        error ->
          SQL.close(connection)
          {:stop, error}
      end
    end
  end

  @impl true
  def handle_call({:read, function}, _from, db),
    do: {:reply, protect(fn -> function.(db) end), db}

  def handle_call({:transaction, function}, _from, db) do
    result =
      protect(fn ->
        execute(db, "BEGIN IMMEDIATE")

        try do
          result = function.(db)
          execute(db, "COMMIT")
          result
        rescue
          error ->
            execute(db, "ROLLBACK")
            reraise error, __STACKTRACE__
        catch
          kind, reason ->
            execute(db, "ROLLBACK")
            :erlang.raise(kind, reason, __STACKTRACE__)
        end
      end)

    {:reply, result, db}
  end

  @impl true
  def terminate(_, db), do: SQL.close(db.connection)

  defp protect(function) do
    try do
      {:ok, function.()}
    rescue
      error -> {:error, error}
    catch
      kind, reason -> {:error, {kind, reason}}
    end
  end

  def fields(db, entity) do
    model =
      db.schema.entities[entity] || db.schema.streams[entity] ||
        raise ArgumentError, "Unknown entity #{entity}"

    Map.reject(model.fields, fn {_, field} ->
      field.options["inverse"] || field.options["stream"]
    end)
  end

  def insert(db, entity, values) do
    definitions = fields(db, entity)
    keys = Map.keys(values) |> Enum.sort()

    if keys == [] or Enum.any?(keys, &(not Map.has_key?(definitions, &1))),
      do: raise(ArgumentError, "Invalid insert columns")

    sql =
      "INSERT INTO #{quote_name(entity)} (#{columns(keys)}) VALUES (#{Enum.map_join(keys, ",", fn _ -> "?" end)})"

    query(db, sql, Enum.map(keys, &encode(values[&1])))
    :ok
  end

  def update(db, %Ref{entity: entity, id: id}, values) do
    definitions = fields(db, entity)
    keys = Map.keys(values) |> Enum.sort()

    if keys == [] or "id" in keys or Enum.any?(keys, &(not Map.has_key?(definitions, &1))),
      do: raise(ArgumentError, "Invalid update columns")

    assignments = Enum.map_join(keys, ",", &(quote_name(&1) <> " = ?"))

    query(
      db,
      "UPDATE #{quote_name(entity)} SET #{assignments} WHERE id = ?",
      Enum.map(keys, &encode(values[&1])) ++ [encode(id)]
    )

    :ok
  end

  def delete(db, %Ref{entity: entity, id: id}) do
    fields(db, entity)
    query(db, "DELETE FROM #{quote_name(entity)} WHERE id = ?", [encode(id)])
    :ok
  end

  def fetch(db, %Ref{entity: entity, id: id}, keys) do
    definitions = fields(db, entity)

    if keys == [] or Enum.any?(keys, &(not Map.has_key?(definitions, &1))),
      do: raise(ArgumentError, "Invalid projection")

    case query(db, "SELECT #{columns(keys)} FROM #{quote_name(entity)} WHERE id = ?", [encode(id)]) do
      [] -> nil
      [row] -> Map.new(Enum.zip(keys, row))
    end
  end

  def ids(db, entity) do
    fields(db, entity)

    query(db, "SELECT id FROM #{quote_name(entity)} ORDER BY rowid", [])
    |> Enum.map(fn [id] -> %Ref{entity: entity, id: %ID{entity: entity, value: id}} end)
  end

  def related(db, entity, foreign, %Ref{} = reference) do
    definitions = fields(db, entity)
    unless Map.has_key?(definitions, foreign), do: raise(ArgumentError, "Invalid relation")

    query(
      db,
      "SELECT id FROM #{quote_name(entity)} WHERE #{quote_name(foreign)} = ? ORDER BY rowid",
      [encode(reference)]
    )
    |> Enum.map(fn [id] -> %Ref{entity: entity, id: %ID{entity: entity, value: id}} end)
  end

  def lookup(db, entity, field, value) do
    definitions = fields(db, entity)
    unless Map.has_key?(definitions, field), do: raise(ArgumentError, "Invalid identity lookup")

    case query(db, "SELECT id FROM #{quote_name(entity)} WHERE #{quote_name(field)} = ?", [
           encode(value)
         ]) do
      [[id]] -> %Ref{entity: entity, id: %ID{entity: entity, value: id}}
      _ -> nil
    end
  end

  def query(db, sql, params) do
    if db.trace, do: send(db.trace, {:flow_sql, sql, params})
    {:ok, statement} = SQL.prepare(db.connection, sql)

    try do
      :ok = SQL.bind(statement, params)

      case SQL.fetch_all(db.connection, statement) do
        {:ok, rows} -> rows
        {:error, reason} -> raise "SQLite: #{inspect(reason)}"
      end
    after
      SQL.release(db.connection, statement)
    end
  end

  defp execute(db, sql) do
    case SQL.execute(db.connection, sql) do
      :ok -> :ok
      {:error, reason} -> raise "SQLite: #{inspect(reason)}"
    end
  end

  defp migrate(db) do
    for {entity, _} <- Map.merge(db.schema.entities, db.schema.streams) do
      definitions = fields(db, entity)

      columns =
        Enum.map(definitions, fn {name, field} ->
          type = underlying(db.schema, field.type)
          optional = match?({:optional, _}, type)

          type =
            case type do
              {:optional, inner} -> underlying(db.schema, inner)
              _ -> type
            end

          base = quote_name(name) <> " " <> sql_type(type)
          base = if name == "id", do: base <> " PRIMARY KEY", else: base
          base = if optional, do: base, else: base <> " NOT NULL"
          base = if field.options["unique"], do: base <> " UNIQUE", else: base

          case type do
            {:named, target} when is_map_key(db.schema.entities, target) ->
              base <> " REFERENCES " <> quote_name(target) <> "(id) DEFERRABLE INITIALLY DEFERRED"

            _ ->
              base
          end
        end)

      execute(db, "CREATE TABLE IF NOT EXISTS #{quote_name(entity)} (#{Enum.join(columns, ",")})")
    end
  end

  defp underlying(schema, {:named, name} = type) do
    case schema.types[name] do
      nil -> type
      inner -> underlying(schema, inner)
    end
  end

  defp underlying(_, type), do: type
  defp sql_type({:number, "integer", _, _, _}), do: "TEXT"
  defp sql_type({:named, "Bool"}), do: "INTEGER"
  defp sql_type({:image, _}), do: "BLOB"
  defp sql_type(_), do: "TEXT"

  defp columns(keys), do: Enum.map_join(keys, ",", &quote_name/1)

  defp quote_name(name) do
    unless Regex.match?(~r/\A[A-Za-z_][A-Za-z_0-9]*\z/, name),
      do: raise(ArgumentError, "Invalid SQL identifier")

    "\"" <> name <> "\""
  end

  def encode(%ID{value: value}), do: value
  def encode(%Ref{id: id}), do: encode(id)
  def encode(%Flow.Image{bytes: bytes}), do: {:blob, bytes}
  def encode(%Decimal{} = value), do: Decimal.to_string(value, :normal)
  def encode(%DateTime{} = value), do: DateTime.to_iso8601(value)
  def encode(true), do: 1
  def encode(false), do: 0

  def encode(value)
      when is_integer(value) and
             (value > 9_223_372_036_854_775_807 or value < -9_223_372_036_854_775_808),
      do: Integer.to_string(value)

  def encode(value) when is_map(value) or is_list(value), do: Jason.encode!(json(value))
  def encode(value), do: value

  defp json(%ID{value: value}), do: value
  defp json(%Ref{id: id}), do: json(id)
  defp json(%Flow.Version{reference: reference}), do: json(reference)
  defp json(%Flow.Embedded{value: value}), do: json(value)
  defp json(%Flow.Image{bytes: bytes}), do: %{"$image" => Base.encode64(bytes)}
  defp json(%DateTime{} = value), do: DateTime.to_iso8601(value)
  defp json(%Decimal{} = value), do: value
  defp json(value) when is_list(value), do: Enum.map(value, &json/1)
  defp json(value) when is_map(value), do: Map.new(value, fn {k, v} -> {k, json(v)} end)
  defp json(value), do: value
end
