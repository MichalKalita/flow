defmodule Flow.ParserTest do
  use ExUnit.Case, async: false
  alias Flow.{Parser, ParseError}

  test "AST is ordered, generic and source-located" do
    assert {:ok, %{kind: :document, value: [form], span: span}} = Parser.parse("[entity User]")
    assert span == %{start: pos(0, 1, 1), end: pos(13, 1, 14)}

    assert form == %{
             kind: :list,
             span: span,
             value: [
               %{kind: :symbol, value: "entity", span: %{start: pos(1, 1, 2), end: pos(7, 1, 8)}},
               %{kind: :symbol, value: "User", span: %{start: pos(8, 1, 9), end: pos(12, 1, 13)}}
             ]
           }
  end

  test "duplicate and unknown declarations remain in the AST for later validation" do
    ast = Parser.parse!("[unknown X][unknown X][type Price [decimal [range 5 1]]]")

    assert Enum.map(ast.value, &shape/1) == [
             {:list, [{:symbol, "unknown"}, {:symbol, "X"}]},
             {:list, [{:symbol, "unknown"}, {:symbol, "X"}]},
             {:list,
              [
                {:symbol, "type"},
                {:symbol, "Price"},
                {:list,
                 [
                   {:symbol, "decimal"},
                   {:list, [{:symbol, "range"}, {:number, "5"}, {:number, "1"}]}
                 ]}
              ]}
           ]
  end

  test "empty lists and empty documents are syntactically valid" do
    assert shape(Parser.parse!("")) == {:document, []}
    assert shape(Parser.parse!("# only a comment\r\n\t")) == {:document, []}
    assert shape(Parser.parse!("[][[]]")) == {:document, [{:list, []}, {:list, [{:list, []}]}]}
  end

  test "references, punctuation and operators are ordinary symbols" do
    ast = Parser.parse!("[$userId user.orders Post.UPDATE * + - true false null]")

    assert ast.value |> hd() |> Map.fetch!(:value) |> Enum.map(& &1.value) ==
             ["$userId", "user.orders", "Post.UPDATE", "*", "+", "-", "true", "false", "null"]

    assert Enum.all?(hd(ast.value).value, &(&1.kind == :symbol))
  end

  test "numbers retain exact spelling without float or integer overflow" do
    values = [
      "0",
      "-0",
      "0.01",
      "2490.00",
      "-90",
      "1e1000000000",
      "1E+9",
      "-1.250e-12",
      String.duplicate("9", 10_000)
    ]

    ast = Parser.parse!("[" <> Enum.join(values, " ") <> "]")
    assert Enum.map(hd(ast.value).value, &shape/1) == Enum.map(values, &{:number, &1})
  end

  for value <- ["01", "-01", "+1", "1.", "1e", "1e+", "1..2", "2^8", "42name"] do
    test "malformed number #{value} is a syntax error" do
      assert {:error, %ParseError{code: :invalid_number, offset: 1, line: 1, column: 2}} =
               Parser.parse("[" <> unquote(value) <> "]")
    end
  end

  test "strings decode JSON escapes and preserve literal UTF-8" do
    source =
      ~S(["\"" "\\" "\/" "\b" "\f" "\n" "\r" "\t" "\u0000" "\u017E" "\uD83D\uDE80" "Příliš žluťoučký 🚀"])

    assert Enum.map(hd(Parser.parse!(source).value).value, &shape/1) ==
             Enum.map(
               [
                 "\"",
                 "\\",
                 "/",
                 "\b",
                 "\f",
                 "\n",
                 "\r",
                 "\t",
                 <<0>>,
                 "ž",
                 "🚀",
                 "Příliš žluťoučký 🚀"
               ],
               &{:string, &1}
             )
  end

  test "quote, brackets and comment marker are inert inside strings" do
    assert shape(Parser.parse!(~S(["[] # [entity Evil] "]))) ==
             {:document, [{:list, [{:string, "[] # [entity Evil] "}]}]}
  end

  for escape <- [
        ~S(\x),
        ~S(\u),
        ~S(\u123),
        ~S(\u12xz),
        ~S(\uD800),
        ~S(\uD800\u1234),
        ~S(\uD800\uD800),
        ~S(\uDC00),
        ~S(\uDFFF)
      ] do
    test "rejects invalid escape #{escape}" do
      assert {:error, %ParseError{code: :invalid_escape, offset: 2}} =
               Parser.parse("[\"" <> unquote(escape) <> "\"]")
    end
  end

  test "incomplete string diagnostics contain opening and failure locations" do
    assert {:error, %ParseError{code: :unclosed_string, offset: 5, opened_at: opened}} =
             Parser.parse("[\"abc")

    assert opened == pos(1, 1, 2)
    assert {:error, %ParseError{code: :invalid_escape, offset: 3}} = Parser.parse("[\"a\\")
  end

  for control <- [0, 9, 10, 13, 31] do
    test "rejects unescaped control #{control} in strings" do
      assert {:error, %ParseError{code: :invalid_string, offset: 2}} =
               Parser.parse(<<"[\"", unquote(control), "\"]">>)
    end
  end

  test "rejects non-whitespace control characters outside strings" do
    for c <- [0, 11, 12, 31, 127] do
      assert {:error, %ParseError{code: :unexpected_character, offset: 1}} =
               Parser.parse(<<"[", c, "]">>)
    end

    assert {:error, %ParseError{code: :unexpected_character}} = Parser.parse("[\u0085]")
  end

  test "all UTF-8 scanning paths reject malformed bytes" do
    for source <- [<<"[", 255, "]">>, <<"[\"", 255, "\"]">>, <<"#", 255>>] do
      assert {:error, %ParseError{code: :invalid_utf8}} = Parser.parse(source)
    end
  end

  test "comments accept tabs but reject non-whitespace control characters" do
    assert {:ok, _} = Parser.parse("#\tcomment\n[]")

    for c <- [0, 11, 12, 31, 127] do
      assert {:error, %ParseError{code: :unexpected_character, offset: 1}} =
               Parser.parse(<<"#", c>>)
    end
  end

  test "Unicode escapes cover both ends of surrogate pairs and non-surrogate boundaries" do
    for {raw, codepoint} <- [
          {~S(\u0000), 0},
          {~S(\uD7FF), 0xD7FF},
          {~S(\uE000), 0xE000},
          {~S(\uFFFF), 0xFFFF},
          {~S(\ud800\udc00), 0x10000},
          {~S(\uDBFF\uDFFF), 0x10FFFF}
        ] do
      ast = Parser.parse!("[\"" <> raw <> "\"]")
      assert hd(hd(ast.value).value).value == <<codepoint::utf8>>
    end

    ast = Parser.parse!("[e\u0301]")
    assert hd(hd(ast.value).value).span == %{start: pos(1, 1, 2), end: pos(4, 1, 4)}
  end

  test "comments, tabs, LF, CRLF and CR update locations consistently" do
    ast = Parser.parse!("# ž🚀\r\n\t[é]\r[x]\n[]#tail")
    assert Enum.map(ast.value, & &1.span.start) == [pos(11, 2, 2), pos(16, 3, 1), pos(20, 4, 1)]
    assert hd(hd(ast.value).value).span == %{start: pos(12, 2, 3), end: pos(14, 2, 4)}
    assert ast.span.end == pos(27, 4, 8)
  end

  test "unmatched brackets and EOF have structured errors" do
    assert {:error, %ParseError{code: :unexpected_close, line: 1, column: 1}} = Parser.parse("]")

    assert {:error, %ParseError{code: :unclosed_list, line: 2, column: 1, opened_at: opened}} =
             Parser.parse("[a\n")

    assert opened == pos(0, 1, 1)
    assert {:error, %ParseError{code: :unclosed_list, opened_at: inner}} = Parser.parse("[[a]")
    assert inner == pos(0, 1, 1)
    assert {:error, %ParseError{code: :unclosed_list, opened_at: inner}} = Parser.parse("[[][")
    assert inner == pos(3, 1, 4)
  end

  test "declarations require brackets and adjacent scalar literals require separators" do
    for source <- ["User", "\"User\"", "42", "[]garbage"] do
      assert {:error, %ParseError{code: :expected_list}} = Parser.parse(source)
    end

    for source <- [~S(["a""b"]), ~S(["a"b]), ~S([a"b"])] do
      assert {:error, %ParseError{code: :missing_separator}} = Parser.parse(source)
    end

    assert {:ok, _} = Parser.parse(~S([a[b]"c"][]))
  end

  test "file identity is carried in errors and bang API raises the same error" do
    assert {:error, %ParseError{file: "app.flow", code: :unclosed_list}} =
             Parser.parse("[", file: "app.flow")

    assert_raise ParseError, "Expected ] before end of source", fn -> Parser.parse!("[") end
    assert Parser.parse!("[]") == elem(Parser.parse("[]"), 1)
  end

  test "byte, depth and node limits have exact boundaries" do
    assert {:ok, _} = Parser.parse("[é]", max_bytes: 4)
    assert {:error, %ParseError{code: :byte_limit}} = Parser.parse("[é]", max_bytes: 3)
    assert {:ok, _} = Parser.parse("[[a]]", max_depth: 2)

    assert {:error, %ParseError{code: :depth_limit, offset: 1}} =
             Parser.parse("[[a]]", max_depth: 1)

    assert {:ok, _} = Parser.parse("[a][b]", max_nodes: 4)

    assert {:error, %ParseError{code: :node_limit, offset: 4}} =
             Parser.parse("[a][b]", max_nodes: 3)

    assert {:ok, _} = Parser.parse("[]", max_nodes: 1)
  end

  test "option errors are programmer errors, not source diagnostics" do
    for key <- [:max_bytes, :max_depth, :max_nodes], value <- [0, -1, nil, 1.5, "1"] do
      assert_raise ArgumentError, fn -> Parser.parse("[]", [{key, value}]) end
    end

    assert_raise ArgumentError, fn -> Parser.parse("[]", file: 123) end
    assert_raise ArgumentError, fn -> Parser.parse("[]", unknown: true) end
  end

  test "wide and deeply nested documents use bounded explicit state" do
    source = "[" <> String.duplicate("value ", 20_000) <> "]"
    assert length(hd(Parser.parse!(source).value).value) == 20_000
    source = String.duplicate("[", 2000) <> String.duplicate("]", 2000)
    assert {:error, %ParseError{code: :depth_limit, offset: 256}} = Parser.parse(source)
    assert {:ok, ast} = Parser.parse(source, max_depth: 2000)
    assert ast.span.end.offset == 4000
    # Also exercise the long-string scanner independently of the list scanner.
    text = String.duplicate("ž🚀", 20_000)
    assert hd(hd(Parser.parse!("[\"" <> text <> "\"]").value).value).value == text
  end

  test "untrusted symbols never allocate atoms" do
    Parser.parse!("[warmup 1 \"string\"]")
    sources = for n <- 1..2000, do: "[untrusted_identifier_#{n}]"
    before = :erlang.system_info(:atom_count)
    Enum.each(sources, &Parser.parse!/1)
    assert :erlang.system_info(:atom_count) == before
  end

  test "generated trees round-trip without knowing any language keywords" do
    :rand.seed(:exsss, {11, 22, 33})

    for _ <- 1..500 do
      expected = {:list, Enum.map(1..:rand.uniform(4), fn _ -> tree(4) end)}
      assert shape(Parser.parse!(render(expected))) == {:document, [expected]}
    end
  end

  test "arbitrary bytes produce a result or a structured error, never an implementation exception" do
    :rand.seed(:exsss, {44, 55, 66})

    for _ <- 1..5000 do
      source = for _ <- 1..:rand.uniform(80), into: "", do: <<:rand.uniform(256) - 1>>

      for input <- [source, "[" <> source <> "]", "[\"" <> source <> "\"]", "#" <> source] do
        case Parser.parse(input) do
          {:ok, %{kind: :document}} ->
            :ok

          {:error, %ParseError{offset: offset, line: line, column: column}} ->
            assert offset >= 0 and offset <= byte_size(input)
            assert line >= 1 and column >= 1
        end
      end
    end
  end

  test "full application and permissions examples parse through the public API" do
    for file <- ["priv/workflows/application.flow", "../examples/permissions.flow"] do
      source = File.read!(file)
      assert {:ok, ast} = Parser.parse(source, file: file)
      assert ast.span.end.offset == byte_size(source)
      assert Enum.all?(ast.value, &(&1.kind == :list))
      assert Enum.all?(ast.value, &(hd(&1.value).kind == :symbol))
    end
  end

  defp pos(offset, line, column), do: %{offset: offset, line: line, column: column}

  defp shape(%{kind: kind, value: value}) when kind in [:list, :document],
    do: {kind, Enum.map(value, &shape/1)}

  defp shape(%{kind: kind, value: value}), do: {kind, value}

  defp tree(depth) do
    case :rand.uniform(if depth > 0, do: 4, else: 3) do
      1 ->
        {:symbol, Enum.random(["future", "User", "actor", "user.orders", "$id", "*", "ž"])}

      2 ->
        {:number, Enum.random(["0", "0.01", "-180", "1e1000", "999999999999999999999999"])}

      3 ->
        {:string, Enum.random(["plain", "Příliš 🚀", "[] # text"])}

      4 ->
        {:list, Enum.map(List.duplicate(nil, :rand.uniform(5) - 1), fn _ -> tree(depth - 1) end)}
    end
  end

  defp render({:list, values}), do: "[" <> Enum.map_join(values, " ", &render/1) <> "]"
  defp render({:string, value}), do: "\"" <> value <> "\""
  defp render({_, value}), do: value
end
