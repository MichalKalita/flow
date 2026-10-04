defmodule OrderLab.Language.Checker do
  @moduledoc "Structural expression checking; refinements are enforced at value boundaries."
  alias OrderLab.Language.{Error, Types}
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
    type = hd(types)

    unless Enum.all?(types, &(compatible?(&1, type) or compatible?(type, &1))),
      do: fail("List items must have compatible types")

    {:list, if(Enum.all?(types, &(base(&1) == @int)), do: @int, else: type)}
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
    expect!(infer(b, env), @bool)
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

    Enum.each(query.where, fn
      {:guard, guard, predicate} ->
        expect!(infer(guard, env), @bool)
        expect!(infer(predicate, narrow(guard, row_env)), @bool)

      predicate ->
        expect!(infer(predicate, row_env), @bool)
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
