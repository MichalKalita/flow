defmodule Flow.MixProject do
  use Mix.Project

  def project do
    [app: :flow, version: "0.2.0", elixir: "~> 1.18", deps: deps()]
  end

  def application do
    [extra_applications: [:logger, :crypto, :public_key, :ssl, :inets]]
  end

  defp deps do
    [
      {:exqlite, "~> 0.42"},
      {:decimal, "~> 2.3"},
      {:jason, "~> 1.4"},
      {:bandit, "~> 1.12"},
      {:websock_adapter, "~> 0.6"},
      {:vix, "~> 0.42"}
    ]
  end
end
