defmodule OrderLab.Language.Evaluator do
  @moduledoc "Pure evaluation of expression trees; data sources are supplied by the host."
  alias OrderLab.Language.Error

  def eval(expression, environment, sources \\ fn name -> raise "Unknown source #{name}" end)
  def eval({:literal, value}, _, _), do: value

  def eval({:variable, name}, env, sources) do
    case Map.fetch(env, name) do
      {:ok, value} -> value
      :error -> sources.(name)
    end
  end

  def eval({:field, parent, field}, env, sources) do
    case eval(parent, env, sources) do
      value when is_map(value) ->
        case Map.fetch(value, field) do
          {:ok, value} -> value
          :error -> fail("Unknown field #{field}")
        end

      _ ->
        fail("Cannot access #{field} on a value that is not a record")
    end
  end

  def eval({:list, values}, env, sources), do: Enum.map(values, &eval(&1, env, sources))

  def eval({:record, fields}, env, sources),
    do: Map.new(fields, fn {key, value} -> {key, eval(value, env, sources)} end)

  def eval({:present, value}, env, sources), do: not is_nil(eval(value, env, sources))
  def eval({:unary, "NOT", value}, env, sources), do: not boolean!(eval(value, env, sources))
  def eval({:unary, "-", value}, env, sources), do: -number!(eval(value, env, sources))

  def eval({:binary, "AND", left, right}, env, sources),
    do: boolean!(eval(left, env, sources)) and boolean!(eval(right, env, sources))

  def eval({:binary, "OR", left, right}, env, sources),
    do: boolean!(eval(left, env, sources)) or boolean!(eval(right, env, sources))

  def eval({:binary, operator, left, right}, env, sources),
    do: binary(operator, eval(left, env, sources), eval(right, env, sources))

  def eval({:builtin, name, args}, env, sources),
    do: call(name, Enum.map(args, &eval(&1, env, sources)), env, sources)

  def eval({:quantifier, kind, name, source, body}, env, sources) do
    rows = list!(eval(source, env, sources))

    case kind do
      "EXISTS" ->
        Enum.any?(rows, &boolean!(eval(body, Map.put(env, name, &1), sources)))

      "ALL" ->
        Enum.all?(rows, &boolean!(eval(body, Map.put(env, name, &1), sources)))

      "SUM" ->
        Enum.reduce(rows, 0, fn row, total ->
          total + number!(eval(body, Map.put(env, name, row), sources))
        end)
    end
  end

  def eval({:query, query}, env, sources) do
    rows = list!(eval(query.source, env, sources))

    rows =
      Enum.filter(rows, fn row ->
        row_env = Map.put(env, query.alias, row)

        Enum.all?(query.where, fn
          {:guard, guard, condition} ->
            not boolean!(eval(guard, env, sources)) or boolean!(eval(condition, row_env, sources))

          condition ->
            boolean!(eval(condition, row_env, sources))
        end)
      end)

    rows =
      if query.order == [],
        do: rows,
        else: Enum.sort(rows, &ordered?(&1, &2, query, env, sources))

    rows =
      if query.limit do
        limit = eval(query.limit, env, sources)
        unless is_integer(limit) and limit >= 0, do: fail("LIMIT must be a non-negative integer")
        Enum.take(rows, limit)
      else
        rows
      end

    rows =
      if query.select,
        do: Enum.map(rows, &eval(query.select, Map.put(env, query.alias, &1), sources)),
        else: rows

    case query.cardinality do
      "MANY" ->
        rows

      "FIRST" ->
        List.first(rows)

      "LAST" ->
        List.last(rows)

      "ONE" ->
        case rows do
          [] -> nil
          [row] -> row
          _ -> fail("ONE query returned more than one record")
        end
    end
  end

  defp ordered?(a, b, query, env, sources) do
    Enum.reduce_while(query.order, true, fn {key, direction}, _ ->
      left = eval(key, Map.put(env, query.alias, a), sources)
      right = eval(key, Map.put(env, query.alias, b), sources)
      comparable!(left, right)

      if left == right,
        do: {:cont, true},
        else: {:halt, if(direction == "DESC", do: left > right, else: left < right)}
    end)
  end

  defp binary(op, a, b) when op in ["=", "=="], do: a == b
  defp binary("!=", a, b), do: a != b
  defp binary("IN", a, b), do: a in list!(b)

  defp binary(op, a, b) when op in ["<", "<=", ">", ">="] do
    comparable!(a, b)

    case op do
      "<" -> a < b
      "<=" -> a <= b
      ">" -> a > b
      ">=" -> a >= b
    end
  end

  defp binary(op, a, b) when op in ["+", "-", "*", "/", "%"] do
    number!(a)
    number!(b)
    if op in ["/", "%"] and b == 0, do: fail("Division by zero")

    case op do
      "+" ->
        a + b

      "-" ->
        a - b

      "*" ->
        a * b

      "/" ->
        a / b

      "%" ->
        unless is_integer(a) and is_integer(b), do: fail("Remainder requires integers")
        rem(a, b)
    end
  end

  defp call(name, args, env, sources) do
    if name in Map.get(env, "$source_names", []),
      do: sources.({name, args}),
      else: builtin(name, args)
  end

  # Closed registry: no apply/Code.eval, atoms or native functions from user source.
  def builtin("count", [rows]) when is_list(rows), do: length(rows)
  def builtin("length", [value]) when is_list(value), do: length(value)
  def builtin("length", [value]) when is_binary(value), do: String.length(value)

  def builtin("contains", [value, needle]) when is_binary(value) and is_binary(needle),
    do: String.contains?(value, needle)

  def builtin("contains", [value, needle]) when is_list(value), do: needle in value
  def builtin("lower", [value]) when is_binary(value), do: String.downcase(value)
  def builtin("upper", [value]) when is_binary(value), do: String.upcase(value)

  def builtin("starts_with", [value, prefix]) when is_binary(value) and is_binary(prefix),
    do: String.starts_with?(value, prefix)

  def builtin("concat", [values]) when is_list(values) do
    unless Enum.all?(values, &is_binary/1), do: fail("concat requires a list of strings")
    Enum.join(values)
  end

  def builtin("merge", [left, right]) when is_map(left) and is_map(right),
    do: Map.merge(left, right)

  def builtin("coalesce", [value, default]), do: if(is_nil(value), do: default, else: value)

  def builtin("min", [left, right]) when is_number(left) and is_number(right),
    do: min(left, right)

  def builtin("max", [left, right]) when is_number(left) and is_number(right),
    do: max(left, right)

  def builtin("round", [value]) when is_number(value), do: round(value)

  def builtin("uuid", [prefix]) when is_binary(prefix),
    do: prefix <> "_" <> Base.encode16(:crypto.strong_rand_bytes(12), case: :lower)

  def builtin("ago", [duration, unit])
      when is_integer(duration) and duration >= 0 and
             unit in ["ms", "seconds", "minutes", "hours"] do
    multiplier = %{"ms" => 1, "seconds" => 1000, "minutes" => 60_000, "hours" => 3_600_000}[unit]

    DateTime.utc_now()
    |> DateTime.add(-duration * multiplier, :millisecond)
    |> DateTime.to_iso8601()
  end

  def builtin("now", []), do: DateTime.utc_now() |> DateTime.to_iso8601()
  def builtin("distinct", [values]) when is_list(values), do: Enum.uniq(values)

  def builtin("group_sum", [rows, key, amount])
      when is_list(rows) and is_binary(key) and is_binary(amount) do
    # Retain first occurrence order to make snapshots deterministic.
    {keys, groups} =
      Enum.reduce(rows, {[], %{}}, fn row, {keys, groups} ->
        unless is_map(row) and Map.has_key?(row, key) and Map.has_key?(row, amount),
          do: fail("group_sum fields do not exist")

        value = row[key]
        count = number!(row[amount])

        case Map.fetch(groups, value) do
          :error ->
            {keys ++ [value], Map.put(groups, value, %{key => value, amount => count})}

          {:ok, existing} ->
            {keys, Map.put(groups, value, Map.update!(existing, amount, &(&1 + count)))}
        end
      end)

    Enum.map(keys, &Map.fetch!(groups, &1))
  end

  def builtin(name, _), do: fail("Unknown builtin or invalid arguments: #{name}")

  def boolean!(value) when is_boolean(value), do: value
  def boolean!(_), do: fail("Expected Bool")
  def list!(value) when is_list(value), do: value
  def list!(_), do: fail("Expected List")
  def number!(value) when is_number(value), do: value
  def number!(_), do: fail("Expected Number")
  defp comparable!(a, b) when is_number(a) and is_number(b), do: :ok
  defp comparable!(a, b) when is_binary(a) and is_binary(b), do: :ok
  defp comparable!(_, _), do: fail("Comparison requires two numbers or two strings")
  defp fail(message), do: Error.fail!(message, stage: :runtime)
end
