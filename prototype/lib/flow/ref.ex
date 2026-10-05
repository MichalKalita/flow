defmodule Flow.Ref do
  @moduledoc "A lazy, branded entity reference. It contains no eagerly loaded columns."
  @enforce_keys [:entity, :id]
  defstruct [:entity, :id]
end
