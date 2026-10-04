defmodule OrderLab.Language.Compiler do
  alias OrderLab.Language.{Parser, Types, Checker, Error}

  def compile!(source, file \\ "<source>", operations \\ %{}) do
    program = Parser.parse!(source, file)

    definitions =
      Map.new(program.types, fn {name, type} -> {name, Types.resolve!(type, program.types)} end)

    Enum.each(definitions, fn {_name, type} -> check_type(type) end)

    tables =
      Map.new(program.tables, fn {name, type} -> {name, Types.resolve!(type, definitions)} end)

    Enum.each(tables, fn {name, type} ->
      case Checker.base(type) do
        {:record, fields} -> Checker.expect!(Map.get(fields, "id", :null), {:named, "String"})
        _ -> fail("Table #{name} must be a record with String id")
      end
    end)

    mqtt =
      Map.new(program.mqtt, fn {name, source} ->
        params = Enum.map(source.params, &%{&1 | type: Types.resolve!(&1.type, definitions)})

        Enum.each(params, fn param ->
          Checker.expect!(Checker.base(param.type), {:named, "String"})

          if param.name in ["message", "request"],
            do: fail("Reserved MQTT parameter name #{param.name}")
        end)

        payload = Types.resolve!(source.payload, definitions)

        fields =
          case Checker.base(payload) do
            {:record, fields} -> fields
            _ -> fail("MQTT payload must be a record")
          end

        if Map.has_key?(fields, "received_at"), do: fail("received_at is reserved MQTT metadata")

        placeholders =
          Regex.scan(~r/\{([A-Za-z_][A-Za-z_0-9]*)\}/, source.topic) |> Enum.map(&List.last/1)

        unless Enum.sort(placeholders) == Enum.sort(Enum.map(params, & &1.name)),
          do: fail("MQTT topic placeholders must match PARAM declarations exactly")

        Enum.each(String.split(source.topic, "/"), fn part ->
          if String.contains?(part, ["{", "}"]) and
               not Regex.match?(~r/^\{[A-Za-z_][A-Za-z_0-9]*\}$/, part),
             do: fail("Topic parameters must occupy an entire topic level")
        end)

        if String.contains?(source.topic, ["+", "#", "\0"]),
          do: fail("MQTT topic templates cannot contain wildcards")

        {name,
         Map.merge(source, %{
           params: params,
           payload: payload,
           row_type: {:record, Map.put(fields, "received_at", {:named, "String"})}
         })}
      end)

    sources = Map.values(mqtt)

    Enum.with_index(sources)
    |> Enum.each(fn {a, index} ->
      Enum.drop(sources, index + 1)
      |> Enum.each(fn b ->
        left = String.split(a.topic, "/")
        right = String.split(b.topic, "/")

        if length(left) == length(right) and
             Enum.zip(left, right)
             |> Enum.all?(fn {x, y} ->
               x == y or String.starts_with?(x, "{") or String.starts_with?(y, "{")
             end),
           do: fail("Ambiguous MQTT topic templates #{a.name} and #{b.name}")
      end)
    end)

    env =
      Map.new(tables, fn {name, type} -> {name, {:list, type}} end)
      |> Map.put("$sources", mqtt)
      |> Map.put(
        "request",
        {:record, %{"id" => {:named, "String"}, "time" => {:named, "String"}}}
      )

    endpoints =
      Enum.map(program.endpoints, fn endpoint ->
        inputs =
          if endpoint.method == "MQTT" do
            source = Map.get(mqtt, endpoint.path) || fail("Unknown MQTT source #{endpoint.path}")

            Enum.map(source.params, &Map.merge(&1, %{default: :missing, line: endpoint.line})) ++
              [%{name: "message", type: source.payload, default: :missing, line: endpoint.line}]
          else
            Enum.map(endpoint.inputs, &%{&1 | type: Types.resolve!(&1.type, definitions)})
          end

        if endpoint.method != "MQTT" do
          unless Regex.match?(
                   ~r{^/(?:[A-Za-z0-9_.-]+|:[A-Za-z_][A-Za-z_0-9]*)(?:/(?:[A-Za-z0-9_.-]+|:[A-Za-z_][A-Za-z_0-9]*))*$},
                   endpoint.path
                 ) or endpoint.path == "/", do: fail("Invalid HTTP path #{endpoint.path}")

          parameters =
            Regex.scan(~r/:([A-Za-z_][A-Za-z_0-9]*)/, endpoint.path) |> Enum.map(&List.last/1)

          if length(parameters) != length(Enum.uniq(parameters)),
            do: fail("Duplicate HTTP path parameter")

          unless parameters -- Enum.map(inputs, & &1.name) == [],
            do: fail("HTTP path parameters require INPUT declarations")
        end

        endpoint = %{endpoint | body: resolve_annotations(endpoint.body, definitions)}

        local =
          Enum.reduce(inputs, env, fn input, env ->
            if Map.has_key?(env, input.name),
              do: fail("Input shadows existing name #{input.name}")

            if input.default != :missing,
              do: Checker.expect_expr!(input.default, input.type, env)

            Map.put(env, input.name, input.type)
          end)

        state = %{
          env: local,
          returns: [],
          transaction: false,
          committed: false,
          loop: false,
          catching: false,
          tables: tables,
          operations: operations
        }

        check_block(endpoint.body, state)

        unless returns?(endpoint.body),
          do: fail("#{endpoint.method} #{endpoint.path} requires a response on every path")

        Map.merge(endpoint, %{inputs: inputs, source_names: Map.keys(mqtt)})
      end)

    signatures = Enum.map(endpoints, &{&1.method, canonical_route(&1.path)})
    unless length(signatures) == length(Enum.uniq(signatures)), do: fail("Duplicate HTTP route")
    %{program | types: definitions, tables: tables, mqtt: mqtt, endpoints: endpoints}
  end

  defp check_block(nodes, state), do: Enum.reduce(nodes, state, &check/2)

  defp check(node, state) do
    try do
      result = check_node(node, state)

      if Map.has_key?(node, :annotation) do
        name = if node.kind == :let, do: node.name, else: node.binding
        actual = Map.fetch!(result.env, name)
        Checker.expect!(actual, node.annotation)
        if node.kind == :let, do: Checker.expect_expr!(node.value, node.annotation, state.env)
        %{result | env: Map.put(result.env, name, node.annotation)}
      else
        result
      end
    rescue
      error in Error -> reraise %{error | file: node.file, line: node.line}, __STACKTRACE__
    end
  end

  defp check_node(%{kind: :let} = n, s), do: bind(s, n.name, Checker.infer(n.value, s.env))

  defp check_node(%{kind: :require} = n, s) do
    failure_spec!(n, s)
    Checker.expect!(Checker.infer(n.condition, s.env), {:named, "Bool"})
    %{s | env: Checker.narrow(n.condition, s.env)}
  end

  defp check_node(%{kind: :fail} = n, s),
    do:
      (
        failure_spec!(n, s)
        s
      )

  defp check_node(%{kind: :return} = n, s) do
    unless is_integer(n.status) and n.status in 200..599,
      do: fail("Response status must be 200..599")

    if s.transaction and not s.committed and not s.loop,
      do: fail("Response cannot leave an uncommitted transaction")

    %{s | returns: s.returns ++ [Checker.infer(n.value, s.env)]}
  end

  defp check_node(%{kind: :when} = n, s) do
    Checker.expect!(Checker.infer(n.condition, s.env), {:named, "Bool"})
    left = check_block(n.body, %{s | env: Checker.narrow(n.condition, s.env)})
    right = check_block(n.otherwise, %{s | env: Checker.narrow_false(n.condition, s.env)})

    unless left.committed == right.committed,
      do: fail("Conditional COMMIT must occur on both paths")

    %{s | committed: left.committed, returns: Enum.uniq(left.returns ++ right.returns)}
  end

  defp check_node(%{kind: :each} = n, s) do
    type = Checker.element!(Checker.infer(n.source, s.env))
    child = check_block(n.body, %{s | env: Map.put(s.env, n.name, type), loop: true, returns: []})

    if n.binding do
      unless returns?(n.body), do: fail("Bound iteration requires RETURN on every path")
      results = child.returns

      type =
        if results == [],
          do: {:named, "JSON"},
          else: Enum.reduce(tl(results), hd(results), &Checker.common!/2)

      bind(s, n.binding, {:list, type})
    else
      s
    end
  end

  defp check_node(%{kind: :transaction} = n, s) do
    if s.transaction or s.loop, do: fail("Nested or iterated TRANSACTION is unsupported")
    result = check_block(n.body, %{s | transaction: true, committed: false})
    unless result.committed, do: fail("TRANSACTION requires COMMIT")
    %{result | transaction: false, committed: false}
  end

  defp check_node(%{kind: :commit}, s) do
    unless s.transaction and not s.committed and not s.loop and not s.catching,
      do: fail("Invalid COMMIT boundary")

    %{s | committed: true}
  end

  defp check_node(%{kind: :try} = n, s) do
    result = check_block(n.body, %{s | catching: true})

    error =
      {:record,
       %{
         "code" => {:named, "String"},
         "message" => {:named, "String"},
         "status" => {:named, "Int"},
         "details" => {:named, "JSON"}
       }}

    handler =
      check_block(n.handler, %{s | catching: true, env: Map.put(s.env, n.error_name, error)})

    # Success-only bindings cannot escape a handler that continues.
    if returns?(n.handler),
      do: %{result | catching: s.catching, returns: Enum.uniq(result.returns ++ handler.returns)},
      else: %{s | returns: Enum.uniq(result.returns ++ handler.returns)}
  end

  defp check_node(%{kind: kind} = n, s) when kind in [:call, :queue] do
    effect!(s, kind == :queue)
    op = Map.get(s.operations, n.operation) || fail("Unknown registered operation #{n.operation}")
    Checker.expect_expr!(n.input, op.input, s.env)

    if kind == :queue do
      unless is_integer(n.attempts) and n.attempts in 1..100 and is_integer(n.delay) and
               n.delay in 0..86_400_000,
             do: fail("Invalid queue policy")

      bind(s, n.binding, {:record, %{"id" => {:named, "String"}, "state" => {:named, "String"}}})
    else
      bind(s, n.binding, op.output)
    end
  end

  defp check_node(%{kind: :insert} = n, s) do
    effect!(s, true)
    type = Map.get(s.tables, n.table) || fail("Unknown table #{n.table}")
    Checker.expect_expr!(n.value, type, s.env)
    bind(s, n.binding, type)
  end

  defp check_node(%{kind: kind} = n, s) when kind in [:update, :delete] do
    effect!(s, true)
    type = Map.get(s.tables, n.table) || fail("Unknown table #{n.table}")
    local = Map.put(s.env, n.alias, type)
    Checker.expect!(Checker.infer(n.condition, local), {:named, "Bool"})

    if kind == :update do
      {:record, fields} = Checker.base(type)
      {:record, changed} = Checker.base(Checker.infer(n.value, local))
      if Map.has_key?(changed, "id"), do: fail("UPDATE cannot change id")

      Enum.each(changed, fn {key, value} ->
        expected = Map.get(fields, key) || fail("Unknown field #{key}")
        Checker.expect!(value, expected)

        case n.value do
          {:record, values} ->
            Checker.expect_expr!(
              Enum.find_value(values, fn {name, expr} -> if name == key, do: expr end),
              expected,
              local
            )

          _ ->
            :ok
        end
      end)
    end

    bind(s, n.binding, {:list, type})
  end

  defp bind(s, nil, _), do: s

  defp bind(s, name, type) do
    if Map.has_key?(s.env, name), do: fail("Duplicate binding #{name}")
    %{s | env: Map.put(s.env, name, type)}
  end

  defp effect!(s, write) do
    if s.committed or (write and not s.transaction),
      do:
        fail("Writes and queues require an uncommitted transaction; effects cannot follow COMMIT")
  end

  defp failure_spec!(n, s) do
    unless is_integer(n.status) and n.status in 400..599,
      do: fail("Failure status must be 400..599")

    Checker.expect!(Checker.infer(n.message, s.env), {:named, "String"})
  end

  defp returns?(nodes),
    do:
      Enum.any?(nodes, fn
        %{kind: kind} when kind in [:return, :fail] -> true
        %{kind: :when, body: a, otherwise: b} -> returns?(a) and returns?(b)
        %{kind: :transaction, body: body} -> returns?(body)
        %{kind: :try, body: a, handler: b} -> returns?(a) and returns?(b)
        _ -> false
      end)

  defp resolve_annotations(nodes, definitions) do
    Enum.map(nodes, fn node ->
      node =
        if Map.has_key?(node, :annotation),
          do: %{node | annotation: Types.resolve!(node.annotation, definitions)},
          else: node

      Enum.reduce([:body, :otherwise, :handler], node, fn key, node ->
        if Map.has_key?(node, key),
          do: Map.update!(node, key, &resolve_annotations(&1, definitions)),
          else: node
      end)
    end)
  end

  defp check_type({:refined, type, predicate}) do
    check_type(type)
    Checker.pure_refinement!(predicate)
    Checker.expect!(Checker.infer(predicate, %{"value" => type}), {:named, "Bool"})
  end

  defp check_type({:list, type}), do: check_type(type)
  defp check_type({:optional, type}), do: check_type(type)
  defp check_type({:record, fields}), do: Enum.each(fields, fn {_, type} -> check_type(type) end)
  defp check_type(_), do: :ok
  defp canonical_route(path), do: Regex.replace(~r/:[A-Za-z_][A-Za-z_0-9]*/, path, ":param")
  defp fail(message), do: Error.fail!(message, stage: :compile)
end
