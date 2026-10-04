defmodule OrderLab.Language.Error do
  defexception [:message, :file, :line, :column, :stage]

  def fail!(message, options \\ []) do
    raise __MODULE__,
          Keyword.merge([message: message, line: 1, column: 1, stage: "compile"], options)
  end

  def diagnostic(error) do
    %{
      "message" => error.message,
      "file" => error.file,
      "line" => error.line,
      "column" => error.column,
      "stage" => error.stage
    }
  end
end
