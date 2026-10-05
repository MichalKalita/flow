defmodule Flow.Types do
  @moduledoc "Exact, bounded data types shared by compilation and runtime validation."
  alias Flow.Syntax, as: S

  @builtins ["String", "Bool", "DateTime"]
  def builtins, do: @builtins

  def compile(node, names, options \\ [])

  def compile(%{kind: :symbol} = node, names, _options) do
    name = S.symbol(node)

    unless name in @builtins or MapSet.member?(names, name),
      do: S.fail(node, :unknown_type, "Unknown type #{name}")

    {:named, name}
  end

  def compile(node, names, compile_options) do
    case S.form(node) do
      {"id", [entity]} ->
        {:id, S.identifier(entity)}

      {"optional", [inner]} ->
        {:optional, compile(inner, names)}

      {"list", [inner | opts]} ->
        options = S.options(opts, ["min", "max"])
        min = bound(options, "min", 0)
        max = bound(options, "max", nil)

        if max == nil and not Keyword.get(compile_options, :relation, false),
          do: S.fail(node, :missing_bound, "Stored and input/output lists require max")

        if min < 0 or (max != nil and (max < min or max < 0)),
          do: S.fail(node, :invalid_bounds, "Invalid list bounds")

        {:list, compile(inner, names), min, max}

      {kind, opts} when kind in ["integer", "decimal"] ->
        options = S.options(opts, if(kind == "integer", do: ["range"], else: ["range", "scale"]))

        range =
          Map.get(options, "range") ||
            S.fail(node, :missing_range, "Numbers require a finite range")

        [lo, hi] = S.args(range, "range", 2)
        lo = constant(lo)
        hi = constant(hi)

        if Decimal.compare(lo, hi) == :gt,
          do: S.fail(range, :invalid_range, "Range minimum exceeds maximum")

        if kind == "integer" and not (integral?(lo) and integral?(hi)),
          do: S.fail(range, :invalid_range, "Integer bounds must be integers")

        scale = if kind == "decimal", do: bound(options, "scale", nil), else: 0

        if scale == nil or scale < 0 or scale > 100,
          do: S.fail(node, :invalid_scale, "Decimals require a scale between 0 and 100")

        {:number, kind, lo, hi, scale}

      {"record", fields} ->
        {:record, fields(fields, names)}

      {"image", opts} ->
        options = S.options(opts, ["maxBytes", "maxWidth", "maxHeight"])

        limits =
          Map.new(["maxBytes", "maxWidth", "maxHeight"], fn key ->
            value = bound(options, key, nil)

            if value == nil or value <= 0,
              do: S.fail(node, :invalid_bounds, "Image requires positive #{key}")

            {key, value}
          end)

        {:image, limits}

      _ ->
        S.fail(node, :invalid_type, "Unsupported type definition")
    end
  end

  def fields(nodes, names) do
    nodes
    |> S.unique(fn node ->
      case S.form(node) do
        {"field", [name, _ | _]} -> S.identifier(name)
        _ -> S.fail(node, :invalid_field, "Expected field name and type")
      end
    end)
    |> Map.new(fn {name, node} ->
      {"field", [_, type | opts]} = S.form(node)
      options = S.options(opts, ["unique", "inverse", "stream", "receivedAt"])
      for key <- ["unique", "receivedAt"], option = options[key], do: S.args(option, key, 0)
      for key <- ["inverse", "stream"], option = options[key], do: S.args(option, key, 1)

      if options["inverse"] && options["stream"],
        do: S.fail(node, :invalid_field, "A field cannot be both inverse and stream")

      {name,
       %{
         type: compile(type, names, relation: !!(options["inverse"] || options["stream"])),
         options: options,
         node: node
       }}
    end)
  end

  def constant(%{kind: :number, value: text}), do: Decimal.new(text)

  def constant(node) do
    case S.form(node) do
      {op, [a, b]} when op in ["add", "sub", "mul", "pow"] ->
        a = constant(a)
        b = constant(b)

        case op do
          "add" ->
            Decimal.add(a, b)

          "sub" ->
            Decimal.sub(a, b)

          "mul" ->
            Decimal.mult(a, b)

          "pow" ->
            unless integral?(b) and Decimal.compare(b, 0) != :lt and
                     Decimal.compare(b, 100) != :gt,
                   do:
                     S.fail(node, :invalid_constant, "Exponent must be an integer from 0 to 100")

            Enum.reduce(List.duplicate(a, Decimal.to_integer(b)), Decimal.new(1), &Decimal.mult/2)
        end

      _ ->
        S.fail(node, :invalid_constant, "Expected a finite numeric constant")
    end
  end

  defp bound(options, key, default) do
    case options[key] do
      nil ->
        default

      node ->
        [value] = S.args(node, key, 1)
        number = constant(value)
        unless integral?(number), do: S.fail(value, :invalid_bounds, "#{key} must be an integer")
        Decimal.to_integer(number)
    end
  end

  defp integral?(number), do: Decimal.equal?(number, Decimal.round(number, 0))
end
