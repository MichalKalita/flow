defmodule Flow.Projection do
  @moduledoc "Reads precisely the declared output fields through the permission-aware accessor."
  alias Flow.{Access, Schema, Value}

  def project(session, type, value) do
    schema = session.db.schema

    case type do
      {:named, name} ->
        case Schema.resolve(schema, type) do
          {:record, fields} ->
            record(session, fields, value)

          ^type ->
            model = schema.entities[name] || schema.streams[name]

            if model,
              do: record(session, model.fields, value),
              else: Value.validate!(schema, type, value)

          resolved ->
            project(session, resolved, value)
        end

      {:record, fields} ->
        record(session, fields, value)

      {:optional, _} when value == nil ->
        nil

      {:optional, inner} ->
        project(session, inner, value)

      {:list, inner, min, max} ->
        values = Access.visible(value, session)

        if not is_list(values) or length(values) < min or (max != nil and length(values) > max),
          do:
            raise(Flow.ValidationError,
              code: :invalid_output,
              message: "Output list is outside bounds"
            )

        Enum.map(values, &project(session, inner, &1))

      _ ->
        Value.validate!(schema, type, value)
    end
  end

  defp record(session, fields, value) do
    Map.new(fields, fn {name, definition} ->
      {name, project(session, definition.type, Access.field(session, value, name))}
    end)
  end
end
