defmodule Flow.Evaluator do
  @moduledoc false
  alias Flow.{
    Access,
    Transaction,
    Expression,
    Syntax,
    Checker,
    Value,
    Schema,
    ID,
    Ref,
    Store,
    Live
  }

  def run(session, program, operation, inputs, handlers, event \\ nil) do
    env = Map.new(inputs, fn {name, value} -> {"$" <> name, value} end)

    env =
      Map.put(env, "request", %{
        "id" => %ID{entity: "Request", value: Transaction.random_id()},
        "time" => session.context["now"]
      })

    env = if event, do: Map.put(env, "event", event), else: env
    runtime = %{session: session, program: program, handlers: handlers}
    env = scope(runtime, operation.bindings, env)

    if when_node = operation.metadata["when"] do
      [condition] = Syntax.args(when_node, "when", 1)
      if evaluate(runtime, condition, env) != true, do: throw({:skip_event, operation.name})
    end

    Enum.each(Enum.sort(Map.keys(operation.bindings)), &resolve(runtime, env, &1))
    evaluate(runtime, operation.result, env)
  end

  defp scope(runtime, bindings, env) do
    scope = make_ref()
    env = Map.merge(env, Map.new(bindings, fn {name, _} -> {name, {:flow_lazy, scope, name}} end))
    :ets.insert(runtime.session.cache, {{:scope, scope}, {bindings, env}})
    env
  end

  defp resolve(runtime, env, name) do
    case Map.fetch!(env, name) do
      {:flow_lazy, scope, name} ->
        key = {:binding, scope, name}

        case :ets.lookup(runtime.session.cache, key) do
          [{_, {:value, value}}] ->
            value

          [{_, :evaluating}] ->
            raise Flow.ValidationError, code: :cyclic_binding, message: "Cyclic binding"

          [] ->
            :ets.insert(runtime.session.cache, {key, :evaluating})
            [{_, {bindings, scope_env}}] = :ets.lookup(runtime.session.cache, {:scope, scope})
            value = evaluate(runtime, Map.fetch!(bindings, name), scope_env)
            :ets.insert(runtime.session.cache, {key, {:value, value}})
            value
        end

      value ->
        value
    end
  end

  defp evaluate(runtime, node, env) do
    constants =
      runtime.program.schema.types
      |> Enum.flat_map(fn
        {_, {:enum, values}} -> Enum.map(values, &{&1, &1})
        _ -> []
      end)
      |> Map.new()

    Expression.evaluate(node, env,
      counter: runtime.session.counter,
      constants: constants,
      field: &field(runtime.session, &1, &2),
      resolve: &resolve(runtime, &1, &2),
      extension: fn node, env, context, eval -> extension(runtime, node, env, context, eval) end
    )
  end

  defp field(_session, %Flow.Image{} = image, name) when name in ["width", "height"],
    do: Map.fetch!(image, if(name == "width", do: :width, else: :height))

  defp field(session, value, name), do: Access.field(session, value, name)

  defp extension(runtime, node, env, _context, eval) do
    {operator, args} = Syntax.form(node)
    session = runtime.session
    schema = runtime.program.schema
    value = &eval.(&1, env)

    case {operator, args} do
      {"entity", [id]} ->
        %ID{entity: entity} = id = value.(id)
        reference = %Ref{entity: entity, id: id}

        unless Transaction.change(session, reference) ||
                 Store.fetch(session.db, reference, ["id"]) do
          unavailable!()
        end

        reference

      {"entities", [name]} ->
        entity = Syntax.symbol(name)
        refs = Store.ids(session.db, entity) ++ Transaction.creates(session, entity)
        refs |> Enum.uniq() |> Access.visible(session)

      {"new", [name]} ->
        {:id, entity} = Schema.resolve(schema, {:named, Syntax.symbol(name)})
        %ID{entity: entity, value: Transaction.random_id()}

      {"as", [name, input]} ->
        Value.validate!(schema, {:named, Syntax.symbol(name)}, value.(input))

      {"map", [collection, variable | values]} ->
        collection = Access.visible(value.(collection), session)
        bindings = Checker.bindings(values)

        Enum.map(collection, fn item ->
          local = scope(runtime, bindings, Map.put(env, Syntax.symbol(variable), item))
          Map.new(bindings, fn {name, _} -> {name, resolve(runtime, local, name)} end)
        end)

      {"groupSum", [collection, key, quantity, brand]} ->
        key = Syntax.symbol(key)
        quantity = Syntax.symbol(quantity)

        value.(collection)
        |> Enum.group_by(&field(session, &1, key))
        |> Enum.map(fn {id, items} ->
          sum =
            Enum.reduce(items, Decimal.new(0), fn item, total ->
              Decimal.add(total, field(session, item, quantity))
            end)

          %{key => id, quantity => Value.validate!(schema, {:named, Syntax.symbol(brand)}, sum)}
        end)

      {"last", [collection, count]} ->
        count = count |> Flow.Types.constant() |> Decimal.to_integer()
        value.(collection) |> Access.visible(session) |> Enum.take(-count)

      {"single", [collection]} ->
        case value.(collection) |> Access.visible(session) do
          [] -> nil
          [item] -> item
        end

      {"order", [collection, path, direction]} ->
        [_, name] = String.split(Syntax.symbol(path), ".")
        values = value.(collection) |> Access.visible(session)

        Enum.sort(values, fn a, b ->
          a = field(session, a, name)
          b = field(session, b, name)
          compared = compare(a, b)
          if Syntax.symbol(direction) == "ASC", do: compared != :gt, else: compared != :lt
        end)

      {"ago", [duration]} ->
        DateTime.add(session.context["now"], -Checker.duration!(duration), :millisecond)

      {"since", [collection, time]} ->
        cutoff = value.(time)

        value.(collection)
        |> Access.visible(session)
        |> Enum.filter(&(DateTime.compare(field(session, &1, "receivedAt"), cutoff) != :lt))

      {"live", [collection]} ->
        %Live{values: value.(collection) |> Access.visible(session)}

      {op, [name, input]} when op in ~w(create publish) ->
        Transaction.create(session, Syntax.symbol(name), value.(input))

      {"set", [path, input]} ->
        parts = String.split(Syntax.symbol(path), ".")
        target = eval.(%{path | value: Enum.drop(parts, -1) |> Enum.join(".")}, env)
        Transaction.set(session, target, List.last(parts), value.(input))

      {"delete", [target]} ->
        Transaction.delete(session, value.(target))

      {"invoke", [method, input]} ->
        method = Syntax.symbol(method)
        contract = runtime.program.plugins[method]
        args = Transaction.invocation(session, method, value.(input), contract)
        handler = Map.fetch!(runtime.handlers, method)

        output =
          case contract.mode do
            :pure -> handler.function.(args)
            :transactional -> handler.function.(session, args)
          end

        Value.validate!(schema, contract.output, output)

      {"enqueue", [method, input, retry]} ->
        method = Syntax.symbol(method)
        contract = runtime.program.plugins[method]
        args = Transaction.invocation(session, method, value.(input), contract)
        Flow.Queue.stage(session, method, args, retry)

      _ ->
        raise ArgumentError, "Unsupported operation #{operator}"
    end
  end

  defp compare(%ID{value: a}, %ID{value: b}), do: compare(a, b)
  defp compare(%DateTime{} = a, %DateTime{} = b), do: DateTime.compare(a, b)
  defp compare(%Decimal{} = a, b), do: Decimal.compare(a, b)
  defp compare(a, %Decimal{} = b), do: Decimal.compare(a, b)

  defp compare(a, b) do
    cond do
      a < b -> :lt
      a > b -> :gt
      true -> :eq
    end
  end

  defp unavailable!,
    do: raise(Flow.ValidationError, code: :not_found, message: "Resource unavailable")
end
