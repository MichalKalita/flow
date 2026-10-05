defmodule Flow.Value do
  @moduledoc "Validates external data without coercing brands, rounding numbers or dropping fields."
  alias Flow.{ID, Ref, Schema, ValidationError, Embedded}

  def validate(schema, type, value) do
    try do
      {:ok, cast(schema, type, value, "$", [])}
    rescue
      error in ValidationError -> {:error, error}
    end
  end

  def validate!(schema, type, value) do
    case validate(schema, type, value) do
      {:ok, value} -> value
      {:error, error} -> raise error
    end
  end

  defp cast(schema, {:named, name}, value, path, seen) do
    value =
      case value do
        %Embedded{type: ^name, value: record} -> record
        value -> value
      end

    if name in seen, do: fail(path, "Cyclic value type #{name}")

    case name do
      "String" ->
        if is_binary(value) and String.valid?(value),
          do: value,
          else: fail(path, "Expected UTF-8 string")

      "Bool" ->
        if is_boolean(value), do: value, else: fail(path, "Expected boolean")

      "DateTime" ->
        datetime(value, path)

      _ ->
        case schema.types[name] do
          nil ->
            unless schema.entities[name] || schema.streams[name],
              do: fail(path, "Unknown type #{name}")

            case value do
              %Ref{entity: ^name, id: id} ->
                %Ref{entity: name, id: cast(schema, {:id, name}, id, path, seen)}

              %Ref{} ->
                fail(path, "Entity reference brand mismatch")

              value ->
                %Ref{entity: name, id: cast(schema, {:id, name}, value, path, seen)}
            end

          type ->
            cast(schema, type, value, path, [name | seen])
        end
    end
  end

  defp cast(_, {:id, entity}, %ID{entity: entity, value: value} = id, path, _) do
    check_id(value, path)
    id
  end

  defp cast(_, {:id, _}, %ID{}, path, _), do: fail(path, "ID brand mismatch")

  defp cast(_, {:id, entity}, value, path, _) do
    check_id(value, path)
    %ID{entity: entity, value: value}
  end

  defp cast(_, {:optional, _}, nil, _, _), do: nil

  defp cast(schema, {:optional, inner}, value, path, seen),
    do: cast(schema, inner, value, path, seen)

  defp cast(schema, {:list, inner, min, max}, value, path, seen) when is_list(value) do
    if length(value) < min or (max != nil and length(value) > max),
      do: fail(path, "List length is outside bounds")

    value
    |> Enum.with_index()
    |> Enum.map(fn {item, i} -> cast(schema, inner, item, "#{path}[#{i}]", seen) end)
  end

  defp cast(_, {:list, _, _, _}, _, path, _), do: fail(path, "Expected list")

  defp cast(_, {:enum, values}, value, path, _) do
    if value in values, do: value, else: fail(path, "Unknown enum value")
  end

  defp cast(_, {:number, kind, lo, hi, scale}, value, path, _) do
    number = number(value, path)

    if Decimal.compare(number, lo) == :lt or Decimal.compare(number, hi) == :gt,
      do: fail(path, "Number is outside range")

    normalized = Decimal.normalize(number)
    if normalized.exp < -scale, do: fail(path, "Number has too many decimal places")
    if kind == "integer", do: Decimal.to_integer(number), else: Decimal.round(number, scale)
  end

  defp cast(schema, {:record, fields}, value, path, seen)
       when is_map(value) and not is_struct(value) do
    stored =
      Map.reject(fields, fn {_, field} -> field.options["inverse"] || field.options["stream"] end)

    extras = Map.keys(value) -- Map.keys(stored)
    if extras != [], do: fail(path, "Unknown fields: #{Enum.map_join(extras, ", ", &inspect/1)}")

    Map.new(stored, fn {name, field} ->
      input =
        case Map.fetch(value, name) do
          {:ok, input} ->
            input

          :error ->
            case Schema.resolve(schema, field.type) do
              {:optional, _} -> nil
              _ -> fail(path <> "." <> name, "Required field is missing")
            end
        end

      {name, cast(schema, field.type, input, path <> "." <> name, seen)}
    end)
  end

  defp cast(_, {:record, _}, _, path, _), do: fail(path, "Expected record")
  defp cast(_, {:image, limits}, value, _path, _), do: Flow.Image.decode!(value, limits)

  defp number(%Decimal{coef: coefficient} = value, _) when is_integer(coefficient), do: value
  defp number(value, _) when is_integer(value), do: Decimal.new(value)

  defp number(_, path),
    do: fail(path, "Expected an exact number; decode JSON with floats: :decimals")

  defp datetime(%DateTime{} = value, _), do: value

  defp datetime(value, path) when is_binary(value) do
    case DateTime.from_iso8601(value) do
      {:ok, time, 0} -> time
      {:ok, time, _} -> time
      _ -> fail(path, "Expected ISO 8601 datetime with timezone")
    end
  end

  defp datetime(_, path), do: fail(path, "Expected datetime")

  defp check_id(value, path) do
    unless is_binary(value) and String.valid?(value) and byte_size(value) in 1..256 and
             not String.contains?(value, <<0>>),
           do: fail(path, "ID must be a nonempty UTF-8 string of at most 256 bytes")
  end

  defp fail(path, message),
    do: raise(ValidationError, code: :invalid_value, message: path <> ": " <> message)
end
