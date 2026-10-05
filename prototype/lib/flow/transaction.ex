defmodule Flow.Transaction do
  @moduledoc "Stages typed changes and authorizes the complete proposed transaction before SQL writes."
  alias Flow.{Access, Ref, Store, Value, Version, Expression}

  def changes(session), do: lookup(session, :changes, [])
  def invocations(session), do: lookup(session, :invocations, [])
  def change(session, reference), do: Enum.find(changes(session), &(&1.reference == reference))

  def create(session, entity, input) do
    definitions = Store.fields(session.db, entity)
    input = Value.validate!(session.db.schema, {:record, definitions}, input)
    id = Map.fetch!(input, "id")
    reference = %Ref{entity: entity, id: id}

    if change(session, reference) || Store.fetch(session.db, reference, ["id"]),
      do: raise(Flow.ValidationError, code: :conflict, message: "Resource already exists")

    stage(session, %{
      reference: reference,
      action: "CREATE",
      after: input,
      changed: Map.keys(input)
    })

    reference
  end

  def set(session, %Ref{} = reference, field, value) do
    definitions = Store.fields(session.db, reference.entity)

    if field == "id",
      do: raise(Flow.ValidationError, code: :immutable_id, message: "Entity id cannot change")

    definition = Map.fetch!(definitions, field)
    value = Value.validate!(session.db.schema, definition.type, value)

    case change(session, reference) do
      nil ->
        unless Store.fetch(session.db, reference, ["id"]), do: unavailable!()

        stage(session, %{
          reference: reference,
          action: "UPDATE",
          after: %{field => value},
          changed: [field]
        })

      %{action: "DELETE"} ->
        unavailable!()

      change ->
        stage(session, %{
          change
          | after: Map.put(change.after, field, value),
            changed: Enum.uniq(change.changed ++ [field])
        })
    end

    value
  end

  def delete(session, %Ref{} = reference) do
    unless Store.fetch(session.db, reference, ["id"]), do: unavailable!()
    stage(session, %{reference: reference, action: "DELETE", after: %{}, changed: []})
    true
  end

  def invocation(session, method, args, contract) do
    type =
      {:record,
       Map.new(contract.inputs, fn {key, input} -> {key, %{type: input.type, options: %{}}} end)}

    args = Value.validate!(session.db.schema, type, args)
    put(session, :invocations, invocations(session) ++ [%{method: method, args: args}])
    args
  end

  def read(session, reference, field, state) do
    if state == :before do
      :original
    else
      case change(session, reference) do
        %{action: "DELETE"} -> unavailable!()
        %{after: values} -> Map.fetch(values, field)
        nil -> :original
      end
    end
  end

  def related(session, original, entity, foreign, reference, state) do
    if state == :before do
      original
    else
      affected = Enum.filter(changes(session), &(&1.reference.entity == entity))
      removed = Enum.map(affected, & &1.reference)

      additions =
        Enum.filter(affected, fn change ->
          change.action != "DELETE" and
            Expression.equal?(Access.raw_field(session, change.reference, foreign), reference)
        end)
        |> Enum.map(& &1.reference)

      (original -- removed) ++ additions
    end
  end

  def authorize!(session) do
    for change <- changes(session) do
      before = %Version{reference: change.reference, state: :before}
      after_state = %Version{reference: change.reference, state: :after}
      target = if change.action == "CREATE", do: after_state, else: before

      facts = %{
        "before" => before,
        "after" => after_state,
        "changed" => change.changed,
        "transaction" => session
      }

      unless Access.granted?(session, change.action, target, facts), do: forbidden!()

      for field <- change.changed do
        unless Access.granted_field?(session, change.action, target, field, facts),
          do: forbidden!()
      end

      check_references!(session, change)
    end

    for invocation <- invocations(session) do
      unless Access.granted?(session, "INVOKE", invocation.method, %{
               "args" => invocation.args,
               "transaction" => session
             }),
             do: forbidden!()
    end

    :ok
  end

  def apply!(session) do
    for change <- changes(session) do
      case change.action do
        "CREATE" -> Store.insert(session.db, change.reference.entity, change.after)
        "UPDATE" -> Store.update(session.db, change.reference, change.after)
        "DELETE" -> Store.delete(session.db, change.reference)
      end
    end

    :ok
  end

  def creates(session, entity),
    do:
      changes(session)
      |> Enum.filter(&(&1.action == "CREATE" and &1.reference.entity == entity))
      |> Enum.map(& &1.reference)

  defp check_references!(session, change) do
    Enum.each(change.after, fn {_, value} -> check_value!(session, value) end)
  end

  defp check_value!(session, %Ref{} = reference) do
    case change(session, reference) do
      %{action: "CREATE"} -> :ok
      %{action: "DELETE"} -> unavailable!()
      _ -> unless Store.fetch(session.db, reference, ["id"]), do: unavailable!()
    end
  end

  defp check_value!(session, values) when is_list(values),
    do: Enum.each(values, &check_value!(session, &1))

  defp check_value!(session, value) when is_map(value) and not is_struct(value),
    do: Enum.each(value, fn {_, value} -> check_value!(session, value) end)

  defp check_value!(_, _), do: :ok

  defp stage(session, change),
    do:
      put(
        session,
        :changes,
        Enum.reject(changes(session), &(&1.reference == change.reference)) ++ [change]
      )

  defp lookup(session, key, default) do
    case :ets.lookup(session.cache, {:transaction, key}) do
      [{_, value}] -> value
      [] -> default
    end
  end

  defp put(session, key, value), do: :ets.insert(session.cache, {{:transaction, key}, value})

  defp unavailable!,
    do: raise(Flow.ValidationError, code: :not_found, message: "Resource unavailable")

  defp forbidden!,
    do: raise(Flow.ValidationError, code: :forbidden, message: "Operation forbidden")
end
