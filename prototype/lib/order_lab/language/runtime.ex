defmodule OrderLab.Language.Failure do
  defexception [:message, :code, :status, :details]
end

defmodule OrderLab.Language.Runtime do
  @moduledoc "Scenario interpreter. Host callbacks implement storage and registered effects."
  alias OrderLab.Language.{Evaluator, Types, Error, Failure}

  def run(endpoint, input, definitions, host, request \\ %{}) do
    unless is_map(input),
      do: raise(Failure, status: 422, code: "invalid_input", message: "Input must be an object")

    known = Enum.map(endpoint.inputs, & &1.name)
    unknown = Map.keys(input) -- known

    if unknown != [],
      do:
        raise(Failure,
          status: 422,
          code: "invalid_input",
          message: "Unknown input fields: #{Enum.join(unknown, ", ")}"
        )

    env =
      Enum.reduce(
        endpoint.inputs,
        %{"request" => request, "$source_names" => Map.get(endpoint, :source_names, [])},
        fn declaration, env ->
          value =
            case Map.fetch(input, declaration.name) do
              {:ok, value} ->
                value

              :error ->
                cond do
                  declaration.default != :missing ->
                    Evaluator.eval(declaration.default, env)

                  match?({:optional, _}, declaration.type) ->
                    nil

                  true ->
                    raise Failure,
                      status: 422,
                      code: "invalid_input",
                      message: "Missing input #{declaration.name}"
                end
            end

          value = Types.validate!(value, declaration.type, definitions, declaration.name)
          Map.put(env, declaration.name, value)
        end
      )

    context = %{
      env: env,
      host: host,
      transaction: false,
      committed: false,
      loop: false,
      continuation: []
    }

    host.(:checkpoint, %{
      checkpoint: Map.drop(%{context | continuation: [{:block, endpoint.body}]}, [:host])
    })

    finish(block(endpoint.body, context))
  end

  def resume(checkpoint, host) do
    ctx = Map.put(checkpoint, :host, host)
    finish(resume_frames(ctx.continuation, %{ctx | continuation: []}))
  end

  defp finish(result) do
    case result do
      {:return, value, status, _} ->
        {status, value}

      {:continue, _} ->
        raise Error, message: "Scenario completed without a response", stage: :runtime
    end
  end

  defp block([], ctx), do: {:continue, ctx}

  defp block([statement | rest], ctx) do
    result =
      try do
        execute(statement, %{ctx | continuation: [{:block, rest} | ctx.continuation]})
      rescue
        error in Error ->
          reraise %{error | file: statement.file, line: statement.line}, __STACKTRACE__
      end

    case result do
      {:continue, child} -> block(rest, %{child | continuation: ctx.continuation})
      {:return, _, _, _} = result -> result
    end
  end

  defp resume_frames([], ctx), do: {:continue, ctx}

  defp resume_frames([{:block, body} | frames], ctx) do
    case block(body, %{ctx | continuation: frames}) do
      {:continue, child} -> resume_frames(frames, child)
      result -> result
    end
  end

  defp resume_frames([:end_transaction | frames], ctx) do
    unless ctx.committed,
      do: raise(Error, message: "Invalid transaction checkpoint", stage: :runtime)

    resume_frames(frames, %{ctx | transaction: false, committed: false})
  end

  defp resume_frames([{:end_when, env} | frames], ctx),
    do: resume_frames(frames, %{ctx | env: env})

  defp execute(%{annotation: type} = node, ctx) do
    result = execute(Map.delete(node, :annotation), ctx)
    child = result_context(result)
    name = if node.kind == :let, do: node.name, else: node.binding

    value =
      try do
        Types.validate!(Map.fetch!(child.env, name), type, %{}, name)
      rescue
        e in Error ->
          raise Failure, status: 422, code: "type_constraint_failed", message: e.message
      end

    replace_context(result, bind(child, name, value))
  end

  defp execute(%{kind: :let} = node, ctx),
    do: continue(bind(ctx, node.name, evaluate(node.value, ctx)))

  defp execute(%{kind: :require} = node, ctx) do
    unless Evaluator.boolean!(evaluate(node.condition, ctx)), do: failure!(node, ctx)
    continue(ctx)
  end

  defp execute(%{kind: :fail} = node, ctx), do: failure!(node, ctx)

  defp execute(%{kind: :return} = node, ctx),
    do: {:return, evaluate(node.value, ctx), node.status, ctx}

  defp execute(%{kind: :when} = node, ctx) do
    branch =
      if Evaluator.boolean!(evaluate(node.condition, ctx)), do: node.body, else: node.otherwise

    case block(branch, %{ctx | continuation: [{:end_when, ctx.env} | ctx.continuation]}) do
      {:continue, child} -> continue(%{ctx | committed: child.committed})
      result -> result
    end
  end

  defp execute(%{kind: :each} = node, ctx) do
    rows = Evaluator.list!(evaluate(node.source, ctx))

    values =
      Enum.map(rows, fn row ->
        child = %{ctx | env: Map.put(ctx.env, node.name, row), loop: true}

        case block(node.body, child) do
          {:return, value, _, _} ->
            value

          {:continue, _} ->
            if node.binding,
              do:
                raise(Error,
                  message: "Bound iteration requires RETURN for every item",
                  stage: :runtime
                )

            nil
        end
      end)

    continue(bind(ctx, node.binding, values))
  end

  defp execute(%{kind: :transaction} = node, ctx) do
    if ctx.transaction,
      do:
        raise(Error,
          message: "Nested TRANSACTION is not supported; use TRY for a savepoint",
          stage: :runtime
        )

    if ctx.loop,
      do:
        raise(Error,
          message: "TRANSACTION cannot be started inside an iteration",
          stage: :runtime
        )

    ctx.host.(:begin, %{})

    try do
      result =
        block(node.body, %{
          ctx
          | transaction: true,
            committed: false,
            continuation: [:end_transaction | ctx.continuation]
        })

      child = result_context(result)

      unless child.committed,
        do:
          raise(Error,
            message: "Transaction requires COMMIT before leaving its block",
            stage: :runtime
          )

      replace_context(result, %{child | transaction: false, committed: false})
    rescue
      error ->
        ctx.host.(:rollback, %{})
        reraise error, __STACKTRACE__
    end
  end

  defp execute(%{kind: :commit}, ctx) do
    unless ctx.transaction and not ctx.committed and not ctx.loop,
      do:
        raise(Error,
          message: "COMMIT must occur once in a transaction, outside an iteration",
          stage: :runtime
        )

    ctx.host.(:commit, %{checkpoint: Map.drop(%{ctx | committed: true}, [:host])})
    continue(%{ctx | committed: true})
  end

  defp execute(%{kind: :try} = node, ctx) do
    savepoint = "flow_" <> Base.encode16(:crypto.strong_rand_bytes(8), case: :lower)
    if ctx.transaction, do: ctx.host.(:savepoint, %{name: savepoint})

    try do
      result = block(node.body, ctx)
      if ctx.transaction, do: ctx.host.(:release, %{name: savepoint})
      result
    rescue
      error in Failure ->
        if ctx.transaction do
          ctx.host.(:rollback_to, %{name: savepoint})
          ctx.host.(:release, %{name: savepoint})
        end

        details = %{
          "code" => error.code,
          "message" => error.message,
          "status" => error.status,
          "details" => error.details
        }

        case block(node.handler, %{ctx | env: Map.put(ctx.env, node.error_name, details)}) do
          {:continue, _} -> continue(ctx)
          result -> result
        end
    end
  end

  defp execute(%{kind: :call} = node, ctx) do
    effects_allowed!(ctx)
    operation = Map.fetch!(OrderLab.Language.Native.operations(), node.operation)
    if Map.get(operation, :effect) == :write, do: writes_allowed!(ctx)
    output = ctx.host.(:call, %{operation: node.operation, input: evaluate(node.input, ctx)})
    continue(bind(ctx, node.binding, output))
  end

  defp execute(%{kind: :publish} = node, ctx) do
    writes_allowed!(ctx)

    result =
      ctx.host.(:publish, %{
        source: node.source,
        parameters: Enum.map(node.parameters, &evaluate(&1, ctx)),
        value: evaluate(node.value, ctx),
        retain: Evaluator.boolean!(evaluate(node.retain, ctx))
      })

    continue(bind(ctx, node.binding, result))
  end

  defp execute(%{kind: :queue} = node, ctx) do
    writes_allowed!(ctx)

    output =
      ctx.host.(:queue, %{
        operation: node.operation,
        input: evaluate(node.input, ctx),
        attempts: node.attempts,
        delay: node.delay,
        disposition: node.disposition
      })

    continue(bind(ctx, node.binding, output))
  end

  defp execute(%{kind: :insert} = node, ctx) do
    writes_allowed!(ctx)
    output = ctx.host.(:insert, %{table: node.table, value: evaluate(node.value, ctx)})
    continue(bind(ctx, node.binding, output))
  end

  defp execute(%{kind: kind} = node, ctx) when kind in [:update, :delete] do
    writes_allowed!(ctx)
    rows = ctx.host.(:source, %{name: node.table})

    selected =
      Enum.filter(rows, fn row ->
        Evaluator.boolean!(
          evaluate(node.condition, %{ctx | env: Map.put(ctx.env, node.alias, row)})
        )
      end)

    output =
      Enum.map(selected, fn row ->
        args = %{table: node.table, row: row}

        args =
          if kind == :update,
            do:
              Map.put(
                args,
                :value,
                evaluate(node.value, %{ctx | env: Map.put(ctx.env, node.alias, row)})
              ),
            else: args

        ctx.host.(kind, args)
      end)

    continue(bind(ctx, node.binding, output))
  end

  defp evaluate(expr, ctx),
    do: Evaluator.eval(expr, ctx.env, fn name -> ctx.host.(:source, %{name: name}) end)

  defp bind(ctx, nil, _), do: ctx
  defp bind(ctx, name, value), do: %{ctx | env: Map.put(ctx.env, name, value)}

  defp failure!(node, ctx),
    do: raise(Failure, status: node.status, code: node.code, message: evaluate(node.message, ctx))

  defp effects_allowed!(%{committed: true}),
    do: raise(Error, message: "Effects cannot execute after COMMIT", stage: :runtime)

  defp effects_allowed!(_), do: :ok

  defp writes_allowed!(ctx) do
    unless ctx.transaction and not ctx.committed,
      do:
        raise(Error,
          message: "Database writes and QUEUE require an active transaction",
          stage: :runtime
        )
  end

  defp continue(ctx), do: {:continue, ctx}
  defp result_context({:continue, ctx}), do: ctx
  defp result_context({:return, _, _, ctx}), do: ctx
  defp replace_context({:continue, _}, ctx), do: {:continue, ctx}
  defp replace_context({:return, value, status, _}, ctx), do: {:return, value, status, ctx}
end
