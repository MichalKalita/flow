defmodule Flow.Syntax do
  @moduledoc false
  alias Flow.ValidationError

  def form(%{kind: :list, value: [%{kind: :symbol, value: name} | args]}),
    do: {name, args}

  def form(node), do: fail(node, :expected_form, "Expected a named bracketed form")

  def symbol(%{kind: :symbol, value: value}), do: value
  def symbol(node), do: fail(node, :expected_symbol, "Expected a symbol")

  def identifier(node) do
    name = symbol(node)

    if Regex.match?(~r/\A[A-Za-z_][A-Za-z_0-9]*\z/, name),
      do: name,
      else: fail(node, :invalid_identifier, "Invalid identifier #{name}")
  end

  def string(%{kind: :string, value: value}), do: value
  def string(node), do: fail(node, :expected_string, "Expected a string")

  def fail(node, code, message),
    do: raise(ValidationError, code: code, message: message, span: Map.get(node, :span))

  def unique(nodes, key_fun) do
    Enum.reduce(nodes, %{}, fn node, found ->
      key = key_fun.(node)
      if Map.has_key?(found, key), do: fail(node, :duplicate, "Duplicate #{key}")
      Map.put(found, key, node)
    end)
  end

  def options(nodes, allowed) do
    unique(nodes, fn node ->
      {key, _} = form(node)
      unless key in allowed, do: fail(node, :unknown_option, "Unknown option #{key}")
      key
    end)
  end

  def args(node, name, count) do
    case form(node) do
      {^name, values} when length(values) == count -> values
      _ -> fail(node, :arity, "#{name} requires #{count} arguments")
    end
  end
end
