defmodule OrderLab.Language.Parser do
  alias OrderLab.Language.{Lexer, Expression, Error}
  alias OrderLab.Language.Expression, as: E

  def parse!(source, file \\ "<source>") do
    program = %{
      types: %{},
      tables: %{},
      seeds: [],
      mqtt: %{},
      websockets: [],
      endpoints: [],
      source: source,
      file: file
    }

    top(Lexer.lines(source, file), program)
  end

  def type!(tokens, file) do
    {type, rest} = type(tokens, file)
    unless rest == [], do: E.fail!("Unexpected token in type", rest, file)
    type
  end

  defp top([], program), do: program

  defp top([%{indent: indent} = line | _], program) when indent != 0,
    do: error!("Top-level declarations must not be indented", line, program.file)

  defp top([line | rest], program) do
    case line.tokens do
      [{:word, "TYPE", _, _}, {:word, name, _, _}, {:symbol, "=", _, _} | tokens] ->
        if Map.has_key?(program.types, name),
          do: error!("Duplicate type #{name}", line, program.file)

        {type, tail} = type(tokens, program.file)

        type =
          case tail do
            [] ->
              type

            [{:word, "WHERE", _, _} | tokens] ->
              {:refined, type, expression!(tokens, line, program.file)}

            _ ->
              error!("Expected WHERE after base type", line, program.file)
          end

        top(rest, %{program | types: Map.put(program.types, name, type)})

      [{:word, "FILTER", _, _}, {:word, name, _, _}] ->
        {fields, rest} = children(rest, line, program.file)

        fields =
          Enum.map(fields, fn field ->
            {name, tokens} = E.identifier(field.tokens, program.file)
            {name, type!(tokens, program.file)}
          end)

        if length(Enum.uniq_by(fields, &elem(&1, 0))) != length(fields),
          do: error!("Duplicate filter field", line, program.file)

        if Map.has_key?(program.types, name),
          do: error!("Duplicate type #{name}", line, program.file)

        top(rest, %{program | types: Map.put(program.types, name, {:record, Map.new(fields)})})

      [{:word, "TABLE", _, _}, {:word, name, _, _}, {:symbol, "=", _, _} | tokens] ->
        if Map.has_key?(program.tables, name),
          do: error!("Duplicate table #{name}", line, program.file)

        top(rest, %{program | tables: Map.put(program.tables, name, type!(tokens, program.file))})

      [{:word, "ON", _, _}, {:word, "MQTT", _, _}, {:word, name, _, _}] ->
        {body, rest} = child_block(rest, line, program.file)

        endpoint = %{
          method: "MQTT",
          path: name,
          inputs: [],
          body: body,
          line: line.line,
          file: program.file
        }

        top(rest, %{program | endpoints: program.endpoints ++ [endpoint]})

      [{:word, "MQTT", _, _}, {:word, name, _, _}] ->
        if Map.has_key?(program.mqtt, name),
          do: error!("Duplicate MQTT source #{name}", line, program.file)

        {clauses, rest} = children(rest, line, program.file)

        source =
          Enum.reduce(
            clauses,
            %{name: name, params: [], topic: nil, payload: nil, history_ms: 300_000},
            fn clause, source ->
              case clause.tokens do
                [{:word, "TOPIC", _, _}, {:string, topic, _, _}] ->
                  if source.topic, do: error!("Duplicate MQTT TOPIC", clause, program.file)
                  %{source | topic: topic}

                [{:word, "PARAM", _, _}, {:word, param, _, _} | tokens] ->
                  if Enum.any?(source.params, &(&1.name == param)),
                    do: error!("Duplicate MQTT parameter", clause, program.file)

                  %{
                    source
                    | params: source.params ++ [%{name: param, type: type!(tokens, program.file)}]
                  }

                [{:word, "PAYLOAD", _, _} | tokens] ->
                  if source.payload, do: error!("Duplicate MQTT PAYLOAD", clause, program.file)
                  %{source | payload: type!(tokens, program.file)}

                [{:word, "HISTORY", _, _}, {:number, duration, _, _}, {:word, unit, _, _}]
                when unit in ["ms", "SECONDS", "MINUTES", "HOURS"] ->
                  unless is_integer(duration) and duration > 0,
                    do: error!("MQTT HISTORY must be positive", clause, program.file)

                  %{
                    source
                    | history_ms:
                        duration *
                          %{
                            "ms" => 1,
                            "SECONDS" => 1000,
                            "MINUTES" => 60_000,
                            "HOURS" => 3_600_000
                          }[unit]
                  }

                _ ->
                  error!("Expected TOPIC, PARAM, PAYLOAD or HISTORY", clause, program.file)
              end
            end
          )

        unless source.topic && source.payload,
          do: error!("MQTT requires TOPIC and PAYLOAD", line, program.file)

        top(rest, %{program | mqtt: Map.put(program.mqtt, name, source)})

      [{:word, "SEED", _, _}, {:word, name, _, _}, {:word, "WITH", _, _} | tokens] ->
        value = expression!(tokens, line, program.file)
        top(rest, %{program | seeds: program.seeds ++ [%{table: name, value: value}]})

      [{:word, "WEBSOCKET", _, _} | tokens] ->
        route = Enum.map_join(tokens, fn {_, value, _, _} -> to_string(value) end)
        {clauses, rest} = children(rest, line, program.file)

        {credentials, clauses} = inputs(clauses, program.file, [])

        endpoint = %{
          path: route,
          sources: [],
          policies: %{},
          inputs: credentials,
          authorization: {:literal, true},
          file: program.file,
          line: line.line
        }

        {endpoint, _} =
          Enum.reduce(clauses, {endpoint, false}, fn clause, {endpoint, authorized} ->
            case clause.tokens do
              [{:word, "AUTHORIZE", _, _} | tokens] ->
                if authorized, do: error!("Duplicate WebSocket AUTHORIZE", clause, program.file)
                {%{endpoint | authorization: expression!(tokens, clause, program.file)}, true}

              [{:word, "SOURCE", _, _}, {:word, name, _, _} | tokens] ->
                policy =
                  case tokens do
                    [] -> {:literal, true}
                    [{:word, "WHERE", _, _} | tokens] -> expression!(tokens, clause, program.file)
                    _ -> error!("Expected WHERE after WebSocket SOURCE", clause, program.file)
                  end

                {%{
                   endpoint
                   | sources: endpoint.sources ++ [name],
                     policies: Map.put(endpoint.policies, name, policy)
                 }, authorized}

              _ ->
                error!(
                  "WebSocket expects INPUT, AUTHORIZE or SOURCE name WHERE expression",
                  clause,
                  program.file
                )
            end
          end)

        top(rest, %{program | websockets: program.websockets ++ [endpoint]})

      [{:word, "HTTP", _, _}, {:word, method, _, _} | route_tokens] ->
        if method not in ["GET", "POST", "PUT", "PATCH", "DELETE"],
          do: error!("Unsupported HTTP method", line, program.file)

        route = Enum.map_join(route_tokens, fn {_, value, _, _} -> to_string(value) end)

        unless String.starts_with?(route, "/"),
          do: error!("HTTP path must start with /", line, program.file)

        {body, rest} =
          Enum.split_while(rest, fn next ->
            next.indent > 0 or
              E.text(next.tokens) not in [
                "HTTP",
                "MQTT",
                "ON",
                "WEBSOCKET",
                "SEED",
                "TYPE",
                "FILTER",
                "TABLE"
              ]
          end)

        {inputs, body} = inputs(body, program.file, [])
        statements = statements(body, program.file)

        endpoint = %{
          method: method,
          path: route,
          inputs: inputs,
          body: statements,
          line: line.line,
          file: program.file
        }

        top(rest, %{program | endpoints: program.endpoints ++ [endpoint]})

      _ ->
        error!("Expected TYPE, FILTER, TABLE, MQTT or HTTP declaration", line, program.file)
    end
  end

  defp inputs(
         [%{tokens: [{:word, "INPUT", _, _}, {:word, name, _, _} | tokens]} = line | rest],
         file,
         acc
       ) do
    {type, tail} = type(tokens, file)

    default =
      case tail do
        [] -> :missing
        [{:symbol, "=", _, _} | tokens] -> expression!(tokens, line, file)
        _ -> error!("Expected input type or default value", line, file)
      end

    if Enum.any?(acc, &(&1.name == name)), do: error!("Duplicate input #{name}", line, file)
    inputs(rest, file, acc ++ [%{name: name, type: type, default: default, line: line.line}])
  end

  defp inputs(rest, _, acc), do: {acc, rest}

  defp statements([], _), do: []

  defp statements(lines, file) do
    indent = hd(lines).indent
    {statements, remaining} = block(lines, indent, file, [])
    unless remaining == [], do: error!("Unexpected indentation", hd(remaining), file)
    statements
  end

  defp block([], _, _, acc), do: {Enum.reverse(acc), []}

  defp block([line | _] = lines, indent, _, acc) when line.indent < indent,
    do: {Enum.reverse(acc), lines}

  defp block([line | rest], indent, file, acc) do
    if line.indent != indent, do: error!("Unexpected indentation", line, file)
    {node, rest} = statement(line, rest, file)
    block(rest, indent, file, [Map.merge(node, %{line: line.line, file: file}) | acc])
  end

  defp statement(%{tokens: [{:word, "TRANSACTION", _, _}]} = line, rest, file) do
    {body, rest} = child_block(rest, line, file)
    {%{kind: :transaction, body: body}, rest}
  end

  defp statement(%{tokens: [{:word, "COMMIT", _, _}]}, rest, _), do: {%{kind: :commit}, rest}

  defp statement(%{tokens: [{:word, keyword, _, _} | tokens]} = line, rest, file)
       when keyword in ["WHEN", "IF"] do
    condition = expression!(tokens, line, file)
    {body, rest} = child_block(rest, line, file)

    {otherwise, rest} =
      case rest do
        [%{indent: indent, tokens: [{:word, "ELSE", _, _}]} = else_line | tail]
        when indent == line.indent ->
          child_block(tail, else_line, file)

        _ ->
          {[], rest}
      end

    {%{kind: :when, condition: condition, body: body, otherwise: otherwise}, rest}
  end

  defp statement(%{tokens: [{:word, "TRY", _, _}]} = line, rest, file) do
    {body, rest} = child_block(rest, line, file)

    case rest do
      [
        %{indent: indent, tokens: [{:word, "CATCH", _, _}, {:word, name, _, _}]} = catch_line
        | tail
      ]
      when indent == line.indent ->
        {handler, rest} = child_block(tail, catch_line, file)
        {%{kind: :try, body: body, error_name: name, handler: handler}, rest}

      _ ->
        error!("TRY requires CATCH error", line, file)
    end
  end

  defp statement(%{tokens: [{:word, "REQUIRE", _, _} | tokens]} = line, rest, file) do
    {condition, tail} = E.parse(tokens, file)
    {status, code, message} = failure_spec(E.expect(tail, "ELSE", file), line, file)
    {%{kind: :require, condition: condition, status: status, code: code, message: message}, rest}
  end

  defp statement(%{tokens: [{:word, "FAIL", _, _} | tokens]} = line, rest, file) do
    {status, code, message} = failure_spec(tokens, line, file)
    {%{kind: :fail, status: status, code: code, message: message}, rest}
  end

  defp statement(%{tokens: [{:word, "RETURN", _, _} | tokens]} = line, rest, file),
    do: {%{kind: :return, value: expression!(tokens, line, file), status: 200}, rest}

  defp statement(
         %{
           tokens: [
             {:word, "RESPONSE", _, _},
             {:number, status, _, _},
             {:word, "WITH", _, _} | tokens
           ]
         } = line,
         rest,
         file
       ),
       do: {%{kind: :return, value: expression!(tokens, line, file), status: status}, rest}

  defp statement(
         %{tokens: [{:word, "FOR", _, _}, {:word, "EACH", _, _} | tokens]} = line,
         rest,
         file
       ),
       do: iteration(tokens, nil, line, rest, file)

  defp statement(
         %{
           tokens: [
             {:word, name, _, _},
             {:symbol, "=", _, _},
             {:word, "FOR", _, _},
             {:word, "EACH", _, _} | tokens
           ]
         } = line,
         rest,
         file
       ),
       do: iteration(tokens, name, line, rest, file)

  defp statement(
         %{tokens: [name_token = {:word, _, _, _}, {:symbol, ":", _, _} | tokens]} = line,
         rest,
         file
       ) do
    {annotation, tail} = type(tokens, file)
    tail = E.expect(tail, "=", file)

    {node, rest} =
      statement(%{line | tokens: [name_token, {:symbol, "=", line.line, 1} | tail]}, rest, file)

    {Map.put(node, :annotation, annotation), rest}
  end

  defp statement(
         %{tokens: [{:word, name, _, _}, {:symbol, "=", _, _} | tokens]} = line,
         rest,
         file
       ),
       do: operation(tokens, name, line, rest, file)

  defp statement(line, rest, file), do: operation(line.tokens, nil, line, rest, file)

  defp operation([{:word, "PUBLISH", _, _} | tokens], binding, line, rest, file) do
    {target, tokens} = E.parse(tokens, file)

    {source, parameters} =
      case target do
        {:builtin, name, args} -> {name, args}
        _ -> error!("PUBLISH expects a typed MQTT source invocation", line, file)
      end

    {value, tokens} = E.parse(E.expect(tokens, "WITH", file), file)

    {retain, tokens} =
      case tokens do
        [{:word, "RETAIN", _, _} | tokens] -> E.parse(tokens, file)
        _ -> {{:literal, false}, tokens}
      end

    {binding, tokens} = optional_as(tokens, binding, file)
    unless tokens == [], do: error!("Unexpected PUBLISH suffix", line, file)

    {%{
       kind: :publish,
       source: source,
       parameters: parameters,
       value: value,
       retain: retain,
       binding: binding
     }, rest}
  end

  defp operation([{:word, "CALL", _, _} | tokens], binding, line, rest, file) do
    {name, tokens} = operation_name(tokens, file)
    {input, tokens} = E.parse(E.expect(tokens, "WITH", file), file)
    {binding, tokens} = optional_as(tokens, binding, file)
    unless tokens == [], do: error!("Unexpected CALL suffix", line, file)
    {%{kind: :call, operation: name, input: input, binding: binding}, rest}
  end

  defp operation([{:word, "QUEUE", _, _} | tokens], binding, line, rest, file) do
    {name, tokens} = operation_name(tokens, file)
    {input, tokens} = E.parse(E.expect(tokens, "WITH", file), file)
    tokens = E.expect(tokens, "POLICY", file)
    [{:number, attempts, _, _} | tokens] = tokens
    tokens = E.expect(tokens, "ATTEMPTS", file) |> E.expect("DELAY", file)
    [{:number, delay, _, _} | tokens] = tokens
    tokens = E.expect(tokens, "ms", file)
    {disposition, tokens} = E.identifier(tokens, file)

    unless disposition in ["RETAIN", "DELETE"],
      do: error!("Queue disposition must be RETAIN or DELETE", line, file)

    {binding, tokens} = optional_as(tokens, binding, file)
    unless tokens == [], do: error!("Unexpected QUEUE suffix", line, file)

    {%{
       kind: :queue,
       operation: name,
       input: input,
       binding: binding,
       attempts: attempts,
       delay: delay,
       disposition: disposition
     }, rest}
  end

  defp operation(
         [{:word, "INSERT", _, _}, {:word, table, _, _}, {:word, "WITH", _, _} | tokens],
         binding,
         line,
         rest,
         file
       ) do
    {value, tokens} = E.parse(tokens, file)
    {binding, tokens} = optional_as(tokens, binding, file)
    unless tokens == [], do: error!("Unexpected INSERT suffix", line, file)
    {%{kind: :insert, table: table, value: value, binding: binding}, rest}
  end

  defp operation([{:word, "UPDATE", _, _} | tokens], binding, line, rest, file) do
    {table, alias_name, condition, tokens} = mutation_prefix(tokens, file)
    {value, tokens} = E.parse(E.expect(tokens, "SET", file), file)
    {binding, tokens} = optional_as(tokens, binding, file)
    unless tokens == [], do: error!("Unexpected UPDATE suffix", line, file)

    {%{
       kind: :update,
       table: table,
       alias: alias_name,
       condition: condition,
       value: value,
       binding: binding
     }, rest}
  end

  defp operation(
         [{:word, "DELETE", _, _}, {:word, "FROM", _, _} | tokens],
         binding,
         line,
         rest,
         file
       ) do
    {table, alias_name, condition, tokens} = mutation_prefix(tokens, file)
    {binding, tokens} = optional_as(tokens, binding, file)
    unless tokens == [], do: error!("Unexpected DELETE suffix", line, file)

    {%{kind: :delete, table: table, alias: alias_name, condition: condition, binding: binding},
     rest}
  end

  defp operation(tokens, binding, line, rest, file) when not is_nil(binding) do
    {value, tail} = E.parse(tokens, file)
    unless tail == [], do: error!("Unexpected expression suffix", line, file)

    if match?({:query, _}, value) and rest != [] and hd(rest).indent > line.indent do
      {clauses, rest} = children(rest, line, file)
      {:query, query} = value
      query = query_lines(clauses, query, file)
      {%{kind: :let, name: binding, value: {:query, query}}, rest}
    else
      {%{kind: :let, name: binding, value: value}, rest}
    end
  end

  defp operation(_, _, line, _, file), do: error!("Unknown statement", line, file)

  defp iteration(tokens, binding, line, rest, file) do
    {name, tokens} = E.identifier(tokens, file)
    value = E.expect(tokens, "IN", file) |> expression!(line, file)
    {body, rest} = child_block(rest, line, file)
    {%{kind: :each, name: name, source: value, binding: binding, body: body}, rest}
  end

  defp query_lines([], query, _), do: query

  defp query_lines([line | rest], query, file) do
    case line.tokens do
      [{:word, kind, _, _} | _] when kind in ["JOIN", "INNER", "LEFT"] ->
        {join, tail} = E.join_clause(line.tokens, file)
        unless tail == [], do: error!("Unexpected JOIN suffix", line, file)
        query_lines(rest, %{query | joins: query.joins ++ [join]}, file)

      [{:word, "WHERE", _, _} | tokens] ->
        query_lines(
          rest,
          %{query | where: query.where ++ [expression!(tokens, line, file)]},
          file
        )

      [{:word, "SELECT", _, _} | tokens] ->
        if query.select, do: error!("Duplicate SELECT", line, file)
        query_lines(rest, %{query | select: expression!(tokens, line, file)}, file)

      [{:word, "ORDER", _, _}, {:word, "BY", _, _} | tokens] ->
        {key, tail} = E.parse(tokens, file)

        {direction, tail} =
          if E.text(tail) in ["ASC", "DESC"], do: {E.text(tail), tl(tail)}, else: {"ASC", tail}

        unless tail == [], do: error!("Unexpected ORDER BY suffix", line, file)
        query_lines(rest, %{query | order: query.order ++ [{key, direction}]}, file)

      [{:word, "LIMIT", _, _} | tokens] ->
        query_lines(rest, %{query | limit: expression!(tokens, line, file)}, file)

      [{:word, "WHEN", _, _} | tokens] ->
        condition = expression!(tokens, line, file)
        {children, rest} = children(rest, line, file)

        query =
          Enum.reduce(children, query, fn child, query ->
            case child.tokens do
              [{:word, "WHERE", _, _} | tokens] ->
                %{
                  query
                  | where: query.where ++ [{:guard, condition, expression!(tokens, child, file)}]
                }

              [{:word, kind, _, _} | _] when kind in ["JOIN", "INNER", "LEFT"] ->
                {join, tail} = E.join_clause(child.tokens, file)
                unless tail == [], do: error!("Unexpected JOIN suffix", child, file)
                %{query | joins: query.joins ++ [%{join | guard: condition}]}

              _ ->
                error!("Conditional query clause must be WHERE or JOIN", child, file)
            end
          end)

        query_lines(rest, query, file)

      _ ->
        error!("Expected JOIN, WHERE, SELECT, ORDER BY, LIMIT or WHEN query clause", line, file)
    end
  end

  defp mutation_prefix(tokens, file) do
    {table, tokens} = E.identifier(tokens, file)
    {alias_name, tokens} = E.expect(tokens, "AS", file) |> E.identifier(file)
    {condition, tokens} = E.expect(tokens, "WHERE", file) |> E.parse(file)
    {table, alias_name, condition, tokens}
  end

  defp failure_spec([{:number, status, _, _}, {:word, code, _, _} | tokens], line, file),
    do: {status, code, expression!(tokens, line, file)}

  defp failure_spec(tokens, line, file) do
    {code, tokens} = E.identifier(tokens, file)
    {422, code, expression!(tokens, line, file)}
  end

  defp operation_name(tokens, file) do
    {name, rest} = E.identifier(tokens, file)
    dotted_name(name, rest, file)
  end

  defp dotted_name(name, [{:symbol, ".", _, _}, {:word, suffix, _, _} | rest], file),
    do: dotted_name(name <> "." <> suffix, rest, file)

  defp dotted_name(name, rest, _), do: {name, rest}
  defp optional_as([{:word, "AS", _, _} | tokens], nil, file), do: E.identifier(tokens, file)
  defp optional_as(tokens, binding, _), do: {binding, tokens}

  defp child_block(rest, line, file) do
    {lines, rest} = children(rest, line, file)
    {statements(lines, file), rest}
  end

  defp children([child | _] = rest, line, file) when child.indent > line.indent do
    {children, rest} = Enum.split_while(rest, &(&1.indent > line.indent))
    if children == [], do: error!("Expected indented block", line, file)
    {children, rest}
  end

  defp children(_, line, file), do: error!("Expected indented block", line, file)

  defp expression!(tokens, line, file) do
    try do
      Expression.parse!(tokens, file)
    rescue
      error in Error ->
        reraise %{error | line: if(error.line == 1, do: line.line, else: error.line)},
                __STACKTRACE__
    end
  end

  defp type([{:symbol, "{", _, _} | tokens], file), do: record_type(tokens, file, %{})

  defp type([{:word, collection, _, _}, {:symbol, "<", _, _} | tokens], file)
       when collection in ["List", "Set"] do
    {element, rest} = type(tokens, file)
    rest = E.expect(rest, ">", file)
    optional({:list, element}, rest)
  end

  defp type(tokens, file) do
    {name, rest} = operation_name(tokens, file)
    optional({:named, name}, rest)
  end

  defp record_type([{:symbol, "}", _, _} | rest], _, fields),
    do: optional({:record, fields}, rest)

  defp record_type(tokens, file, fields) do
    {name, tokens} = E.identifier(tokens, file)
    if Map.has_key?(fields, name), do: E.fail!("Duplicate field #{name}", tokens, file)
    {type, tokens} = E.expect(tokens, ":", file) |> type(file)
    fields = Map.put(fields, name, type)

    case E.text(tokens) do
      "}" -> optional({:record, fields}, tl(tokens))
      "," -> record_type(tl(tokens), file, fields)
      _ -> E.fail!("Expected comma or closing brace in type", tokens, file)
    end
  end

  defp optional(type, [{:symbol, "?", _, _} | rest]), do: {{:optional, type}, rest}
  defp optional(type, rest), do: {type, rest}
  defp error!(message, line, file), do: Error.fail!(message, file: file, line: line.line)
end
