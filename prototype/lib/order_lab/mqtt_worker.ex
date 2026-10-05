defmodule OrderLab.MQTTWorker do
  use GenServer
  def start_link(_), do: GenServer.start_link(__MODULE__, nil, name: __MODULE__)

  def init(_) do
    Process.send_after(self(), :tick, 250)
    {:ok, nil}
  end

  def handle_info(:tick, state) do
    OrderLab.Store.deliver_mqtt()
    Process.send_after(self(), :tick, 250)
    {:noreply, state}
  end
end
