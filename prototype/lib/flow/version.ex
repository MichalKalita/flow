defmodule Flow.Version do
  @moduledoc false
  @enforce_keys [:reference, :state]
  defstruct [:reference, :state]
end
