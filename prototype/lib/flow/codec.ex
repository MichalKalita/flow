defmodule Flow.Codec do
  @moduledoc false
  alias Flow.{Schema, Value, Embedded}

  def decode(schema, type, value, parent \\ nil)
  def decode(_, {:optional, _}, nil, _), do: nil
  def decode(schema, {:optional, inner}, value, parent), do: decode(schema, inner, value, parent)

  def decode(schema, {:named, name} = type, value, parent) do
    case Schema.resolve(schema, type) do
      {:record, fields} ->
        %Embedded{
          type: name,
          parent: parent,
          value: decode(schema, {:record, fields}, value, parent)
        }

      ^type ->
        Value.validate!(schema, type, external(type, value))

      inner ->
        decode(schema, inner, value, parent)
    end
  end

  def decode(schema, {:record, fields}, value, parent) do
    value = json(value)

    Map.new(fields, fn {name, field} ->
      {name, decode(schema, field.type, Map.fetch!(value, name), parent)}
    end)
  end

  def decode(schema, {:list, inner, _, _}, value, parent) do
    value |> json() |> Enum.map(&decode(schema, inner, &1, parent))
  end

  def decode(schema, {:number, kind, _, _, _} = type, value, _) do
    value = if is_binary(value), do: Decimal.new(value), else: value

    value =
      if kind == "integer" and is_struct(value, Decimal),
        do: Decimal.to_integer(value),
        else: value

    Value.validate!(schema, type, value)
  end

  def decode(schema, {:image, _} = type, %{"$image" => image}, _),
    do: Flow.Value.validate!(schema, type, Base.decode64!(image))

  def decode(schema, type, value, _), do: Value.validate!(schema, type, value)

  defp external({:named, "Bool"}, 0), do: false
  defp external({:named, "Bool"}, 1), do: true
  defp external(_, value), do: value
  defp json(value) when is_binary(value), do: Jason.decode!(value, floats: :decimals)
  defp json(value), do: value

  def unembed(%Embedded{value: value}), do: unembed(value)
  def unembed(values) when is_list(values), do: Enum.map(values, &unembed/1)

  def unembed(values) when is_map(values) and not is_struct(values),
    do: Map.new(values, fn {k, v} -> {k, unembed(v)} end)

  def unembed(value), do: value
end
