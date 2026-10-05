defmodule Flow.Expression do
  @moduledoc "Bounded pure expression evaluation. Entity access and permission recursion are host callbacks."
  alias Flow.{ID, Ref, Syntax, Version, Embedded}

  @binary ~w(eq ne gt ge lt le add sub mul div contains)
  @unary ~w(not count)
  @quantified ~w(any all where flatMap sum)

  def validate(node) do
    try do
      check(node)
      :ok
    rescue
      error in Flow.ValidationError -> {:error, error}
    end
  end

  def evaluate(node, environment, options \\ []) do
    counter = Keyword.get_lazy(options, :counter, fn -> :counters.new(1, []) end)

    context = %{
      counter: counter,
      limit: Keyword.get(options, :max_steps, 100_000),
      field: Keyword.get(options, :field, &plain_field/2),
      can: Keyword.get(options, :can, fn _, _, _, _ -> false end),
      creates: Keyword.get(options, :creates, fn _, _ -> [] end),
      resolve: Keyword.get(options, :resolve, fn env, name -> Map.fetch!(env, name) end),
      extension:
        Keyword.get(options, :extension, fn _, _, _, _ ->
          raise ArgumentError, "Unsupported expression"
        end),
      constants: Keyword.get(options, :constants, %{})
    }

    eval(node, environment, context)
  end

  defp check(%{kind: kind}) when kind in [:symbol, :string, :number], do: :ok

  defp check(node) do
    {operator, args} = Syntax.form(node)

    cond do
      operator in @binary and length(args) == 2 ->
        Enum.each(args, &check/1)

      operator in @unary and length(args) == 1 ->
        Enum.each(args, &check/1)

      operator in ~w(and or list concat) ->
        Enum.each(args, &check/1)

      operator in @quantified and length(args) == 3 ->
        [collection, variable, expression] = args
        Syntax.identifier(variable)
        check(collection)
        check(expression)

      operator == "can" and length(args) == 2 ->
        [action, target] = args
        action!(action)
        check(target)

      operator == "canAs" and length(args) == 3 ->
        [actor, action, target] = args
        action!(action)
        check(actor)
        check(target)

      operator == "only" and length(args) >= 2 ->
        [changed | fields] = args

        unless Syntax.symbol(changed) == "changed",
          do: Syntax.fail(changed, :invalid_expression, "only requires changed")

        Enum.each(fields, &Syntax.identifier/1)

      operator == "creates" and length(args) == 2 ->
        [transaction, entity] = args

        unless Syntax.symbol(transaction) == "transaction",
          do: Syntax.fail(transaction, :invalid_expression, "creates requires transaction")

        Syntax.identifier(entity)

      operator == "record" ->
        Syntax.unique(args, fn field ->
          {name, values} = Syntax.form(field)

          if length(values) != 1,
            do: Syntax.fail(field, :arity, "Record fields require one value")

          check(hd(values))
          name
        end)

      true ->
        Syntax.fail(node, :invalid_expression, "Unsupported expression or arity: #{operator}")
    end
  end

  defp action!(node) do
    unless Syntax.symbol(node) in ~w(READ USE CREATE UPDATE DELETE INVOKE),
      do: Syntax.fail(node, :unknown_action, "Unknown permission action")
  end

  defp eval(node, env, context) do
    :counters.add(context.counter, 1, 1)

    if :counters.get(context.counter, 1) > context.limit,
      do:
        raise(Flow.ValidationError,
          code: :evaluation_limit,
          message: "Expression step budget exceeded"
        )

    evaluate_node(node, env, context)
  end

  defp evaluate_node(%{kind: :string, value: value}, _, _), do: value
  defp evaluate_node(%{kind: :number, value: value}, _, _), do: Decimal.new(value)

  defp evaluate_node(%{kind: :symbol, value: value}, env, context) do
    case value do
      "true" ->
        true

      "false" ->
        false

      "null" ->
        nil

      _ ->
        case String.split(value, ".") do
          [name | fields] ->
            initial =
              if Map.has_key?(env, name),
                do: context.resolve.(env, name),
                else: Map.fetch!(context.constants, name)

            Enum.reduce(fields, initial, fn field, current -> context.field.(current, field) end)
        end
    end
  end

  defp evaluate_node(node, env, context) do
    {operator, args} = Syntax.form(node)

    case {operator, args} do
      {"and", values} ->
        Enum.all?(values, &boolean!(eval(&1, env, context)))

      {"or", values} ->
        Enum.any?(values, &boolean!(eval(&1, env, context)))

      {"not", [value]} ->
        not boolean!(eval(value, env, context))

      {"list", values} ->
        Enum.map(values, &eval(&1, env, context))

      {"record", fields} ->
        Map.new(fields, fn field ->
          {name, [value]} = Syntax.form(field)
          {name, eval(value, env, context)}
        end)

      {"concat", values} ->
        Enum.map_join(values, &text!(eval(&1, env, context)))

      {"count", [collection]} ->
        length(list!(eval(collection, env, context)))

      {"only", [changed | fields]} ->
        changes = list!(eval(changed, env, context))
        allowed = Enum.map(fields, &Syntax.symbol/1)
        Enum.all?(changes, &(&1 in allowed))

      {"creates", [_, entity]} ->
        context.creates.(Map.fetch!(env, "transaction"), Syntax.symbol(entity))

      {"can", [action, target]} ->
        context.can.(
          Map.fetch!(env, "actor"),
          Syntax.symbol(action),
          eval(target, env, context),
          context.counter
        ) == true

      {"canAs", [actor, action, target]} ->
        context.can.(
          eval(actor, env, context),
          Syntax.symbol(action),
          eval(target, env, context),
          context.counter
        ) == true

      {operator, [collection, variable, expression]} when operator in @quantified ->
        values = list!(eval(collection, env, context))
        variable = Syntax.symbol(variable)
        f = fn value -> eval(expression, Map.put(env, variable, value), context) end

        case operator do
          "any" ->
            Enum.any?(values, &boolean!(f.(&1)))

          "all" ->
            Enum.all?(values, &boolean!(f.(&1)))

          "where" ->
            Enum.filter(values, &boolean!(f.(&1)))

          "flatMap" ->
            Enum.flat_map(values, &list!(f.(&1)))

          "sum" ->
            Enum.reduce(values, Decimal.new(0), fn value, total ->
              Decimal.add(total, numeric!(f.(value)))
            end)
        end

      {operator, [a, b]} when operator in @binary ->
        a = eval(a, env, context)
        b = eval(b, env, context)
        binary(operator, a, b)

      _ ->
        context.extension.(node, env, context, fn expression, environment ->
          eval(expression, environment, context)
        end)
    end
  end

  defp binary("eq", a, b), do: equal?(a, b)
  defp binary("ne", a, b), do: not equal?(a, b)
  defp binary("contains", values, value), do: Enum.any?(list!(values), &equal?(&1, value))

  defp binary(op, a, b) when op in ~w(gt ge lt le) do
    compared = compare(a, b)

    case op do
      "gt" -> compared == :gt
      "ge" -> compared in [:gt, :eq]
      "lt" -> compared == :lt
      "le" -> compared in [:lt, :eq]
    end
  end

  defp binary("add", a, b), do: Decimal.add(numeric!(a), numeric!(b))
  defp binary("sub", a, b), do: Decimal.sub(numeric!(a), numeric!(b))
  defp binary("mul", a, b), do: Decimal.mult(numeric!(a), numeric!(b))
  defp binary("div", a, b), do: Decimal.div(numeric!(a), numeric!(b))

  def equal?(%Ref{entity: entity, id: a}, %Ref{entity: entity, id: b}), do: equal?(a, b)
  def equal?(%Version{reference: a}, b), do: equal?(a, b)
  def equal?(a, %Version{reference: b}), do: equal?(a, b)
  def equal?(%Embedded{value: a}, b), do: equal?(a, b)
  def equal?(a, %Embedded{value: b}), do: equal?(a, b)
  def equal?(%ID{entity: entity, value: a}, %ID{entity: entity, value: b}), do: a == b

  def equal?(%Decimal{} = a, b) when is_integer(b) or is_struct(b, Decimal),
    do: Decimal.equal?(a, b)

  def equal?(a, %Decimal{} = b) when is_integer(a), do: Decimal.equal?(a, b)

  def equal?(a, b) when is_list(a) and is_list(b),
    do: length(a) == length(b) and Enum.all?(Enum.zip(a, b), fn {x, y} -> equal?(x, y) end)

  def equal?(a, b) when is_map(a) and is_map(b) and not is_struct(a) and not is_struct(b),
    do:
      Map.keys(a) |> MapSet.new() |> MapSet.equal?(MapSet.new(Map.keys(b))) and
        Enum.all?(a, fn {k, v} -> equal?(v, b[k]) end)

  def equal?(a, b), do: a === b

  defp compare(%DateTime{} = a, %DateTime{} = b), do: DateTime.compare(a, b)

  defp compare(a, b) when is_binary(a) and is_binary(b) do
    cond do
      a < b -> :lt
      a > b -> :gt
      true -> :eq
    end
  end

  defp compare(a, b), do: Decimal.compare(numeric!(a), numeric!(b))
  defp numeric!(%Decimal{coef: c} = value) when is_integer(c), do: value
  defp numeric!(value) when is_integer(value), do: Decimal.new(value)
  defp numeric!(_), do: raise(ArgumentError, "Expected exact number")
  defp boolean!(value) when is_boolean(value), do: value
  defp boolean!(_), do: raise(ArgumentError, "Expected boolean")
  defp list!(value) when is_list(value), do: value
  defp list!(_), do: raise(ArgumentError, "Expected collection")
  defp text!(%ID{value: value}), do: value
  defp text!(%Decimal{} = value), do: Decimal.to_string(value)
  defp text!(value) when is_binary(value), do: value
  defp text!(_), do: raise(ArgumentError, "Expected text")

  defp plain_field(value, field) when is_map(value) and not is_struct(value),
    do: Map.fetch!(value, field)

  defp plain_field(values, field) when is_list(values),
    do: Enum.map(values, &plain_field(&1, field))

  defp plain_field(_, _), do: raise(ArgumentError, "Field access requires an entity resolver")
end
