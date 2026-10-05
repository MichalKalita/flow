defmodule Flow.Reads do
  @moduledoc false

  def capture(session, function) do
    scope = make_ref()
    prior = stack(session)
    :ets.insert(session.cache, {:read_scopes, [scope | prior]})
    :ets.insert(session.cache, {{:reads, scope}, MapSet.new()})

    try do
      value = function.()
      [{_, reads}] = :ets.lookup(session.cache, {:reads, scope})
      {value, MapSet.to_list(reads)}
    after
      :ets.insert(session.cache, {:read_scopes, prior})
      :ets.delete(session.cache, {:reads, scope})
    end
  end

  def add(session, target, field, action),
    do: propagate(session, [%{target: target, field: field, action: action}])

  def propagate(session, sources) do
    case stack(session) do
      [scope | _] ->
        [{_, reads}] = :ets.lookup(session.cache, {:reads, scope})

        :ets.insert(
          session.cache,
          {{:reads, scope}, Enum.reduce(sources, reads, &MapSet.put(&2, &1))}
        )

      [] ->
        :ok
    end
  end

  defp stack(session) do
    case :ets.lookup(session.cache, :read_scopes) do
      [{_, scopes}] -> scopes
      [] -> []
    end
  end

  def encode(sources) do
    Enum.map(sources, fn source ->
      target =
        case source.target do
          %Flow.Ref{entity: entity, id: id} ->
            %{"entity" => entity, "id" => id.value}

          %Flow.Embedded{type: type, value: value, parent: parent} ->
            %{
              "record" => type,
              "value" => value,
              "parent" => %{"entity" => parent.entity, "id" => parent.id.value}
            }
        end

      %{"target" => target, "field" => source.field, "action" => source.action}
    end)
  end

  def authorize!(session, proof) do
    for source <- proof do
      target =
        if record = source["target"]["record"] do
          parent = reference(source["target"]["parent"])
          # Embedded values have no independent identity: the containing entity
          # must still be readable when this immutable value is delivered.
          unless Flow.Access.allowed?(session, "READ", parent) or
                   Flow.Access.allowed?(session, "USE", parent) do
            forbidden!()
          end

          decoded =
            Flow.Codec.decode(
              session.db.schema,
              {:named, record},
              source["target"]["value"],
              parent
            )

          decoded
        else
          reference(source["target"])
        end

      case source["action"] do
        "READ" -> Flow.Access.field(session, target, source["field"])
        "USE" -> Flow.Access.using_field(session, target, source["field"])
      end
    end

    :ok
  end

  defp reference(value),
    do: %Flow.Ref{
      entity: value["entity"],
      id: %Flow.ID{entity: value["entity"], value: value["id"]}
    }

  defp forbidden!,
    do: raise(Flow.ValidationError, code: :forbidden, message: "Queued data no longer accessible")
end
