defmodule OrderLab.Workflow do
  @moduledoc "Loads the application's single Flow source."
  def path,
    do:
      System.get_env("FLOW_PATH") ||
        Path.join(:code.priv_dir(:order_lab), "workflows/application.flow")

  def source, do: File.read!(path())

  def compile!,
    do:
      OrderLab.Language.Compiler.compile!(source(), path(), OrderLab.Language.Native.operations())
end
