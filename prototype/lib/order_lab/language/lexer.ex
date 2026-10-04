defmodule OrderLab.Language.Lexer do
  alias OrderLab.Language.Error

  def lines(source, file) do
    {lines, pending, depth} =
      source
      |> String.split("\n")
      |> Enum.with_index(1)
      |> Enum.reduce({[], nil, 0}, fn {line, number}, {lines, pending, depth} ->
        if String.contains?(line, "\t"),
          do: Error.fail!("Tabs are not supported; use spaces", file: file, line: number)

        indent = byte_size(line) - byte_size(String.trim_leading(line))
        tokens = scan(String.trim_leading(line), number, indent + 1, file, [])

        if tokens == [] do
          {lines, pending, depth}
        else
          entry = pending || %{indent: indent, line: number, tokens: []}
          entry = %{entry | tokens: entry.tokens ++ tokens}

          depth =
            Enum.reduce(tokens, depth, fn token, acc ->
              case text(token) do
                x when x in ["{", "[", "("] -> acc + 1
                x when x in ["}", "]", ")"] -> acc - 1
                _ -> acc
              end
            end)

          if depth < 0, do: Error.fail!("Unexpected closing delimiter", file: file, line: number)
          if depth == 0, do: {lines ++ [entry], nil, 0}, else: {lines, entry, depth}
        end
      end)

    if pending || depth != 0,
      do: Error.fail!("Unclosed delimiter", file: file, line: (pending || %{line: 1}).line)

    lines
  end

  def text({_, value, _, _}) when is_binary(value), do: value
  def text(_), do: nil
  def position({_, _, line, column}), do: [line: line, column: column]

  defp scan("", _, _, _, tokens), do: Enum.reverse(tokens)
  defp scan("#" <> _, _, _, _, tokens), do: Enum.reverse(tokens)

  defp scan(<<c, rest::binary>>, line, col, file, tokens) when c in [32, 13],
    do: scan(rest, line, col + 1, file, tokens)

  defp scan(input, line, col, file, tokens) do
    cond do
      match = Regex.run(~r/^"(?:[^"\\]|\\.)*"/u, input) ->
        [raw] = match

        case Jason.decode(raw) do
          {:ok, value} -> next(input, raw, {:string, value, line, col}, line, col, file, tokens)
          _ -> Error.fail!("Invalid string escape", file: file, line: line, column: col)
        end

      match = Regex.run(~r/^\d+(?:\.\d+)?/, input) ->
        [raw] = match

        value =
          if String.contains?(raw, "."), do: String.to_float(raw), else: String.to_integer(raw)

        next(input, raw, {:number, value, line, col}, line, col, file, tokens)

      match = Regex.run(~r/^[A-Za-z_][A-Za-z_0-9]*/, input) ->
        [raw] = match
        next(input, raw, {:word, raw, line, col}, line, col, file, tokens)

      match = Regex.run(~r/^(?:>=|<=|!=|==|[{}\[\]().,:?+*\/%<>=!\-])/, input) ->
        [raw] = match
        next(input, raw, {:symbol, raw, line, col}, line, col, file, tokens)

      true ->
        Error.fail!("Unexpected character #{inspect(String.first(input))}",
          file: file,
          line: line,
          column: col
        )
    end
  end

  defp next(input, raw, token, line, col, file, tokens) do
    rest = binary_part(input, byte_size(raw), byte_size(input) - byte_size(raw))
    scan(rest, line, col + String.length(raw), file, [token | tokens])
  end
end
