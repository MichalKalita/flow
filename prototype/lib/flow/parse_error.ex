defmodule Flow.ParseError do
  @moduledoc "A syntax or input-limit error, with a zero-based byte offset and one-based location."
  defexception [:code, :message, :file, :offset, :line, :column, :opened_at]

  @type t :: %__MODULE__{
          code: atom(),
          message: binary(),
          file: binary(),
          offset: non_neg_integer(),
          line: pos_integer(),
          column: pos_integer(),
          opened_at: map() | nil
        }
end
