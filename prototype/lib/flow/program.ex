defmodule Flow.Program do
  @moduledoc "Compiled declarations and checked operation contracts."
  alias Flow.{Schema, Permissions, Auth, Checker, Syntax, Types, Value, Literal}
  defstruct [:schema, :permissions, :auth, operations: %{}, plugins: %{}, seeds: []]
  @metadata ~w(input output http atomic on when websocket result)

  def compile(source, options \\ []) do
    with {:ok, schema} <- Schema.compile(source),
         {:ok, permissions} <- Permissions.compile(schema),
         {:ok, auth} <- Auth.compile(schema, Keyword.get(options, :auth, %{})) do
      try do
        names =
          MapSet.new(
            Map.keys(schema.types) ++ Map.keys(schema.entities) ++ Map.keys(schema.streams)
          )

        plugins = plugins!(schema, names)
        Flow.PolicyChecker.validate!(schema, permissions, plugins)

        operations =
          Map.new(schema.operations, fn {name, node} ->
            {name, operation!(schema, names, plugins, node)}
          end)

        check_routes!(operations)
        streams!(schema)
        seeds = seeds!(schema)

        {:ok,
         %__MODULE__{
           schema: schema,
           permissions: permissions,
           auth: auth,
           operations: operations,
           plugins: plugins,
           seeds: seeds
         }}
      rescue
        error in Flow.ValidationError ->
          {:error, error}

        error in [MatchError, CaseClauseError, KeyError] ->
          {:error,
           %Flow.ValidationError{
             code: :invalid_declaration,
             message: "Malformed declaration: #{Exception.message(error)}"
           }}
      end
    end
  end

  def compile!(source, options \\ []) do
    case compile(source, options) do
      {:ok, program} -> program
      {:error, error} -> raise error
    end
  end

  defp plugins!(schema, names) do
    Enum.reduce(schema.plugins, %{}, fn {name, node}, contracts ->
      {"plugin", [_ | methods]} = Syntax.form(node)
      methods = Syntax.unique(methods, &elem(Syntax.form(&1), 0))

      Enum.reduce(methods, contracts, fn {method, node}, contracts ->
        {_, components} = Syntax.form(node)
        {inputs, rest} = Enum.split_with(components, &(elem(Syntax.form(&1), 0) == "input"))
        inputs = inputs!(inputs, names, schema)
        opts = Syntax.options(rest, ~w(output pure transactional release))
        [output] = Syntax.args(opts["output"] || node, "output", 1)
        for key <- ~w(pure transactional release), opt = opts[key], do: Syntax.args(opt, key, 0)

        if opts["pure"] && opts["transactional"],
          do: Syntax.fail(node, :invalid_plugin, "Plugin cannot be both pure and transactional")

        mode =
          cond do
            opts["pure"] -> :pure
            opts["transactional"] -> :transactional
            true -> :external
          end

        Map.put(contracts, name <> "." <> method, %{
          inputs: inputs,
          output: Types.compile(output, names),
          mode: mode,
          release: not is_nil(opts["release"]),
          node: node
        })
      end)
    end)
  end

  defp inputs!(nodes, names, schema) do
    nodes
    |> Syntax.unique(fn node ->
      case Syntax.form(node) do
        {"input", [name, _ | _]} -> Syntax.identifier(name)
        _ -> Syntax.fail(node, :invalid_input, "Input requires a name and type")
      end
    end)
    |> Map.new(fn {name, node} ->
      {_, [_, type | rest]} = Syntax.form(node)
      type = Types.compile(type, names)
      opts = Syntax.options(rest, ["default"])

      default =
        case opts["default"] do
          nil ->
            :required

          option ->
            [value] = Syntax.args(option, "default", 1)
            {:default, Value.validate!(schema, type, Literal.decode(value))}
        end

      {name, %{type: type, default: default}}
    end)
  end

  defp operation!(schema, names, plugins, node) do
    {kind, [name | components]} = Syntax.form(node)
    name = Syntax.identifier(name)
    {inputs, components} = Enum.split_with(components, &(elem(Syntax.form(&1), 0) == "input"))
    inputs = inputs!(inputs, names, schema)
    {meta, bindings} = Enum.split_with(components, &(elem(Syntax.form(&1), 0) in @metadata))
    meta = Syntax.options(meta, @metadata -- ["input"])
    [output] = Syntax.args(meta["output"] || node, "output", 1)
    output = Types.compile(output, names)
    [result] = Syntax.args(meta["result"] || node, "result", 1)
    if meta["atomic"], do: Syntax.args(meta["atomic"], "atomic", 0)

    if kind == "mutate" and is_nil(meta["atomic"]),
      do: Syntax.fail(node, :missing_atomic, "Mutations require atomic")

    if kind == "query" and meta["atomic"],
      do: Syntax.fail(node, :query_effect, "Queries cannot declare atomic")

    bindings = Checker.bindings(bindings)

    if Enum.any?(Map.keys(bindings), &(&1 in ~w(request event actor context))),
      do: Syntax.fail(node, :reserved_name, "Reserved binding name")

    env = Map.new(inputs, fn {name, input} -> {"$" <> name, input.type} end)

    env =
      Map.put(
        env,
        "request",
        {:shape, %{"id" => {:named, "RequestID"}, "time" => {:named, "DateTime"}}}
      )

    env =
      if meta["on"] do
        {stream, _actor} = event!(schema, meta["on"])
        Map.put(env, "event", {:named, stream})
      else
        env
      end

    context = %{
      schema: schema,
      names: names,
      plugins: plugins,
      kind: kind,
      env: env,
      bindings: bindings,
      seen: []
    }

    Enum.each(bindings, fn {name, value} -> Checker.infer(value, %{context | seen: [name]}) end)
    actual = Checker.infer(result, context)

    actual =
      if meta["websocket"] do
        case actual do
          {:live, inner} -> inner
          _ -> Syntax.fail(result, :invalid_stream, "WebSocket query requires a live result")
        end
      else
        if match?({:live, _}, actual),
          do: Syntax.fail(result, :invalid_stream, "live requires websocket")

        actual
      end

    Checker.compatible!(schema, actual, output, result)

    if meta["when"] do
      [condition] = Syntax.args(meta["when"], "when", 1)
      Checker.compatible!(schema, Checker.infer(condition, context), {:named, "Bool"}, condition)
    end

    route = if meta["http"], do: route!(meta["http"], inputs)
    if meta["on"] && kind != "mutate", do: Syntax.fail(node, :invalid_event, "on requires mutate")

    if meta["websocket"] do
      case Syntax.form(meta["websocket"]) do
        {"websocket", [path, source]} ->
          Syntax.string(path)
          [stream] = Syntax.args(source, "source", 1)

          unless schema.streams[Syntax.symbol(stream)],
            do: Syntax.fail(source, :unknown_type, "Unknown stream")

        _ ->
          Syntax.fail(node, :invalid_stream, "websocket requires path and source")
      end
    end

    %{
      name: name,
      kind: kind,
      inputs: inputs,
      output: output,
      bindings: bindings,
      result: result,
      metadata: meta,
      route: route,
      node: node
    }
  end

  defp event!(schema, node) do
    case Syntax.form(node) do
      {"on", [stream, actor]} ->
        stream = Syntax.identifier(stream)

        unless schema.streams[stream],
          do: Syntax.fail(node, :unknown_type, "Unknown event stream")

        [service] = Syntax.args(actor, "actor", 1)
        [id] = Syntax.args(service, "service", 1)

        unless schema.entities["Service"],
          do: Syntax.fail(node, :invalid_event, "Service identity is not declared")

        {stream, Syntax.string(id)}

      _ ->
        Syntax.fail(node, :invalid_event, "on requires stream and actor")
    end
  end

  defp route!(node, inputs) do
    case Syntax.form(node) do
      {"http", [method, path | opts]} ->
        method = Syntax.symbol(method)
        path = Syntax.string(path)

        unless method in ~w(GET POST PUT PATCH DELETE) and String.starts_with?(path, "/"),
          do: Syntax.fail(node, :invalid_route, "Invalid HTTP route")

        for [_, name] <- Regex.scan(~r/\{([A-Za-z_][A-Za-z_0-9]*)\}/, path) do
          unless Map.has_key?(inputs, name),
            do: Syntax.fail(node, :invalid_route, "Unknown path input #{name}")
        end

        opts = Syntax.options(opts, ["status"])

        status =
          case opts["status"] do
            nil ->
              200

            option ->
              [value] = Syntax.args(option, "status", 1)
              n = Types.constant(value)

              unless Decimal.equal?(n, Decimal.round(n, 0)) and Decimal.compare(n, 200) != :lt and
                       Decimal.compare(n, 299) != :gt,
                     do: Syntax.fail(value, :invalid_route, "Success status must be 200..299")

              Decimal.to_integer(n)
          end

        %{method: method, path: path, status: status}

      _ ->
        Syntax.fail(node, :invalid_route, "http requires method and path")
    end
  end

  defp check_routes!(operations) do
    Enum.reduce(operations, MapSet.new(), fn {_, op}, seen ->
      if op.route do
        key = {op.route.method, Regex.replace(~r/\{[^}]+\}/, op.route.path, "{}")}

        if MapSet.member?(seen, key),
          do: Syntax.fail(op.node, :duplicate_route, "Duplicate HTTP route")

        MapSet.put(seen, key)
      else
        seen
      end
    end)
  end

  defp streams!(schema) do
    Enum.each(schema.streams, fn {_, model} ->
      [topic] = Syntax.args(model.options["mqtt"] || model.node, "mqtt", 1)
      Syntax.string(topic)
      {"history", opts} = Syntax.form(model.options["history"] || model.node)
      opts = Syntax.options(opts, ~w(duration maxMessages))
      [duration] = Syntax.args(opts["duration"] || model.node, "duration", 1)
      Checker.duration!(duration)
      [maximum] = Syntax.args(opts["maxMessages"] || model.node, "maxMessages", 1)
      n = Types.constant(maximum)

      unless Decimal.equal?(n, Decimal.round(n, 0)) and Decimal.compare(n, 1) != :lt and
               Decimal.compare(n, 100_000) != :gt,
             do: Syntax.fail(maximum, :invalid_stream, "maxMessages must be 1..100000")
    end)
  end

  defp seeds!(schema) do
    Enum.map(schema.seeds, fn node ->
      case Syntax.form(node) do
        {"seed", [entity, rows]} ->
          name = Syntax.identifier(entity)

          model =
            schema.entities[name] || Syntax.fail(entity, :unknown_type, "Seed requires an entity")

          {"rows", values} = Syntax.form(rows)

          type =
            {:record,
             Map.reject(model.fields, fn {_, f} -> f.options["inverse"] || f.options["stream"] end)}

          {name, Enum.map(values, &Value.validate!(schema, type, Literal.decode(&1)))}

        _ ->
          Syntax.fail(node, :invalid_seed, "seed requires an entity and rows")
      end
    end)
  end
end
