defmodule OrderLab.Language.Checker do
  @moduledoc "Structural expression checking; refinements are enforced at value boundaries."
  alias OrderLab.Language.{Error, Types, Evaluator}
  @bool {:named, "Bool"}
  @int {:named, "Int"}
  @number {:named, "Number"}
  @string {:named, "String"}
  @json {:named, "JSON"}

  def infer({:literal, nil}, _), do: :null
  def infer({:literal, v}, _) when is_boolean(v), do: @bool
  def infer({:literal, v}, _) when is_integer(v), do: @int
  def infer({:literal, v}, _) when is_number(v), do: @number
  def infer({:literal, v}, _) when is_binary(v), do: @string

  def infer({:variable, name}, env),
    do: Map.get(env, name) || fail("Unknown variable or source #{name}")

  def infer({:field, parent, field}, env) do
    case base(infer(parent, env)) do
      {:record, fields} ->
        Map.get(fields, field) || fail("Unknown field #{field}")

      {:named, name} when name in ["File", "Image"] ->
        Map.get(OrderLab.FileValue.fields(name), field) || fail("Unknown #{name} field #{field}")

      {:optional, _} ->
        fail("Optional record must be guarded with IS PRESENT before accessing #{field}")

      _ ->
        fail("Field #{field} requires a record")
    end
  end

  def infer({:record, fields}, env),
    do: {:record, Map.new(fields, fn {key, value} -> {key, infer(value, env)} end)}

  def infer({:list, []}, _), do: {:list, :empty}

  def infer({:list, items}, env) do
    types = Enum.map(items, &infer(&1, env))
    {:list, Enum.reduce(tl(types), hd(types), &common!/2)}
  end

  def infer({:conditional, condition, positive, negative}, env) do
    expect!(infer(condition, env), @bool)

    common!(
      infer(positive, narrow(condition, env)),
      infer(negative, narrow_false(condition, env))
    )
  end

  def infer({:present, value}, env) do
    infer(value, env)
    @bool
  end

  def infer({:unary, "NOT", value}, env), do: expect!(infer(value, env), @bool)
  def infer({:unary, "-", value}, env), do: numeric!(infer(value, env))

  def infer({:binary, "AND", a, b}, env) do
    expect!(infer(a, env), @bool)
    expect!(infer(b, narrow(a, env)), @bool)
  end

  def infer({:binary, "OR", a, b}, env) do
    expect!(infer(a, env), @bool)
    expect!(infer(b, narrow_false(a, env)), @bool)
  end

  def infer({:binary, op, a, b}, env) when op in ["+", "-", "*", "/", "%"] do
    left = numeric!(infer(a, env))
    right = numeric!(infer(b, env))

    if op == "%",
      do:
        (
          expect!(left, @int)
          expect!(right, @int)
        )

    if op != "/" and left == @int and right == @int, do: @int, else: @number
  end

  def infer({:binary, "IN", a, b}, env) do
    expect!(infer(a, env), element!(infer(b, env)))
    @bool
  end

  def infer({:binary, op, a, b}, env) when op in ["=", "==", "!=", "<", "<=", ">", ">="] do
    left = base(infer(a, env))
    right = base(infer(b, env))

    unless compatible?(left, right) or compatible?(right, left),
      do: fail("Comparison uses incompatible types")

    if op in ["<", "<=", ">", ">="] do
      unless (numeric?(left) and numeric?(right)) or (left == @string and right == @string),
        do: fail("Ordered comparison requires numbers or strings")
    end

    @bool
  end

  def infer({:quantifier, kind, name, source, body}, env) do
    element = element!(infer(source, env))
    result = infer(body, Map.put(env, name, element))
    if kind == "SUM", do: numeric!(result), else: expect!(result, @bool)
  end

  def infer({:query, query}, env) do
    row = element!(infer(query.source, env))
    row_env = Map.put(env, query.alias, row)

    aliases = [query.alias | Enum.map(query.joins, & &1.alias)]
    unless length(aliases) == length(Enum.uniq(aliases)), do: fail("Duplicate query alias")

    row_env =
      Enum.reduce(query.joins, row_env, fn join, row_env ->
        if join.guard, do: expect!(infer(join.guard, env), @bool)
        inner_env = if join.guard, do: narrow(join.guard, row_env), else: row_env
        type = element!(infer(join.source, inner_env))
        expect!(infer(join.condition, Map.put(inner_env, join.alias, type)), @bool)
        output = if join.kind == :left or join.guard, do: {:optional, type}, else: type
        Map.put(row_env, join.alias, output)
      end)

    row_env =
      Enum.reduce(query.where, row_env, fn
        {:guard, guard, predicate}, row_env ->
          expect!(infer(guard, env), @bool)
          expect!(infer(predicate, narrow(guard, row_env)), @bool)
          row_env

        predicate, row_env ->
          expect!(infer(predicate, row_env), @bool)
          narrow(predicate, row_env)
      end)

    Enum.each(query.order, fn {key, _} ->
      type = base(infer(key, row_env))
      unless numeric?(type) or type == @string, do: fail("ORDER BY requires a number or string")
    end)

    if query.limit, do: expect!(infer(query.limit, env), @int)
    selected = if query.select, do: infer(query.select, row_env), else: row
    if query.cardinality == "MANY", do: {:list, selected}, else: {:optional, selected}
  end

  def infer({:builtin, name, args}, env) do
    types = Enum.map(args, &infer(&1, env))

    case Map.fetch(Map.get(env, "$sources", %{}), name) do
      {:ok, source} ->
        unless length(types) == length(source.params),
          do: fail("Wrong number of source parameters for #{name}")

        Enum.zip(types, source.params)
        |> Enum.each(fn {actual, param} -> expect!(actual, param.type) end)

        {:list, source.row_type}

      :error ->
        builtin(name, types, args)
    end
  end

  defp builtin(name, [type], _) when name in ["count", "distinct"] do
    element!(type)
    if name == "count", do: @int, else: type
  end

  defp builtin("length", [type], _) do
    unless match?({:list, _}, base(type)) or base(type) == @string,
      do: fail("length requires String or List")

    @int
  end

  defp builtin(name, [type], _) when name in ["lower", "upper", "uuid"],
    do:
      (
        expect!(type, @string)
        @string
      )

  defp builtin("now", [], _), do: @string

  defp builtin("ago", [duration, unit], _),
    do:
      (
        expect!(duration, @int)
        expect!(unit, @string)
        @string
      )

  defp builtin("starts_with", [a, b], _),
    do:
      (
        expect!(a, @string)
        expect!(b, @string)
        @bool
      )

  defp builtin("contains", [a, b], _) do
    case base(a) do
      {:list, type} ->
        expect!(b, type)

      _ ->
        expect!(a, @string)
        expect!(b, @string)
    end

    @bool
  end

  defp builtin("concat", [a], _),
    do:
      (
        expect!(element!(a), @string)
        @string
      )

  defp builtin("round", [a], _),
    do:
      (
        numeric!(a)
        @int
      )

  defp builtin(name, [a, b], _) when name in ["min", "max"] do
    numeric!(a)
    numeric!(b)
    if base(a) == @int and base(b) == @int, do: @int, else: @number
  end

  defp builtin("coalesce", [a, b], _) do
    case base(a) do
      {:optional, inner} ->
        expect!(b, inner)
        inner

      :null ->
        b

      inner ->
        expect!(b, inner)
        inner
    end
  end

  defp builtin("merge", [a, b], _) do
    case {base(a), base(b)} do
      {{:record, left}, {:record, right}} -> {:record, Map.merge(left, right)}
      _ -> fail("merge requires records")
    end
  end

  defp builtin("group_sum", [rows, key_type, amount_type], [
         _,
         {:literal, key},
         {:literal, amount}
       ]) do
    expect!(key_type, @string)
    expect!(amount_type, @string)

    case base(element!(rows)) do
      {:record, fields} ->
        key_type = Map.get(fields, key) || fail("Unknown group_sum key #{key}")
        amount_type = Map.get(fields, amount) || fail("Unknown group_sum amount #{amount}")
        numeric!(amount_type)
        # A sum is not guaranteed to satisfy the refinement on individual amounts.
        {:list, {:record, %{key => key_type, amount => base(amount_type)}}}

      _ ->
        fail("group_sum requires records")
    end
  end

  defp builtin(name, _, _), do: fail("Unknown builtin or invalid arguments: #{name}")

  def narrow_false({:unary, "NOT", predicate}, env), do: narrow(predicate, env)
  def narrow_false(_, env), do: env

  def narrow({:present, {:variable, name}}, env), do: Map.update!(env, name, &unwrap_optional/1)

  def narrow({:present, {:field, {:variable, name}, field}}, env) do
    Map.update!(env, name, fn type ->
      case base(type) do
        {:record, fields} -> {:record, Map.update!(fields, field, &unwrap_optional/1)}
        _ -> type
      end
    end)
  end

  def narrow({:binary, "AND", a, b}, env), do: narrow(b, narrow(a, env))
  def narrow({:unary, "NOT", {:unary, "NOT", predicate}}, env), do: narrow(predicate, env)
  def narrow(_, env), do: env

  defp unwrap_optional(type) do
    case base(type) do
      {:optional, inner} -> inner
      inner -> inner
    end
  end

  def base({:refined, type, _}), do: base(type)
  def base(type), do: type

  def element!(type) do
    case base(type) do
      {:list, inner} -> inner
      _ -> fail("Expected List, got #{inspect(type)}")
    end
  end

  def compatible?(actual, expected) do
    case {base(actual), base(expected)} do
      {{:named, "Image"}, {:named, "File"}} ->
        true

      {_, @json} ->
        true

      {:empty, _} ->
        true

      {:null, {:optional, _}} ->
        true

      {{:optional, a}, {:optional, b}} ->
        compatible?(a, b)

      {a, {:optional, b}} ->
        compatible?(a, b)

      {{:named, a}, {:named, b}}
      when a in ["Int", "Float", "Number"] and b in ["Float", "Number"] ->
        true

      {{:list, a}, {:list, b}} ->
        compatible?(a, b)

      {{:record, a}, {:record, b}} ->
        Map.keys(a) -- Map.keys(b) == [] and
          Enum.all?(b, fn {key, type} ->
            case Map.fetch(a, key) do
              {:ok, actual} -> compatible?(actual, type)
              :error -> match?({:optional, _}, type)
            end
          end)

      {a, a} ->
        true

      _ ->
        false
    end
  end

  def common!(a, b) do
    case {a, b} do
      {same, same} -> same
      _ -> common_base!(base(a), base(b))
    end
  end

  defp common_base!(a, a), do: a
  defp common_base!(:empty, b), do: b
  defp common_base!(a, :empty), do: a
  defp common_base!(:null, {:optional, b}), do: {:optional, b}
  defp common_base!({:optional, a}, :null), do: {:optional, a}
  defp common_base!(:null, b), do: {:optional, b}
  defp common_base!(a, :null), do: {:optional, a}
  defp common_base!({:optional, a}, {:optional, b}), do: {:optional, common!(a, b)}
  defp common_base!({:optional, a}, b), do: {:optional, common!(a, b)}
  defp common_base!(a, {:optional, b}), do: {:optional, common!(a, b)}
  defp common_base!({:list, a}, {:list, b}), do: {:list, common!(a, b)}

  defp common_base!({:record, a}, {:record, b}) do
    unless Enum.sort(Map.keys(a)) == Enum.sort(Map.keys(b)),
      do: fail("Branches and list records must have the same fields")

    {:record, Map.new(a, fn {key, type} -> {key, common!(type, b[key])} end)}
  end

  defp common_base!({:named, a}, {:named, b})
       when a in ~w(Int Number Float) and b in ~w(Int Number Float), do: @number

  defp common_base!(_, _), do: fail("Branches or list items have incompatible types")

  def expect_expr!(expression, expected, env) do
    expect!(infer(expression, env), expected)
    check_constants!(expression, expected)
  end

  defp check_constants!(expression, expected) do
    if constant?(expression) do
      try do
        Types.validate!(Evaluator.eval(expression, %{}), expected)
      rescue
        error in Error -> reraise %{error | stage: :compile}, __STACKTRACE__
      end
    else
      case {expression, base(expected)} do
        {{:conditional, _, positive, negative}, _} ->
          check_constants!(positive, expected)
          check_constants!(negative, expected)

        {{:record, fields}, {:record, types}} ->
          Enum.each(fields, fn {key, expr} -> check_constants!(expr, Map.fetch!(types, key)) end)

        {{:list, items}, {:list, type}} ->
          Enum.each(items, &check_constants!(&1, type))

        {_, {:optional, type}} ->
          check_constants!(expression, type)

        _ ->
          :ok
      end
    end
  end

  def pure_refinement!({:builtin, name, _}) when name in ~w(uuid now ago),
    do: fail("Refinements must be deterministic; #{name} is not allowed")

  def pure_refinement!(value) when is_tuple(value),
    do: value |> Tuple.to_list() |> Enum.each(&pure_refinement!/1)

  def pure_refinement!(value) when is_list(value), do: Enum.each(value, &pure_refinement!/1)

  def pure_refinement!(value) when is_map(value),
    do: Enum.each(value, fn {_, item} -> pure_refinement!(item) end)

  def pure_refinement!(_), do: :ok

  defp constant?({:literal, _}), do: true
  defp constant?({:record, fields}), do: Enum.all?(fields, fn {_, expr} -> constant?(expr) end)
  defp constant?({:list, items}), do: Enum.all?(items, &constant?/1)
  defp constant?({:unary, _, value}), do: constant?(value)
  defp constant?({:binary, _, left, right}), do: constant?(left) and constant?(right)

  defp constant?({:conditional, condition, positive, negative}),
    do: constant?(condition) and constant?(positive) and constant?(negative)

  defp constant?({:present, value}), do: constant?(value)

  defp constant?({:builtin, name, args})
       when name in ~w(count length contains lower upper starts_with concat merge coalesce min max round distinct group_sum),
       do: Enum.all?(args, &constant?/1)

  defp constant?(_), do: false

  def expect!(actual, expected) do
    unless compatible?(actual, expected),
      do: fail("Expected #{Types.describe(expected)}, got #{inspect(actual)}")

    actual
  end

  defp numeric!(type) do
    type = base(type)
    unless numeric?(type), do: fail("Expected numeric type")
    type
  end

  defp numeric?({:named, name}), do: name in ["Int", "Float", "Number"]
  defp numeric?(_), do: false
  defp fail(message), do: Error.fail!(message, stage: :compile)
end
