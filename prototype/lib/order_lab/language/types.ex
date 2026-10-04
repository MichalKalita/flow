defmodule OrderLab.Language.Types do
  @moduledoc "Strict JSON type validation, including recursively resolved refinement predicates."
  alias OrderLab.Language.{Error, Evaluator}
  @primitives ~w(String Int Number Float Bool JSON)

  def resolve!(type, definitions, visited \\ [])

  def resolve!({:named, name} = type, definitions, visited) do
    cond do
      name in @primitives ->
        type

      name in visited ->
        Error.fail!("Cyclic type: #{Enum.join(Enum.reverse([name | visited]), " -> ")}")

      Map.has_key?(definitions, name) ->
        resolve!(definitions[name], definitions, [name | visited])

      true ->
        Error.fail!("Unknown type #{name}")
    end
  end

  def resolve!({:list, type}, defs, visited), do: {:list, resolve!(type, defs, visited)}
  def resolve!({:optional, type}, defs, visited), do: {:optional, resolve!(type, defs, visited)}

  def resolve!({:record, fields}, defs, visited),
    do: {:record, Map.new(fields, fn {key, type} -> {key, resolve!(type, defs, visited)} end)}

  def resolve!({:refined, type, condition}, defs, visited),
    do: {:refined, resolve!(type, defs, visited), condition}

  def validate!(value, type, definitions \\ %{}, path \\ "value") do
    resolved = resolve!(type, definitions)
    normalized = normalize(value, resolved)
    check!(normalized, resolved, path)
    normalized
  end

  defp normalize(value, {:refined, type, _}), do: normalize(value, type)
  defp normalize(nil, {:optional, _}), do: nil
  defp normalize(value, {:optional, type}), do: normalize(value, type)

  defp normalize(value, {:list, type}) when is_list(value),
    do: Enum.map(value, &normalize(&1, type))

  defp normalize(value, {:record, fields}) when is_map(value) do
    Enum.reduce(fields, value, fn {name, type}, record ->
      cond do
        Map.has_key?(record, name) -> Map.put(record, name, normalize(record[name], type))
        match?({:optional, _}, type) -> Map.put(record, name, nil)
        true -> record
      end
    end)
  end

  defp normalize(value, _), do: value

  defp check!(nil, {:optional, _}, _), do: :ok
  defp check!(value, {:optional, type}, path), do: check!(value, type, path)

  defp check!(value, {:refined, type, predicate}, path) do
    check!(value, type, path)

    accepted =
      Evaluator.eval(predicate, %{"value" => value}, fn name ->
        invalid!(path, "unknown variable #{name} in refinement")
      end)

    unless accepted === true, do: invalid!(path, "refinement predicate is not satisfied")
  end

  defp check!(value, {:named, "String"}, _) when is_binary(value), do: :ok
  defp check!(value, {:named, "Int"}, _) when is_integer(value), do: :ok

  defp check!(value, {:named, name}, _) when name in ["Number", "Float"] and is_number(value),
    do: :ok

  defp check!(value, {:named, "Bool"}, _) when is_boolean(value), do: :ok
  defp check!(value, {:named, "JSON"}, path), do: json!(value, path)

  defp check!(value, {:list, type}, path) when is_list(value) do
    Enum.with_index(value)
    |> Enum.each(fn {item, index} -> check!(item, type, "#{path}[#{index}]") end)
  end

  defp check!(value, {:record, fields}, path) when is_map(value) do
    unknown = Map.keys(value) -- Map.keys(fields)
    if unknown != [], do: invalid!(path, "unknown fields #{Enum.join(unknown, ", ")}")

    Enum.each(fields, fn {name, type} ->
      case Map.fetch(value, name) do
        {:ok, field} ->
          check!(field, type, "#{path}.#{name}")

        :error ->
          unless match?({:optional, _}, type),
            do: invalid!("#{path}.#{name}", "required field is missing")
      end
    end)
  end

  defp check!(_, type, path), do: invalid!(path, "expected #{describe(type)}")

  defp json!(value, _)
       when is_nil(value) or is_boolean(value) or is_number(value) or is_binary(value), do: :ok

  defp json!(value, path) when is_list(value), do: Enum.each(value, &json!(&1, path))

  defp json!(value, path) when is_map(value) do
    Enum.each(value, fn {key, item} ->
      unless is_binary(key), do: invalid!(path, "JSON record keys must be strings")
      json!(item, path <> "." <> key)
    end)
  end

  defp json!(_, path), do: invalid!(path, "expected JSON value")

  def describe({:named, name}), do: name
  def describe({:list, type}), do: "List<#{describe(type)}>"
  def describe({:optional, type}), do: describe(type) <> "?"
  def describe({:record, _}), do: "record"
  def describe({:refined, type, _}), do: describe(type) <> " satisfying its refinement"
  defp invalid!(path, reason), do: Error.fail!("#{path}: #{reason}", stage: :validation)
end
