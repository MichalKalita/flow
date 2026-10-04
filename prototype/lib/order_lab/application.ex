defmodule OrderLab.Application do
  use Application

  def start(_type, _args) do
    port = System.get_env("PORT", "4000") |> String.to_integer()

    Supervisor.start_link(
      [
        OrderLab.Store,
        OrderLab.EmailWorker,
        {Bandit, plug: OrderLab.Router, ip: {127, 0, 0, 1}, port: port}
      ],
      strategy: :one_for_one,
      name: OrderLab.Supervisor
    )
  end
end
