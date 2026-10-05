defmodule Flow.ID do
  @enforce_keys [:entity, :value]
  defstruct [:entity, :value]
end
