defmodule Flow.Access do
  @moduledoc "Request-scoped reads with current data permissions and a private lazy field cache."
  alias Flow.{Store, Ref, Embedded, Expression, Permissions, Codec, Version, Transaction}
  defstruct [:db, :policy, :principal, :context, :cache, :counter]

  def with_session(db, policy, principal, context, function) do
    cache = :ets.new(__MODULE__, [:set, :private])

    session = %__MODULE__{
      db: db,
      policy: policy,
      principal: principal,
      context: context,
      cache: cache,
      counter: :counters.new(1, [])
    }

    try do
      function.(session)
    after
      :ets.delete(cache)
    end
  end

  def allowed?(session, action, target),
    do: allowed?(session, session.principal, action, target, MapSet.new())

  def granted?(session, action, target, facts \\ %{}) do
    evaluate = evaluator(session, session.principal, target, MapSet.new(), facts)

    Permissions.allowed?(
      session.policy,
      session.principal.type,
      if(is_binary(target), do: target, else: target_type(target)),
      action,
      evaluate
    )
  end

  def granted_field?(session, action, target, field, facts \\ %{}) do
    evaluate = evaluator(session, session.principal, target, MapSet.new(), facts)
    name = target_type(target) <> "." <> field

    not MapSet.member?(session.policy.field_targets, name) or
      Permissions.allowed?(session.policy, session.principal.type, name, action, evaluate)
  end

  defp allowed?(session, principal, action, target, seen) do
    type = target_type(target)
    key = {principal.identity, action, target}

    if type == nil or MapSet.member?(seen, key) do
      false
    else
      evaluate =
        evaluator(session, principal, policy_target(session, target), MapSet.put(seen, key))

      Permissions.allowed?(session.policy, principal.type, type, action, evaluate)
    end
  end

  def field(session, %Ref{} = reference, field) do
    evaluator =
      evaluator(session, session.principal, policy_target(session, reference), MapSet.new())

    unless Permissions.field_allowed?(
             session.policy,
             session.principal.type,
             reference.entity,
             field,
             "READ",
             evaluator
           ),
           do: denied!()

    raw_field(session, reference, field) |> visible(session)
  end

  def field(session, %Embedded{} = embedded, field) do
    evaluator = evaluator(session, session.principal, embedded, MapSet.new())

    unless Permissions.field_allowed?(
             session.policy,
             session.principal.type,
             embedded.type,
             field,
             "READ",
             evaluator
           ),
           do: denied!()

    raw_field(session, embedded, field) |> visible(session)
  end

  def field(session, values, field) when is_list(values),
    do: values |> visible(session) |> Enum.map(&field(session, &1, field))

  def field(_session, map, field) when is_map(map) and not is_struct(map),
    do: Map.fetch!(map, field)

  def field(_, _, _), do: raise(ArgumentError, "Cannot read field")

  def visible(values, session) when is_list(values) do
    Enum.filter(values, fn value ->
      target_type(value) == nil or allowed?(session, "READ", value)
    end)
  end

  def visible(value, _), do: value

  def raw_field(session, %Ref{} = reference, field),
    do: version_field(session, reference, field, :after)

  def raw_field(session, %Version{reference: reference, state: state}, field),
    do: version_field(session, reference, field, state)

  def raw_field(_session, %Embedded{value: value}, field), do: Map.fetch!(value, field)

  def raw_field(session, values, field) when is_list(values),
    do: Enum.map(values, &raw_field(session, &1, field))

  def raw_field(_session, map, field) when is_map(map) and not is_struct(map),
    do: Map.fetch!(map, field)

  def raw_field(_, _, _), do: raise(ArgumentError, "Cannot resolve policy field")

  defp evaluator(session, principal, target, seen, facts \\ %{}) do
    env =
      %{
        "actor" => pin(principal.identity, :before),
        "target" => target,
        "context" => session.context,
        "parent" => if(is_struct(target, Embedded), do: target.parent, else: nil)
      }
      |> Map.merge(facts)

    constants =
      session.db.schema.types
      |> Enum.flat_map(fn
        {_, {:enum, values}} -> Enum.map(values, &{&1, &1})
        _ -> []
      end)
      |> Map.new()
      |> Map.merge(Map.new(~w(READ USE CREATE UPDATE DELETE INVOKE), &{&1, &1}))

    fn condition ->
      Expression.evaluate(condition, env,
        counter: session.counter,
        constants: constants,
        field: &raw_field(session, &1, &2),
        creates: &Transaction.creates/2,
        can: fn actor, action, target, _counter ->
          type = target_type(actor)

          if type,
            do: allowed?(session, %{type: type, identity: actor}, action, target, seen),
            else: false
        end
      )
    end
  end

  defp target_type(%Ref{entity: entity}), do: entity
  defp target_type(%Version{reference: reference}), do: target_type(reference)
  defp target_type(%Embedded{type: type}), do: type
  defp target_type(_), do: nil

  defp policy_target(session, %Ref{} = reference) do
    _ = session
    %Version{reference: reference, state: :after}
  end

  defp policy_target(_, value), do: value

  defp version_field(session, %Ref{entity: entity} = reference, field, state) do
    model = session.db.schema.entities[entity] || session.db.schema.streams[entity]
    definition = Map.fetch!(model.fields, field)

    value =
      cond do
        relation = definition.options["inverse"] || definition.options["stream"] ->
          {_, [%{value: path}]} = Flow.Syntax.form(relation)
          [target, foreign] = String.split(path, ".")
          original = Store.related(session.db, target, foreign, reference)
          Transaction.related(session, original, target, foreign, reference, state)

        true ->
          raw =
            case Transaction.read(session, reference, field, state) do
              {:ok, value} -> value
              _ -> original_field(session, reference, field)
            end

          Codec.decode(session.db.schema, definition.type, raw, reference)
      end

    if state == :before, do: pin(value, :before), else: value
  end

  defp original_field(session, reference, field) do
    key = {:original, reference, field}

    case :ets.lookup(session.cache, key) do
      [{_, value}] ->
        value

      [] ->
        row = Store.fetch(session.db, reference, [field]) || denied!()
        :ets.insert(session.cache, {key, row[field]})
        row[field]
    end
  end

  defp pin(%Ref{} = reference, state), do: %Version{reference: reference, state: state}

  defp pin(%Embedded{} = value, state),
    do: %{value | value: pin(value.value, state), parent: pin(value.parent, state)}

  defp pin(values, state) when is_list(values), do: Enum.map(values, &pin(&1, state))

  defp pin(values, state) when is_map(values) and not is_struct(values),
    do: Map.new(values, fn {k, v} -> {k, pin(v, state)} end)

  defp pin(value, _), do: value
  defp denied!, do: raise(Flow.ValidationError, code: :not_found, message: "Resource unavailable")
end
