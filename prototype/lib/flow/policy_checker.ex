defmodule Flow.PolicyChecker do
  @moduledoc "Checks policy types and rejects nonmonotone recursive permission definitions."
  alias Flow.{Checker, Syntax}

  def validate!(schema, permissions, plugins) do
    actors = Map.keys(schema.entities) ++ ["Anonymous"]

    names =
      MapSet.new(Map.keys(schema.types) ++ Map.keys(schema.entities) ++ Map.keys(schema.streams))

    edges =
      Enum.flat_map(permissions.grants, fn {{actor, target}, grants} ->
        wildcard = actor == "*"

        for actor <- if(wildcard, do: actors, else: [actor]),
            grant <- grants,
            source_action <- grant.actions,
            reduce: [] do
          edges ->
            context = context(schema, names, plugins, actor, target, source_action, wildcard)
            type = Checker.infer(grant.condition, context)
            Checker.compatible!(schema, type, {:named, "Bool"}, grant.condition)
            deps = dependencies(grant.condition, context, true)

            edges ++
              Enum.flat_map(deps, fn {actors, targets, action, positive, node} ->
                for dependency_actor <- actors,
                    dependency_target <- targets,
                    do:
                      {{actor, target, source_action},
                       {dependency_actor, dependency_target, action}, positive, node}
              end)
        end
      end)

    # Source action is attached separately below; includes has already expanded
    # the grant action set, so each inherited action receives the same dependencies.
    reject_negative_cycles!(edges)
    :ok
  end

  defp context(schema, names, plugins, actor, target, action, wildcard) do
    root = hd(String.split(target, "."))
    type = {:named, root}

    env = %{
      "actor" =>
        cond do
          wildcard -> :any_actor
          actor == "Anonymous" -> :anonymous
          true -> {:named, actor}
        end,
      "target" => type,
      "context" =>
        if(schema.types["Context"],
          do: {:named, "Context"},
          else: {:shape, %{"now" => {:named, "DateTime"}}}
        )
    }

    env =
      if action in ~w(CREATE UPDATE DELETE INVOKE),
        do: Map.put(env, "transaction", :transaction),
        else: env

    env = if action in ~w(UPDATE DELETE), do: Map.put(env, "before", type), else: env
    env = if action in ~w(CREATE UPDATE), do: Map.put(env, "after", type), else: env

    env =
      if action in ~w(CREATE UPDATE DELETE),
        do: Map.put(env, "changed", {:list, {:named, "String"}, 0, nil}),
        else: env

    env =
      if contract = plugins[target] do
        unless action == "INVOKE",
          do:
            Syntax.fail(
              contract.node,
              :invalid_permission,
              "Plugin permissions only support INVOKE"
            )

        Map.put(
          env,
          "args",
          {:shape, Map.new(contract.inputs, fn {key, input} -> {key, input.type} end)}
        )
      else
        env
      end

    parents = parent_types(schema, root)

    env =
      if parents != [],
        do: Map.put(env, "parent", {:union, Enum.map(parents, &{:named, &1})}),
        else: env

    %{
      schema: schema,
      names: names,
      plugins: plugins,
      kind: "policy",
      env: env,
      bindings: %{},
      seen: [],
      actor: actor
    }
  end

  defp parent_types(schema, record) do
    for {name, model} <- Map.merge(schema.entities, schema.streams),
        Enum.any?(model.fields, fn {_, field} ->
          contains_type?(schema, field.type, record, [])
        end),
        do: name
  end

  defp contains_type?(_, {:named, name}, name, _), do: true

  defp contains_type?(schema, {:named, name}, record, seen) do
    if name in seen do
      false
    else
      case schema.types[name] do
        {:record, fields} ->
          Enum.any?(fields, fn {_, f} -> contains_type?(schema, f.type, record, [name | seen]) end)

        {:list, inner, _, _} ->
          contains_type?(schema, inner, record, [name | seen])

        {:optional, inner} ->
          contains_type?(schema, inner, record, [name | seen])

        _ ->
          false
      end
    end
  end

  defp contains_type?(schema, {:list, inner, _, _}, record, seen),
    do: contains_type?(schema, inner, record, seen)

  defp contains_type?(schema, {:optional, inner}, record, seen),
    do: contains_type?(schema, inner, record, seen)

  defp contains_type?(_, _, _, _), do: false

  defp dependencies(%{kind: :list} = node, context, positive) do
    {operator, args} = Syntax.form(node)

    case {operator, args} do
      {"can", [action, target]} ->
        [
          {[context.actor], reference_types(Checker.infer(target, context)),
           Syntax.symbol(action), positive, node}
        ]

      {"canAs", [actor, action, target]} ->
        [
          {reference_types(Checker.infer(actor, context)),
           reference_types(Checker.infer(target, context)), Syntax.symbol(action), positive, node}
        ]

      {"not", [value]} ->
        dependencies(value, context, not positive)

      {op, args} when op in ~w(and or) ->
        Enum.flat_map(args, &dependencies(&1, context, positive))

      {op, [collection, variable, body]} when op in ~w(any all where flatMap sum) ->
        {:list, inner, _, _} = Checker.infer(collection, context)
        local = %{context | env: Map.put(context.env, Syntax.symbol(variable), inner)}

        dependencies(collection, context, false) ++
          dependencies(body, local, if(op in ~w(any all), do: positive, else: false))

      _ ->
        Enum.flat_map(args, &dependencies(&1, context, false))
    end
  end

  defp dependencies(_, _, _), do: []
  defp reference_types({:named, name}), do: [name]
  defp reference_types({:optional, inner}), do: reference_types(inner)

  defp reference_types({:union, types}),
    do: Enum.flat_map(types, &reference_types/1) |> Enum.uniq()

  defp reject_negative_cycles!(edges) do
    graph = :digraph.new()

    try do
      for {from, to, _, _} <- edges do
        :digraph.add_vertex(graph, from)
        :digraph.add_vertex(graph, to)
        :digraph.add_edge(graph, from, to)
      end

      for {from, to, false, node} <- edges do
        if from == to or :digraph.get_path(graph, to, from) != false,
          do:
            Syntax.fail(
              node,
              :negative_permission_cycle,
              "Recursive permissions must be positive"
            )
      end
    after
      :digraph.delete(graph)
    end
  end
end
