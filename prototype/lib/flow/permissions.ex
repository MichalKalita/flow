defmodule Flow.Permissions do
  @moduledoc "Positive grants with explicitly scoped action inheritance and field restrictions."
  alias Flow.Syntax, as: S
  defstruct grants: %{}, field_targets: MapSet.new()

  def compile(schema) do
    try do
      groups =
        Enum.reduce(schema.permissions, %{}, fn node, groups ->
          {"permissions", [actor | rules]} = S.form(node)
          actor = S.symbol(actor)

          Enum.reduce(rules, groups, fn rule, groups ->
            {target, grants} = S.form(rule)
            validate_target!(schema, target, rule)
            Map.update(groups, {actor, target}, grants, &(&1 ++ grants))
          end)
        end)

      compiled = Map.new(groups, fn {key, grants} -> {key, compile_grants!(grants)} end)

      fields =
        groups
        |> Map.keys()
        |> Enum.map(&elem(&1, 1))
        |> Enum.filter(&field_target?(schema, &1))
        |> MapSet.new()

      {:ok, %__MODULE__{grants: compiled, field_targets: fields}}
    rescue
      error in Flow.ValidationError ->
        {:error, error}

      error in [MatchError, CaseClauseError, KeyError] ->
        {:error,
         %Flow.ValidationError{
           code: :invalid_permission,
           message: "Malformed permission: #{Exception.message(error)}"
         }}
    end
  end

  # The caller is the trusted evaluator. It supplies an authenticated actor type,
  # current context and a bounded evaluator; this API does not authenticate clients.
  def allowed?(policy, actor_type, target_type, action, evaluate) do
    grants =
      Map.get(policy.grants, {actor_type, target_type}, []) ++
        Map.get(policy.grants, {"*", target_type}, [])

    try do
      Enum.any?(grants, fn grant ->
        MapSet.member?(grant.actions, action) and evaluate.(grant.condition) == true
      end)
    rescue
      _ -> false
    catch
      _, _ -> false
    end
  end

  def field_allowed?(policy, actor_type, entity, field, action, evaluate) do
    target = entity <> "." <> field

    allowed?(policy, actor_type, entity, action, evaluate) and
      (not MapSet.member?(policy.field_targets, target) or
         allowed?(policy, actor_type, target, action, evaluate))
  end

  defp compile_grants!(grants) do
    parsed =
      Enum.map(grants, fn node ->
        {action, options} = S.form(node)
        options = S.options(options, ~w(when includes))
        [condition] = S.args(Map.fetch!(options, "when"), "when", 1)

        case Flow.Expression.validate(condition) do
          :ok -> :ok
          {:error, error} -> raise error
        end

        includes =
          case options["includes"] do
            nil ->
              []

            option ->
              {_, values} = S.form(option)
              Enum.map(values, &S.symbol/1)
          end

        %{action: action, includes: includes, condition: condition, node: node}
      end)

    graph =
      Enum.reduce(parsed, %{}, fn grant, graph ->
        Map.update(graph, grant.action, grant.includes, &Enum.uniq(&1 ++ grant.includes))
      end)

    Enum.map(parsed, fn grant ->
      actions = closure!(grant.action, graph, [], grant.node) |> MapSet.new()

      if (MapSet.member?(actions, "READ") or MapSet.member?(actions, "USE")) and
           uses_change_state?(grant.condition),
         do:
           S.fail(
             grant.node,
             :invalid_inheritance,
             "READ/USE grants cannot depend on a proposed change"
           )

      %{actions: actions, condition: grant.condition}
    end)
  end

  defp closure!(action, graph, seen, node) do
    if action in seen,
      do: S.fail(node, :cyclic_inheritance, "Cyclic permission action inheritance")

    [
      action
      | Enum.flat_map(Map.get(graph, action, []), &closure!(&1, graph, [action | seen], node))
    ]
  end

  defp uses_change_state?(%{kind: :symbol, value: value}),
    do: hd(String.split(value, ".")) in ~w(before after changed transaction)

  defp uses_change_state?(%{kind: :list, value: nodes}),
    do: Enum.any?(nodes, &uses_change_state?/1)

  defp uses_change_state?(_), do: false

  defp validate_target!(schema, target, node) do
    case String.split(target, ".") do
      [name] ->
        unless Map.has_key?(schema.entities, name) or Map.has_key?(schema.streams, name) or
                 match?({:record, _}, schema.types[name]),
               do:
                 S.fail(
                   node,
                   :unknown_target,
                   "Permission target must be an entity, stream or record"
                 )

      [name, field] ->
        if plugin = schema.plugins[name] do
          {_, [_ | methods]} = S.form(plugin)

          unless Enum.any?(methods, &(elem(S.form(&1), 0) == field)),
            do: S.fail(node, :unknown_target, "Unknown plugin method #{target}")
        else
          model = schema.entities[name] || schema.streams[name]

          fields =
            if model do
              model.fields
            else
              case schema.types[name] do
                {:record, fields} -> fields
                _ -> %{}
              end
            end

          unless Map.has_key?(fields, field),
            do: S.fail(node, :unknown_target, "Unknown permission field #{target}")
        end

      _ ->
        S.fail(node, :unknown_target, "Invalid permission target #{target}")
    end
  end

  defp field_target?(schema, target),
    do:
      String.contains?(target, ".") and
        not Map.has_key?(schema.plugins, hd(String.split(target, ".")))
end
