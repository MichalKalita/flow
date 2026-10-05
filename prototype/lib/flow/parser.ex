defmodule Flow.Parser do
  @moduledoc """
  Parses bracketed UTF-8 declarations into an ordered, source-located AST.

  Nodes are maps with `kind`, `value` and a half-open `span`. Kinds are
  `:document`, `:list`, `:symbol`, `:string` and `:number`. Number values retain
  their exact source text. Identifiers never become Elixir atoms.

  Only syntax is checked: declarations, types, operators and their meanings
  belong to a later validator. No application code is evaluated.

  Options: `file`, `max_bytes` (4 MiB), `max_depth` (256), `max_nodes` (100,000).
  Limits must be positive integers. The document itself is not a counted node.
  Columns count Unicode code points; offsets count bytes. CRLF is one newline.
  """
  alias Flow.ParseError

  @number ~r/\A-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?\z/
  @escapes %{
    ?" => "\"",
    ?\\ => "\\",
    ?/ => "/",
    ?b => "\b",
    ?f => "\f",
    ?n => "\n",
    ?r => "\r",
    ?t => "\t"
  }

  @type position :: %{offset: non_neg_integer(), line: pos_integer(), column: pos_integer()}
  @type ast_node :: %{
          kind: :document | :list | :symbol | :string | :number,
          value: binary() | [ast_node()],
          span: %{start: position(), end: position()}
        }

  @spec parse(binary(), keyword()) :: {:ok, ast_node()} | {:error, Flow.ParseError.t()}
  def parse(source, options \\ []) when is_binary(source) do
    options =
      Keyword.validate!(options,
        file: "<source>",
        max_bytes: 4_194_304,
        max_depth: 256,
        max_nodes: 100_000
      )

    unless is_binary(options[:file]), do: raise(ArgumentError, "file must be a string")

    for key <- [:max_bytes, :max_depth, :max_nodes] do
      unless is_integer(options[key]) and options[key] > 0,
        do: raise(ArgumentError, "#{key} must be a positive integer")
    end

    state = %{
      rest: source,
      source: source,
      position: %{offset: 0, line: 1, column: 1},
      file: options[:file],
      max_depth: options[:max_depth],
      max_nodes: options[:max_nodes],
      frames: [],
      roots: [],
      depth: 0,
      nodes: 0
    }

    try do
      if byte_size(source) > options[:max_bytes],
        do: fail(state, :byte_limit, "Source exceeds max_bytes")

      {:ok, walk(state)}
    rescue
      error in ParseError -> {:error, error}
    end
  end

  @spec parse!(binary(), keyword()) :: ast_node()
  def parse!(source, options \\ []) do
    case parse(source, options) do
      {:ok, ast} -> ast
      {:error, error} -> raise error
    end
  end

  defp walk(%{rest: "", frames: []} = state) do
    node(:document, Enum.reverse(state.roots), %{offset: 0, line: 1, column: 1}, state)
  end

  defp walk(%{rest: "", frames: [{opened_at, _} | _]} = state),
    do: fail(state, :unclosed_list, "Expected ] before end of source", opened_at: opened_at)

  defp walk(%{rest: "\r\n" <> rest} = state),
    do: walk(newline(state, rest, 2))

  defp walk(%{rest: <<c, _::binary>>} = state) when c in [32, 9, 10, 13] do
    {_, state} = character(state)
    walk(state)
  end

  defp walk(%{rest: "#" <> _} = state), do: walk(comment(state))

  defp walk(%{rest: "[" <> _} = state) do
    if state.depth >= state.max_depth, do: fail(state, :depth_limit, "Source exceeds max_depth")
    state = reserve(state)
    frame = {state.position, []}
    walk(%{ascii(state, 1) | frames: [frame | state.frames], depth: state.depth + 1})
  end

  defp walk(%{rest: "]" <> _, frames: []} = state),
    do: fail(state, :unexpected_close, "Unexpected ]")

  defp walk(%{rest: "]" <> _, frames: [{start, values} | frames]} = state) do
    state = %{ascii(state, 1) | frames: frames, depth: state.depth - 1}
    walk(emit(state, node(:list, Enum.reverse(values), start, state)))
  end

  defp walk(%{frames: []} = state),
    do: fail(state, :expected_list, "A declaration must be enclosed in [ ]")

  defp walk(%{rest: "\"" <> _} = state) do
    state = reserve(state)
    start = state.position
    {value, state} = string(ascii(state, 1), start, [])
    boundary(state)
    walk(emit(state, node(:string, value, start, state)))
  end

  defp walk(state) do
    state = reserve(state)
    start = state.position
    state = symbol(state)
    boundary(state)
    raw = binary_part(state.source, start.offset, state.position.offset - start.offset)

    kind =
      if numeric?(raw) do
        unless Regex.match?(@number, raw),
          do: fail(state, :invalid_number, "Invalid number", at: start)

        :number
      else
        :symbol
      end

    walk(emit(state, node(kind, raw, start, state)))
  end

  defp symbol(%{rest: ""} = state), do: state

  defp symbol(%{rest: <<c, _::binary>>} = state) when c in [32, 9, 10, 13, ?[, ?], ?#, ?"],
    do: state

  defp symbol(state) do
    {c, next} = character(state)

    if c < 32 or c in 127..159,
      do: fail(state, :unexpected_character, "Control character outside string")

    symbol(next)
  end

  defp numeric?(<<c, _::binary>>) when c in ?0..?9, do: true
  defp numeric?(<<sign, c, _::binary>>) when sign in [?-, ?+] and c in ?0..?9, do: true
  defp numeric?(_), do: false

  defp boundary(%{rest: ""}), do: :ok
  defp boundary(%{rest: <<c, _::binary>>}) when c in [32, 9, 10, 13, ?[, ?], ?#], do: :ok

  defp boundary(state),
    do: fail(state, :missing_separator, "Expected whitespace or a bracket between values")

  defp string(%{rest: ""} = state, start, _),
    do: fail(state, :unclosed_string, "Expected closing quote", opened_at: start)

  defp string(%{rest: "\"" <> _} = state, _, parts),
    do: {parts |> Enum.reverse() |> IO.iodata_to_binary(), ascii(state, 1)}

  defp string(%{rest: "\\" <> _} = state, start, parts) do
    {value, next} = escape(ascii(state, 1), state.position)
    string(next, start, [value | parts])
  end

  defp string(state, start, parts) do
    {c, next} = character(state)
    if c < 32, do: fail(state, :invalid_string, "Control characters in strings must be escaped")
    string(next, start, [<<c::utf8>> | parts])
  end

  defp escape(%{rest: <<c, _::binary>>} = state, _) when is_map_key(@escapes, c),
    do: {Map.fetch!(@escapes, c), ascii(state, 1)}

  defp escape(%{rest: "u" <> _} = state, start) do
    {high, next} = hex(ascii(state, 1), start)

    cond do
      high in 0xD800..0xDBFF ->
        unless String.starts_with?(next.rest, "\\u"),
          do:
            fail(next, :invalid_escape, "Expected low surrogate after high surrogate", at: start)

        {low, next} = hex(ascii(next, 2), start)

        unless low in 0xDC00..0xDFFF,
          do: fail(next, :invalid_escape, "Invalid low surrogate", at: start)

        {<<0x10000 + (high - 0xD800) * 0x400 + low - 0xDC00::utf8>>, next}

      high in 0xDC00..0xDFFF ->
        fail(next, :invalid_escape, "Unexpected low surrogate", at: start)

      true ->
        {<<high::utf8>>, next}
    end
  end

  defp escape(state, start), do: fail(state, :invalid_escape, "Invalid string escape", at: start)

  defp hex(%{rest: <<digits::binary-size(4), _::binary>>} = state, start) do
    if Regex.match?(~r/\A[0-9a-fA-F]{4}\z/, digits),
      do: {String.to_integer(digits, 16), ascii(state, 4)},
      else: fail(state, :invalid_escape, "Expected four hexadecimal digits", at: start)
  end

  defp hex(state, start), do: fail(state, :invalid_escape, "Incomplete Unicode escape", at: start)

  defp comment(%{rest: ""} = state), do: state
  defp comment(%{rest: <<c, _::binary>>} = state) when c in [10, 13], do: state

  defp comment(state) do
    {c, next} = character(state)

    if (c < 32 and c != 9) or c in 127..159,
      do: fail(state, :unexpected_character, "Control character outside string")

    comment(next)
  end

  defp character(%{rest: <<c::utf8, rest::binary>>} = state) do
    bytes = byte_size(state.rest) - byte_size(rest)

    if c in [10, 13],
      do: {c, newline(state, rest, bytes)},
      else:
        {c,
         %{
           state
           | rest: rest,
             position: %{
               state.position
               | offset: state.position.offset + bytes,
                 column: state.position.column + 1
             }
         }}
  end

  defp character(state), do: fail(state, :invalid_utf8, "Invalid UTF-8")

  defp ascii(state, bytes) do
    %{
      state
      | rest: binary_part(state.rest, bytes, byte_size(state.rest) - bytes),
        position: %{
          state.position
          | offset: state.position.offset + bytes,
            column: state.position.column + bytes
        }
    }
  end

  defp newline(state, rest, bytes),
    do: %{
      state
      | rest: rest,
        position: %{
          offset: state.position.offset + bytes,
          line: state.position.line + 1,
          column: 1
        }
    }

  defp reserve(state) do
    if state.nodes >= state.max_nodes, do: fail(state, :node_limit, "Source exceeds max_nodes")
    %{state | nodes: state.nodes + 1}
  end

  defp emit(%{frames: []} = state, node), do: %{state | roots: [node | state.roots]}

  defp emit(%{frames: [{start, values} | frames]} = state, node),
    do: %{state | frames: [{start, [node | values]} | frames]}

  defp node(kind, value, start, state),
    do: %{kind: kind, value: value, span: %{start: start, end: state.position}}

  defp fail(state, code, message, options \\ []) do
    at = Keyword.get(options, :at, state.position)

    raise ParseError,
      code: code,
      message: message,
      file: state.file,
      offset: at.offset,
      line: at.line,
      column: at.column,
      opened_at: options[:opened_at]
  end
end
