defmodule Flow.Schema do
  @moduledoc "Declaration index and checked data model for a Flow program."
  alias Flow.Syntax, as: S
  alias Flow.Types

  defstruct types: %{},
            entities: %{},
            streams: %{},
            operations: %{},
            plugins: %{},
            permissions: [],
            auth: nil,
            transports: %{},
            seeds: []

  @declarations ~w(type enum entity stream query mutate plugin permissions auth transport seed)

  def compile(source) when is_binary(source) do
    with {:ok, ast} <- Flow.Parser.parse(source), do: compile(ast)
  end

  def compile(%{kind: :document, value: declarations}) do
    try do
      groups =
        Enum.group_by(declarations, fn node ->
          {kind, _} = S.form(node)

          unless kind in @declarations,
            do: S.fail(node, :unknown_declaration, "Unknown declaration #{kind}")

          kind
        end)

      data = index(Enum.flat_map(~w(type enum entity stream), &Map.get(groups, &1, [])))
      names = Map.keys(data) |> MapSet.new()

      if Enum.any?(Types.builtins(), &Map.has_key?(data, &1)),
        do: S.fail(hd(declarations), :reserved_name, "Built-in types cannot be redefined")

      schema =
        Enum.reduce(data, %__MODULE__{}, fn {name, node}, schema ->
          {kind, [_ | args]} = S.form(node)

          case kind do
            "type" ->
              unless length(args) == 1, do: S.fail(node, :arity, "type requires one definition")
              put_in(schema.types[name], Types.compile(hd(args), names))

            "enum" ->
              if args == [], do: S.fail(node, :empty_enum, "Enum must contain values")
              values = Enum.map(args, &S.identifier/1)

              if length(Enum.uniq(values)) != length(values),
                do: S.fail(node, :duplicate, "Duplicate enum value")

              put_in(schema.types[name], {:enum, values})

            kind when kind in ["entity", "stream"] ->
              {fields, options} = Enum.split_with(args, fn n -> elem(S.form(n), 0) == "field" end)
              options = S.options(options, if(kind == "stream", do: ~w(mqtt history), else: []))
              fields = Types.fields(fields, names)

              fields =
                if kind == "stream" do
                  fields
                  |> Map.put_new("id", %{
                    type: {:id, name},
                    options: %{"generated" => true},
                    node: node
                  })
                  |> Map.put_new("receivedAt", %{
                    type: {:named, "DateTime"},
                    options: %{"receivedAt" => true},
                    node: node
                  })
                else
                  fields
                end

              model = %{fields: fields, options: options, node: node}

              if kind == "entity",
                do: put_in(schema.entities[name], model),
                else: put_in(schema.streams[name], model)
          end
        end)

      check_models!(schema)
      operations = index(Enum.flat_map(~w(query mutate), &Map.get(groups, &1, [])))
      plugins = index(Map.get(groups, "plugin", []))
      transports = index(Map.get(groups, "transport", []))

      auth =
        case Map.get(groups, "auth", []) do
          [] -> nil
          [node] -> node
          [_, duplicate | _] -> S.fail(duplicate, :duplicate, "Duplicate auth declaration")
        end

      schema = %{
        schema
        | operations: operations,
          plugins: plugins,
          transports: transports,
          auth: auth,
          permissions: Map.get(groups, "permissions", []),
          seeds: Map.get(groups, "seed", [])
      }

      check_permissions!(schema)
      {:ok, schema}
    rescue
      error in Flow.ValidationError -> {:error, error}
    end
  end

  def compile!(source) do
    case compile(source) do
      {:ok, schema} -> schema
      {:error, error} -> raise error
    end
  end

  def resolve(schema, type, seen \\ MapSet.new())

  def resolve(schema, {:named, name}, seen) do
    if MapSet.member?(seen, name), do: raise(ArgumentError, "Cyclic type #{name}")

    case schema.types[name] do
      {:named, _} = inner -> resolve(schema, inner, MapSet.put(seen, name))
      nil -> {:named, name}
      inner -> inner
    end
  end

  def resolve(_schema, type, _seen), do: type

  defp index(nodes) do
    S.unique(nodes, fn node ->
      case S.form(node) do
        {_, [name | _]} -> S.identifier(name)
        _ -> S.fail(node, :arity, "Declaration requires a name")
      end
    end)
  end

  defp check_models!(schema) do
    models = Map.merge(schema.entities, schema.streams)
    Enum.each(schema.types, fn {name, type} -> check_type!(schema, type, [name], nil) end)

    Enum.each(models, fn {name, model} ->
      Enum.each(model.fields, fn {_field, definition} ->
        check_type!(schema, definition.type, [], definition.node)

        for relation <- ~w(inverse stream), option = definition.options[relation] do
          [path] = S.args(option, relation, 1)

          case String.split(S.symbol(path), ".") do
            [entity, field] ->
              target =
                models[entity] ||
                  S.fail(path, :unknown_relation, "Unknown relation entity #{entity}")

              target_field =
                target.fields[field] ||
                  S.fail(path, :unknown_relation, "Unknown relation field #{entity}.#{field}")

              unless target_field.type == {:named, name},
                do: S.fail(path, :invalid_relation, "Relation must point back to #{name}")

              case definition.type do
                {:list, {:named, ^entity}, _, _} -> :ok
                _ -> S.fail(path, :invalid_relation, "Relation field must be a list of #{entity}")
              end

              if relation == "stream" and not Map.has_key?(schema.streams, entity),
                do: S.fail(path, :invalid_relation, "stream requires a stream target")

            _ ->
              S.fail(path, :invalid_relation, "Relation requires Entity.field")
          end
        end
      end)

      if Map.has_key?(schema.entities, name) do
        field =
          model.fields["id"] || S.fail(model.node, :missing_id, "Entity #{name} requires id")

        unless resolve(schema, field.type) == {:id, name},
          do: S.fail(field.node, :invalid_id, "Entity id must be branded for #{name}")
      end
    end)
  end

  defp check_type!(schema, {:named, name}, seen, node) do
    if name in seen, do: S.fail(node || %{span: nil}, :cyclic_type, "Cyclic type #{name}")

    case schema.types[name] do
      nil -> :ok
      type -> check_type!(schema, type, [name | seen], node)
    end
  end

  defp check_type!(schema, {:id, entity}, _, node) do
    unless entity in ["Request", "Job"] or Map.has_key?(schema.entities, entity) or
             Map.has_key?(schema.streams, entity),
           do: S.fail(node || %{span: nil}, :invalid_id, "ID refers to unknown entity #{entity}")
  end

  defp check_type!(schema, {:optional, inner}, seen, node),
    do: check_type!(schema, inner, seen, node)

  defp check_type!(schema, {:list, inner, _, _}, seen, node),
    do: check_type!(schema, inner, seen, node)

  defp check_type!(schema, {:record, fields}, seen, _) do
    Enum.each(fields, fn {_, field} -> check_type!(schema, field.type, seen, field.node) end)
  end

  defp check_type!(_, _, _, _), do: :ok

  defp check_permissions!(schema) do
    targets = Map.keys(schema.entities) ++ Map.keys(schema.streams) ++ Map.keys(schema.types)

    targets =
      targets ++
        Enum.flat_map(schema.plugins, fn {name, node} ->
          {_, [_ | methods]} = S.form(node)

          Enum.map(methods, fn method ->
            {key, _} = S.form(method)
            name <> "." <> key
          end)
        end)

    Enum.each(schema.permissions, fn node ->
      {actor, rules} =
        case S.form(node) do
          {"permissions", [actor | rules]} -> {actor, rules}
          _ -> S.fail(node, :arity, "permissions requires an actor")
        end

      actor = S.symbol(actor)

      unless actor in ["*", "Anonymous"] or Map.has_key?(schema.entities, actor),
        do: S.fail(node, :unknown_actor, "Unknown permission actor #{actor}")

      Enum.each(rules, fn rule ->
        {target, grants} = S.form(rule)
        root = hd(String.split(target, "."))

        unless target in targets or Map.has_key?(schema.entities, root),
          do: S.fail(rule, :unknown_target, "Unknown permission target #{target}")

        Enum.each(grants, fn grant ->
          {action, options} = S.form(grant)

          unless action in ~w(READ USE CREATE UPDATE DELETE INVOKE),
            do: S.fail(grant, :unknown_action, "Unknown permission action #{action}")

          options = S.options(options, ~w(when includes))

          condition =
            options["when"] || S.fail(grant, :missing_condition, "A grant requires when")

          S.args(condition, "when", 1)

          if includes = options["includes"] do
            {_, actions} = S.form(includes)

            if actions == [] or
                 Enum.any?(
                   actions,
                   &(S.symbol(&1) not in ~w(READ USE CREATE UPDATE DELETE INVOKE))
                 ),
               do: S.fail(includes, :unknown_action, "Invalid included permission action")
          end
        end)
      end)
    end)
  end
end
