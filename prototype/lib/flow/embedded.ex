defmodule Flow.Embedded do
  @moduledoc false
  @enforce_keys [:type, :value, :parent]
  defstruct [:type, :value, :parent]
end
