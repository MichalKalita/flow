defmodule Flow.Checker do
  @moduledoc false
  alias Flow.{Syntax, Types, Schema, Value}

  def infer(node, context) do
    case node do
      %{kind: :number, value: value} -> {:literal_number, Decimal.new(value)}
      %{kind: :string} -> {:named, "String"}
      %{kind: :symbol, value: value} -> symbol(node, value, context)
      _ -> form(node, context)
    end
  end

  def bindings(nodes) do
    nodes
    |> Syntax.unique(fn node -> elem(Syntax.form(node), 0) end)
    |> Map.new(fn {name, node} ->
      [value] = Syntax.args(node, name, 1)
      {name, value}
    end)
  end

  def compatible!(schema, actual, expected, node) do
    unless compatible?(schema, actual, expected),
      do:
        Syntax.fail(node, :type_mismatch, "Expected #{inspect(expected)}, got #{inspect(actual)}")

    expected
  end

  def compatible?(_schema, type, type), do: true
  def compatible?(_schema, :any_actor, _), do: true
  def compatible?(_schema, _, :any_actor), do: true

  def compatible?(schema, {:union, actual}, expected),
    do: Enum.all?(actual, &compatible?(schema, &1, expected))

  def compatible?(schema, actual, {:union, expected}),
    do: Enum.any?(expected, &compatible?(schema, actual, &1))

  def compatible?(schema, {:optional, actual}, {:optional, expected}),
    do: compatible?(schema, actual, expected)

  def compatible?(_schema, {:list, :empty, 0, 0}, {:list, _, 0, _}), do: true

  def compatible?(schema, actual, {:optional, inner}),
    do: actual == :null or compatible?(schema, actual, inner)

  def compatible?(schema, {:literal_number, number}, expected) do
    match?({:ok, _}, Value.validate(schema, expected, number))
  end

  def compatible?(schema, {:enum_literal, value}, expected) do
    case Schema.resolve(schema, expected) do
      {:enum, values} -> value in values
      _ -> false
    end
  end

  def compatible?(schema, {:list, actual, amin, amax}, {:list, expected, emin, emax}) do
    amin >= emin and (emax == nil or (amax != nil and amax <= emax)) and
      compatible?(schema, actual, expected)
  end

  def compatible?(schema, actual, expected) do
    case {record_fields(schema, actual), record_fields(schema, expected)} do
      {actual, expected} when is_map(actual) and is_map(expected) ->
        Enum.all?(expected, fn {key, type} ->
          Map.has_key?(actual, key) and compatible?(schema, actual[key], type)
        end)

      _ ->
        resolved = Schema.resolve(schema, expected)
        if resolved != expected, do: compatible?(schema, actual, resolved), else: false
    end
  end

  def record_fields(_schema, {:shape, fields}), do: fields
  def record_fields(_schema, {:record, fields}), do: Map.new(fields, fn {k, v} -> {k, v.type} end)

  def record_fields(schema, {:named, name} = type) do
    case schema.entities[name] || schema.streams[name] do
      nil ->
        case Schema.resolve(schema, type) do
          {:record, fields} -> record_fields(schema, {:record, fields})
          _ -> nil
        end

      model ->
        Map.new(model.fields, fn {k, v} -> {k, v.type} end)
    end
  end

  def record_fields(_, _), do: nil

  def field!(context, type, field, node) do
    case type do
      {:optional, inner} ->
        field!(context, inner, field, node)

      {:union, types} ->
        {:union, Enum.map(types, &field!(context, &1, field, node)) |> Enum.uniq()}

      {:list, inner, min, max} ->
        {:list, field!(context, inner, field, node), min, max}

      _ ->
        fields = record_fields(context.schema, type)

        if fields,
          do: fields[field] || Syntax.fail(node, :unknown_field, "Unknown field #{field}"),
          else: image_field(context, type, field, node)
    end
  end

  defp image_field(context, type, field, node) do
    case Schema.resolve(context.schema, type) do
      {:image, limits} when field in ["width", "height"] ->
        {:number, "integer", Decimal.new(1),
         Decimal.new(limits[if(field == "width", do: "maxWidth", else: "maxHeight")]), 0}

      _ ->
        Syntax.fail(node, :unknown_field, "Cannot read field #{field} on #{inspect(type)}")
    end
  end

  defp symbol(node, value, context) do
    case value do
      "true" ->
        {:named, "Bool"}

      "false" ->
        {:named, "Bool"}

      "null" ->
        :null

      _ ->
        [root | fields] = String.split(value, ".")

        type =
          case Map.fetch(context.env, root) do
            {:ok, type} ->
              type

            :error ->
              case context.bindings[root] do
                nil ->
                  if (Map.get(context, :kind) == "policy" and
                        value in ~w(READ USE CREATE UPDATE DELETE INVOKE)) or
                       Enum.any?(context.schema.types, fn {_, type} ->
                         match?({:enum, _}, type) and value in elem(type, 1)
                       end),
                     do: {:enum_literal, value},
                     else: Syntax.fail(node, :unknown_binding, "Unknown binding #{root}")

                expression ->
                  if root in context.seen,
                    do: Syntax.fail(node, :cyclic_binding, "Cyclic binding #{root}")

                  infer(expression, %{context | seen: [root | context.seen]})
              end
          end

        Enum.reduce(fields, type, &field!(context, &2, &1, node))
    end
  end

  defp form(node, context) do
    {operator, args} = Syntax.form(node)
    schema = context.schema
    infer = &infer(&1, context)

    case {operator, args} do
      {"can", [action, target]} ->
        policy!(context, node)
        action!(action)
        reference!(schema, infer.(target), target)
        {:named, "Bool"}

      {"canAs", [actor, action, target]} ->
        policy!(context, node)
        action!(action)
        reference!(schema, infer.(actor), actor)
        reference!(schema, infer.(target), target)
        {:named, "Bool"}

      {"only", [changed | fields]} when fields != [] ->
        policy!(context, node)

        unless Syntax.symbol(changed) == "changed" and Map.has_key?(context.env, "changed"),
          do: Syntax.fail(node, :invalid_expression, "only requires change state")

        target = Map.fetch!(context.env, "target")
        Enum.each(fields, &field!(context, target, Syntax.identifier(&1), &1))
        {:named, "Bool"}

      {"creates", [transaction, entity]} ->
        policy!(context, node)

        unless Syntax.symbol(transaction) == "transaction" and
                 Map.has_key?(context.env, "transaction"),
               do: Syntax.fail(node, :invalid_expression, "creates requires transaction")

        entity = Syntax.identifier(entity)

        unless schema.entities[entity] || schema.streams[entity],
          do: Syntax.fail(node, :unknown_type, "Unknown created entity")

        {:list, {:named, entity}, 0, nil}

      {"updates", [transaction, reference, before, after_state]} ->
        policy!(context, node)

        unless Syntax.symbol(transaction) == "transaction" and
                 Map.has_key?(context.env, "transaction"),
               do: Syntax.fail(node, :invalid_expression, "updates requires transaction")

        type = infer.(reference)
        reference!(schema, type, reference)

        for {pattern, name} <- [{before, "before"}, {after_state, "after"}] do
          {^name, fields} = Syntax.form(pattern)

          fields
          |> bindings()
          |> Enum.each(fn {field, value} ->
            compatible!(schema, infer.(value), field!(context, type, field, pattern), value)
          end)
        end

        {:named, "Bool"}

      {"record", fields} ->
        fields = bindings(fields)
        {:shape, Map.new(fields, fn {name, value} -> {name, infer.(value)} end)}

      {"list", []} ->
        {:list, :empty, 0, 0}

      {"list", values} ->
        [first | rest] = Enum.map(values, infer)
        Enum.each(rest, &compatible!(schema, &1, first, node))
        {:list, first, length(values), length(values)}

      {"entity", [id]} ->
        case Schema.resolve(schema, infer.(id)) do
          {:id, entity} ->
            unless schema.entities[entity] || schema.streams[entity],
              do: Syntax.fail(id, :unknown_type, "ID is not a stored entity")

            {:named, entity}

          _ ->
            Syntax.fail(id, :type_mismatch, "entity requires a branded ID")
        end

      {"entities", [name]} ->
        entity = Syntax.identifier(name)

        unless schema.entities[entity] || schema.streams[entity],
          do: Syntax.fail(name, :unknown_type, "Unknown entity")

        {:list, {:named, entity}, 0, nil}

      {"new", [name]} ->
        type = Types.compile(name, context.names)

        unless match?({:id, _}, Schema.resolve(schema, type)),
          do: Syntax.fail(name, :type_mismatch, "new requires an ID type")

        type

      {"as", [name, value]} ->
        type = Types.compile(name, context.names)
        actual = infer.(value)

        unless numeric?(schema, actual) and numeric?(schema, type),
          do: Syntax.fail(node, :type_mismatch, "as requires numeric types")

        type

      {op, [a, b]} when op in ~w(add sub mul div) ->
        unless numeric?(schema, infer.(a)) and numeric?(schema, infer.(b)),
          do: Syntax.fail(node, :type_mismatch, "Arithmetic requires numbers")

        :numeric

      {op, [a, b]} when op in ~w(eq ne gt ge lt le) ->
        actual = infer.(a)
        expected = infer.(b)

        if op in ~w(gt ge lt le) and
             not (comparable?(schema, actual) and comparable?(schema, expected)),
           do:
             Syntax.fail(
               node,
               :type_mismatch,
               "Ordering requires numbers, datetimes, strings or branded IDs"
             )

        unless compatible?(schema, actual, expected) or compatible?(schema, expected, actual) or
                 (numeric?(schema, actual) and numeric?(schema, expected)),
               do: Syntax.fail(node, :type_mismatch, "Comparison operands are incompatible")

        {:named, "Bool"}

      {op, values} when op in ~w(and or) ->
        Enum.each(values, &compatible!(schema, infer.(&1), {:named, "Bool"}, &1))
        {:named, "Bool"}

      {"not", [value]} ->
        compatible!(schema, infer.(value), {:named, "Bool"}, node)

      {"concat", values} ->
        Enum.each(values, fn value ->
          type = infer.(value)

          unless type == {:named, "String"} or match?({:id, _}, Schema.resolve(schema, type)),
            do: Syntax.fail(value, :type_mismatch, "concat requires strings or IDs")
        end)

        {:named, "String"}

      {"contains", [collection, value]} ->
        {inner, _, _} = list!(infer.(collection), collection)
        compatible!(schema, infer.(value), inner, value)
        {:named, "Bool"}

      {"count", [collection]} ->
        list!(infer.(collection), collection)
        :numeric

      {op, [collection, variable, expression]} when op in ~w(any all where flatMap sum) ->
        {inner, min, max} = list!(infer.(collection), collection)
        local = %{context | env: Map.put(context.env, Syntax.identifier(variable), inner)}
        result = infer(expression, local)

        case op do
          op when op in ~w(any all where) ->
            compatible!(schema, result, {:named, "Bool"}, expression)
            if op == "where", do: {:list, inner, 0, max}, else: {:named, "Bool"}

          "sum" ->
            unless numeric?(schema, result),
              do: Syntax.fail(expression, :type_mismatch, "sum requires a numeric expression")

            :numeric

          "flatMap" ->
            {result, rmin, rmax} = list!(result, expression)
            {:list, result, min * rmin, if(max && rmax, do: max * rmax)}
        end

      {"map", [collection, variable | values]} when values != [] ->
        {inner, min, max} = list!(infer.(collection), collection)
        variable = Syntax.identifier(variable)
        bindings = bindings(values)

        local = %{
          context
          | bindings: Map.merge(context.bindings, bindings),
            env: Map.put(context.env, variable, inner),
            seen: []
        }

        result =
          Map.new(bindings, fn {name, expression} ->
            {name, infer(expression, %{local | seen: [name]})}
          end)

        {:list, {:shape, result}, min, max}

      {"groupSum", [collection, key, quantity, brand]} ->
        {inner, collection_min, max} = list!(infer.(collection), collection)
        key = Syntax.identifier(key)
        quantity = Syntax.identifier(quantity)
        key_type = field!(context, inner, key, node)

        unless numeric?(schema, field!(context, inner, quantity, node)),
          do: Syntax.fail(node, :type_mismatch, "groupSum requires numeric quantities")

        brand = Types.compile(brand, context.names)

        unless numeric?(schema, brand),
          do: Syntax.fail(node, :type_mismatch, "groupSum requires numeric output brand")

        {:list, {:shape, %{key => key_type, quantity => brand}},
         if(collection_min > 0, do: 1, else: 0), max}

      {"last", [collection, count]} ->
        {inner, _, max} = list!(infer.(collection), collection)
        n = constant_integer!(count)
        {:list, inner, 0, if(max, do: min(max, n), else: n)}

      {"single", [collection]} ->
        {inner, _, max} = list!(infer.(collection), collection)

        unless max == 1,
          do: Syntax.fail(collection, :type_mismatch, "single requires a list bounded by 1")

        {:optional, inner}

      {"order", [collection, path, direction]} ->
        {inner, _, _} = list!(infer.(collection), collection)
        [entity, field] = String.split(Syntax.symbol(path), ".")

        unless inner == {:named, entity},
          do: Syntax.fail(path, :type_mismatch, "Sort key must belong to list element")

        field!(context, inner, field, path)

        unless Syntax.symbol(direction) in ~w(ASC DESC),
          do: Syntax.fail(direction, :invalid_order, "Expected ASC or DESC")

        infer.(collection)

      {"ago", [duration]} ->
        duration!(duration)
        {:named, "DateTime"}

      {"since", [collection, time]} ->
        {inner, _, _} = list!(infer.(collection), collection)

        compatible!(
          schema,
          field!(context, inner, "receivedAt", node),
          {:named, "DateTime"},
          node
        )

        compatible!(schema, infer.(time), {:named, "DateTime"}, time)
        infer.(collection)

      {"live", [collection]} ->
        {inner, _, _} = list!(infer.(collection), collection)
        {:live, inner}

      {op, [name, value]} when op in ~w(create publish) ->
        effect!(context, node)
        name = Syntax.identifier(name)
        model = if op == "publish", do: schema.streams[name], else: schema.entities[name]
        unless model, do: Syntax.fail(node, :unknown_type, "Unknown #{op} target")

        stored =
          Map.reject(model.fields, fn {_, f} ->
            f.options["inverse"] || f.options["stream"] || f.options["receivedAt"] ||
              f.options["generated"]
          end)

        compatible!(schema, infer.(value), {:record, stored}, value)
        reject_extra_fields!(schema, infer.(value), stored, value)
        {:named, name}

      {"set", [path, value]} ->
        effect!(context, node)
        type = infer.(path)
        compatible!(schema, infer.(value), type, value)
        type

      {"delete", [target]} ->
        effect!(context, node)

        case infer.(target) do
          {:named, name} ->
            unless schema.entities[name],
              do: Syntax.fail(node, :type_mismatch, "delete requires an entity")

          _ ->
            Syntax.fail(node, :type_mismatch, "delete requires an entity")
        end

        {:named, "Bool"}

      {op, [method, value | options]} when op in ~w(invoke enqueue) ->
        effect!(context, node)

        contract =
          context.plugins[Syntax.symbol(method)] ||
            Syntax.fail(method, :unknown_plugin, "Unknown plugin method")

        compatible!(
          schema,
          infer.(value),
          {:shape, Map.new(contract.inputs, fn {k, v} -> {k, v.type} end)},
          value
        )

        reject_extra_fields!(schema, infer.(value), contract.inputs, value)

        if op == "invoke" and contract.mode == :external,
          do: Syntax.fail(node, :unsafe_effect, "External plugins require enqueue")

        if op == "enqueue",
          do: retry!(options, node),
          else: if(options != [], do: Syntax.fail(node, :arity, "invoke takes two arguments"))

        if op == "enqueue" do
          unless match?({:record, _}, schema.types["QueueReceipt"]),
            do: Syntax.fail(node, :unknown_type, "enqueue requires QueueReceipt record type")

          {:named, "QueueReceipt"}
        else
          contract.output
        end

      _ ->
        Syntax.fail(node, :invalid_expression, "Unknown expression or invalid arity #{operator}")
    end
  end

  def duration!(node) do
    case Regex.run(~r/\A([1-9][0-9]*)(ms|s|m|h|d)\z/, Syntax.string(node)) do
      [_, count, unit] ->
        String.to_integer(count) *
          Map.fetch!(
            %{"ms" => 1, "s" => 1000, "m" => 60_000, "h" => 3_600_000, "d" => 86_400_000},
            unit
          )

      _ ->
        Syntax.fail(node, :invalid_duration, "Invalid duration")
    end
  end

  defp retry!([retry], node) do
    {"retry", values} = Syntax.form(retry)
    options = Syntax.options(values, ~w(attempts delay exhausted))
    [attempts] = Syntax.args(options["attempts"] || node, "attempts", 1)
    constant_integer!(attempts)
    [delay] = Syntax.args(options["delay"] || node, "delay", 1)
    duration!(delay)
    [exhausted] = Syntax.args(options["exhausted"] || node, "exhausted", 1)

    unless Syntax.symbol(exhausted) == "RETAIN",
      do: Syntax.fail(exhausted, :invalid_retry, "Expected RETAIN")
  end

  defp retry!(_, node), do: Syntax.fail(node, :invalid_retry, "enqueue requires a retry policy")

  defp constant_integer!(node) do
    number = Types.constant(node)

    unless Decimal.equal?(number, Decimal.round(number, 0)) and Decimal.compare(number, 0) != :lt and
             Decimal.compare(number, 100_000) != :gt,
           do: Syntax.fail(node, :invalid_bound, "Expected integer from 0 to 100000")

    Decimal.to_integer(number)
  end

  defp effect!(context, node),
    do:
      if(context.kind in ["query", "policy"],
        do: Syntax.fail(node, :query_effect, "Queries cannot mutate data or invoke effects")
      )

  defp policy!(context, node),
    do:
      unless(context.kind == "policy",
        do:
          Syntax.fail(
            node,
            :invalid_expression,
            "Permission operators are only valid in policies"
          )
      )

  defp action!(node),
    do:
      unless(Syntax.symbol(node) in ~w(READ USE CREATE UPDATE DELETE INVOKE),
        do: Syntax.fail(node, :unknown_action, "Unknown permission action")
      )

  defp reference!(schema, {:optional, type}, node), do: reference!(schema, type, node)

  defp reference!(schema, {:union, types}, node),
    do: Enum.each(types, &reference!(schema, &1, node))

  defp reference!(schema, {:named, name}, node) do
    unless schema.entities[name] || schema.streams[name] ||
             match?({:record, _}, schema.types[name]),
           do: Syntax.fail(node, :type_mismatch, "Expected a protected reference")
  end

  defp reference!(_, _, node),
    do: Syntax.fail(node, :type_mismatch, "Expected a protected reference")

  defp numeric?(_, :numeric), do: true
  defp numeric?(_, {:literal_number, _}), do: true
  defp numeric?(schema, type), do: match?({:number, _, _, _, _}, Schema.resolve(schema, type))

  defp comparable?(schema, type),
    do:
      numeric?(schema, type) or
        Schema.resolve(schema, type) in [{:named, "String"}, {:named, "DateTime"}] or
        match?({:id, _}, Schema.resolve(schema, type))

  defp list!({:list, inner, min, max}, _), do: {inner, min, max}
  defp list!(_, node), do: Syntax.fail(node, :type_mismatch, "Expected list")

  defp reject_extra_fields!(schema, actual, expected, node) do
    actual = record_fields(schema, actual) || Syntax.fail(node, :type_mismatch, "Expected record")

    if Map.keys(actual) -- Map.keys(expected) != [],
      do: Syntax.fail(node, :unknown_field, "Unknown record field")
  end
end
