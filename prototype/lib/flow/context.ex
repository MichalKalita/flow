defmodule Flow.Context do
  @moduledoc "Typed trusted facts supplied by the host; client input never populates this context."
  def build!(schema, principal, transport, provider, now \\ DateTime.utc_now()) do
    facts = provider.(principal, transport)

    unless is_map(facts) and not is_struct(facts),
      do: raise(ArgumentError, "Context provider must return a record")

    facts = Map.put(facts, "now", now)

    if schema.types["Context"],
      do: Flow.Value.validate!(schema, {:named, "Context"}, facts),
      else: Map.take(facts, ["now"])
  end
end
