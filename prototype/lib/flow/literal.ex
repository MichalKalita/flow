defmodule Flow.Literal do
  @moduledoc false
  alias Flow.Syntax
  def decode(%{kind: :string, value: value}), do: value
  def decode(%{kind: :number, value: value}), do: Decimal.new(value)
  def decode(%{kind: :symbol, value: "true"}), do: true
  def decode(%{kind: :symbol, value: "false"}), do: false
  def decode(%{kind: :symbol, value: "null"}), do: nil
  def decode(%{kind: :symbol, value: value}), do: value

  def decode(node) do
    case Syntax.form(node) do
      {"list", values} ->
        Enum.map(values, &decode/1)

      {"record", fields} ->
        fields
        |> Flow.Checker.bindings()
        |> Map.new(fn {name, value} -> {name, decode(value)} end)

      _ ->
        Syntax.fail(node, :invalid_literal, "Expected a literal value")
    end
  end
end
