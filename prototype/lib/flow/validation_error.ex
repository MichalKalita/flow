defmodule Flow.ValidationError do
  defexception [:code, :message, :span]
end
