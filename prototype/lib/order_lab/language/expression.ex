defmodule OrderLab.Language.Expression do
  alias OrderLab.Language.{Lexer, Error}

  @precedence %{
    "OR" => 10,
    "AND" => 20,
    "=" => 30,
    "==" => 30,
    "!=" => 30,
    "<" => 30,
    ">" => 30,
    "<=" => 30,
    ">=" => 30,
    "IN" => 30,
    "IS" => 30,
    "BETWEEN" => 30,
    "+" => 40,
    "-" => 40,
    "*" => 50,
    "/" => 50,
    "%" => 50
  }

  def parse!(tokens, file) do
    {expression, rest} = parse(tokens, file)
    unless rest == [], do: fail!("Unexpected token after expression", rest, file)
    expression
  end

  def parse(tokens, file), do: expr(tokens, 0, file)

  defp expr(tokens, minimum, file) do
    {left, rest} = prefix(tokens, file)
    {left, rest} = postfix(left, rest, file)
    infix(left, rest, minimum, file)
  end

  defp prefix([{:number, value, _, _} | rest], _), do: {{:literal, value}, rest}
  defp prefix([{:string, value, _, _} | rest], _), do: {{:literal, value}, rest}

  defp prefix([{:word, value, _, _} | rest], _) when value in ["true", "false", "null"],
    do: {{:literal, %{"true" => true, "false" => false, "null" => nil}[value]}, rest}

  defp prefix([{:symbol, ":", _, _}, {:word, name, _, _} | rest], _),
    do: {{:variable, name}, rest}

  defp prefix([{:word, "NOT", _, _} | rest], file) do
    {value, rest} = expr(rest, 25, file)
    {{:unary, "NOT", value}, rest}
  end

  defp prefix([{:symbol, "-", _, _} | rest], file) do
    {value, rest} = expr(rest, 60, file)
    {{:unary, "-", value}, rest}
  end

  defp prefix([{:symbol, "(", _, _} | rest], file) do
    {value, rest} = parse(rest, file)
    {value, expect(rest, ")", file)}
  end

  defp prefix([{:symbol, "[", _, _} | rest], file) do
    {items, rest} = sequence(rest, "]", file, [])
    {{:list, items}, rest}
  end

  defp prefix([{:symbol, "{", _, _} | rest], file) do
    {fields, rest} = fields(rest, file, [])
    {{:record, fields}, rest}
  end

  defp prefix([{:word, quantifier, _, _}, {:word, name, _, _}, {:word, "IN", _, _} | rest], file)
       when quantifier in ["EXISTS", "ALL", "SUM"] do
    {source, rest} = expr(rest, 0, file)
    delimiter = %{"EXISTS" => "WHERE", "ALL" => "SATISFY", "SUM" => "OF"}[quantifier]
    {body, rest} = expr(expect(rest, delimiter, file), 0, file)
    {{:quantifier, quantifier, name, source, body}, rest}
  end

  defp prefix([{:word, "COUNT", _, _} | rest], file) do
    {value, rest} = expr(rest, 60, file)
    {{:builtin, "count", [value]}, rest}
  end

  defp prefix([{:word, cardinality, _, _}, {:word, "FROM", _, _} | rest], file)
       when cardinality in ["ONE", "FIRST", "LAST"] do
    query(rest, file, cardinality)
  end

  defp prefix([{:word, "FROM", _, _} | rest], file), do: query(rest, file, "MANY")
  defp prefix([{:word, name, _, _} | rest], _), do: {{:variable, name}, rest}
  defp prefix(tokens, file), do: fail!("Expected expression", tokens, file)

  defp postfix(left, [{:symbol, ".", _, _}, {:word, field, _, _} | rest], file),
    do: postfix({:field, left, field}, rest, file)

  defp postfix(left, [{:symbol, "(", _, _} | rest], file) do
    {args, rest} = sequence(rest, ")", file, [])
    name = name!(left, file)
    postfix({:builtin, name, args}, rest, file)
  end

  defp postfix(left, rest, _), do: {left, rest}

  defp infix(left, [token | rest] = tokens, minimum, file) do
    op = Lexer.text(token)
    precedence = Map.get(@precedence, op, -1)

    if precedence < minimum do
      {left, tokens}
    else
      case op do
        "IS" ->
          {negated, rest} = if text(rest) == "NOT", do: {true, tl(rest)}, else: {false, rest}
          rest = expect(rest, "PRESENT", file)
          value = {:present, left}
          value = if negated, do: {:unary, "NOT", value}, else: value
          infix(value, rest, minimum, file)

        "BETWEEN" ->
          {low, rest} = expr(rest, 31, file)
          {high, rest} = expr(expect(rest, "AND", file), 31, file)

          infix(
            {:binary, "AND", {:binary, ">=", left, low}, {:binary, "<=", left, high}},
            rest,
            minimum,
            file
          )

        _ ->
          {right, rest} = expr(rest, precedence + 1, file)
          infix({:binary, op, left, right}, rest, minimum, file)
      end
    end
  end

  defp infix(left, [], _, _), do: {left, []}

  defp query(tokens, file, cardinality) do
    {source, rest} = expr(tokens, 60, file)
    rest = expect(rest, "AS", file)
    {alias_name, rest} = identifier(rest, file)

    query_clauses(rest, file, %{
      source: source,
      alias: alias_name,
      cardinality: cardinality,
      where: [],
      select: nil,
      order: [],
      limit: nil
    })
  end

  defp query_clauses([{:word, "WHERE", _, _} | rest], file, query) do
    {condition, rest} = parse(rest, file)
    query_clauses(rest, file, %{query | where: query.where ++ [condition]})
  end

  defp query_clauses([{:word, "SELECT", _, _} | rest], file, query) do
    if query.select, do: fail!("Duplicate SELECT", rest, file)
    {selection, rest} = parse(rest, file)
    query_clauses(rest, file, %{query | select: selection})
  end

  defp query_clauses([{:word, "ORDER", _, _}, {:word, "BY", _, _} | rest], file, query) do
    {key, rest} = parse(rest, file)

    {direction, rest} =
      case text(rest) do
        x when x in ["ASC", "DESC"] -> {x, tl(rest)}
        _ -> {"ASC", rest}
      end

    rest = if text(rest) == ",", do: tl(rest), else: rest
    query_clauses(rest, file, %{query | order: query.order ++ [{key, direction}]})
  end

  defp query_clauses([{:word, "LIMIT", _, _} | rest], file, query) do
    {limit, rest} = parse(rest, file)
    query_clauses(rest, file, %{query | limit: limit})
  end

  defp query_clauses(rest, _, query), do: {{:query, query}, rest}

  defp sequence([{:symbol, close, _, _} | rest], close, _, acc), do: {Enum.reverse(acc), rest}

  defp sequence(tokens, close, file, acc) do
    {value, rest} = parse(tokens, file)

    case text(rest) do
      ^close -> {Enum.reverse([value | acc]), tl(rest)}
      "," -> sequence(tl(rest), close, file, [value | acc])
      _ -> fail!("Expected comma or #{close}", rest, file)
    end
  end

  defp fields([{:symbol, "}", _, _} | rest], _, acc), do: {Enum.reverse(acc), rest}

  defp fields(tokens, file, acc) do
    {name, rest} = identifier(tokens, file)
    if Enum.any?(acc, &(elem(&1, 0) == name)), do: fail!("Duplicate field #{name}", tokens, file)
    {value, rest} = parse(expect(rest, ":", file), file)

    case text(rest) do
      "}" -> {Enum.reverse([{name, value} | acc]), tl(rest)}
      "," -> fields(tl(rest), file, [{name, value} | acc])
      _ -> fail!("Expected comma or closing brace", rest, file)
    end
  end

  def name!({:variable, name}, _), do: name
  def name!({:field, parent, name}, file), do: name!(parent, file) <> "." <> name

  def name!(_, file),
    do: Error.fail!("Only registered operations and builtins can be called", file: file)

  def identifier([{:word, name, _, _} | rest], _), do: {name, rest}
  def identifier(tokens, file), do: fail!("Expected identifier", tokens, file)

  def expect([token | rest] = tokens, expected, file) do
    if Lexer.text(token) == expected, do: rest, else: fail!("Expected #{expected}", tokens, file)
  end

  def expect([], expected, file), do: fail!("Expected #{expected}", [], file)
  def text([token | _]), do: Lexer.text(token)
  def text([]), do: nil

  def fail!(message, [token | _], file),
    do: Error.fail!(message, [file: file] ++ Lexer.position(token))

  def fail!(message, [], file), do: Error.fail!(message, file: file)
end
