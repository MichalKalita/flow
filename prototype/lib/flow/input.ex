defmodule Flow.Input do
  @moduledoc false
  alias Flow.{Schema, Access, Value}
  def materialize(session, type, value, mode \\ :read)
  def materialize(_session, {:optional, _}, nil, _mode), do: nil

  def materialize(session, {:optional, inner}, value, mode),
    do: materialize(session, inner, value, mode)

  def materialize(session, {:list, inner, min, max}, values, mode) do
    values = if mode == :read, do: Access.visible(values, session), else: values

    if length(values) < min or (max != nil and length(values) > max),
      do: raise(Flow.ValidationError, code: :invalid_value, message: "Input list outside bounds")

    Enum.map(values, &materialize(session, inner, &1, mode))
  end

  def materialize(session, {:named, _} = type, value, mode) do
    case Schema.resolve(session.db.schema, type) do
      {:record, fields} -> materialize(session, {:record, fields}, value, mode)
      resolved -> Value.validate!(session.db.schema, resolved, value)
    end
  end

  def materialize(session, {:record, fields}, value, mode) do
    if is_map(value) and not is_struct(value) and Map.keys(value) -- Map.keys(fields) != [],
      do: raise(Flow.ValidationError, code: :invalid_value, message: "Unknown plugin input field")

    Map.new(fields, fn {name, field} ->
      input =
        if mode == :use and is_struct(value),
          do: Access.using_field(session, value, name),
          else: Access.field(session, value, name)

      {name, materialize(session, field.type, input, mode)}
    end)
  end

  def materialize(session, type, value, _), do: Value.validate!(session.db.schema, type, value)

  def use_value(session, %Flow.Embedded{type: type} = value),
    do: materialize(session, {:named, type}, value, :use)

  def use_value(session, values) when is_list(values),
    do: Enum.map(values, &use_value(session, &1))

  def use_value(_, value), do: value
end
