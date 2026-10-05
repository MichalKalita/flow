defmodule OrderLab.MixProject do
  use Mix.Project

  def project do
    [app: :order_lab, version: "0.1.0", elixir: "~> 1.18", deps: deps()]
  end

  def application do
    [extra_applications: [:logger, :crypto, :inets, :ssl], mod: {OrderLab.Application, []}]
  end

  defp deps do
    [
      {:bandit, "~> 1.12"},
      {:jason, "~> 1.4"},
      {:exqlite, "~> 0.40"},
      {:vix, "~> 0.41"},
      {:websock_adapter, "~> 0.5"}
    ]
  end
end
