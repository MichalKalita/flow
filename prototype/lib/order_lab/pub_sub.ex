defmodule OrderLab.PubSub do
  @moduledoc "Process-monitored, bounded live subscriptions; no durable replay queue."
  use GenServer
  def start_link(_), do: GenServer.start_link(__MODULE__, nil, name: __MODULE__)
  def subscribe(pid, key), do: GenServer.call(__MODULE__, {:subscribe, pid, key})
  def unsubscribe(pid, key), do: GenServer.call(__MODULE__, {:unsubscribe, pid, key})
  def remove(pid), do: GenServer.cast(__MODULE__, {:remove, pid})
  def broadcast(key, message), do: GenServer.call(__MODULE__, {:broadcast, key, message})
  def stats, do: GenServer.call(__MODULE__, :stats)
  def init(_), do: {:ok, %{clients: %{}, delivered: 0, overflows: 0}}

  def handle_call({:subscribe, pid, key}, _, state) do
    client =
      Map.get_lazy(state.clients, pid, fn ->
        %{monitor: Process.monitor(pid), keys: MapSet.new()}
      end)

    client = %{client | keys: MapSet.put(client.keys, key)}
    {:reply, :ok, %{state | clients: Map.put(state.clients, pid, client)}}
  end

  def handle_call({:unsubscribe, pid, key}, _, state) do
    clients =
      case Map.fetch(state.clients, pid) do
        {:ok, client} ->
          Map.put(state.clients, pid, %{client | keys: MapSet.delete(client.keys, key)})

        :error ->
          state.clients
      end

    {:reply, :ok, %{state | clients: clients}}
  end

  def handle_call({:broadcast, key, message}, _, state) do
    state =
      Enum.reduce(state.clients, state, fn {pid, client}, state ->
        if MapSet.member?(client.keys, key) do
          case Process.info(pid, :message_queue_len) do
            {:message_queue_len, count} when count < 128 ->
              send(pid, {:flow_ws_message, key, message})
              %{state | delivered: state.delivered + 1}

            {:message_queue_len, _} ->
              send(pid, :flow_ws_overflow)

              %{
                state
                | clients: Map.put(state.clients, pid, %{client | keys: MapSet.new()}),
                  overflows: state.overflows + 1
              }

            nil ->
              state
          end
        else
          state
        end
      end)

    {:reply, :ok, state}
  end

  def handle_call(:stats, _, state) do
    {:reply,
     %{
       "connections" => map_size(state.clients),
       "subscriptions" =>
         Enum.reduce(state.clients, 0, fn {_, client}, total ->
           total + MapSet.size(client.keys)
         end),
       "delivered" => state.delivered,
       "overflows" => state.overflows
     }, state}
  end

  def handle_cast({:remove, pid}, state), do: {:noreply, remove_client(state, pid)}
  def handle_info({:DOWN, _, :process, pid, _}, state), do: {:noreply, remove_client(state, pid)}

  defp remove_client(state, pid) do
    case Map.pop(state.clients, pid) do
      {nil, _} ->
        state

      {client, remaining} ->
        Process.demonitor(client.monitor, [:flush])
        %{state | clients: remaining}
    end
  end
end

defmodule OrderLab.WebSocket do
  @behaviour WebSock
  def init(endpoint), do: {:ok, %{endpoint: endpoint, subscriptions: MapSet.new()}}

  def handle_in({data, [opcode: :text]}, state) do
    if byte_size(data) > 8192 do
      {:stop, :message_too_large, 1009, state}
    else
      case Jason.decode(data) do
        {:ok, command} when is_map(command) -> command(command, state)
        _ -> reply_error("invalid_json", "Expected a JSON command object", state)
      end
    end
  end

  def handle_in(_, state), do: reply_error("text_required", "Use a JSON text frame", state)

  defp command(command, state) do
    cond do
      Map.keys(command) -- ~w(action source params latest) != [] ->
        reply_error("unknown_fields", "Unknown subscription command fields", state)

      command["action"] not in ["subscribe", "unsubscribe"] ->
        reply_error("unknown_action", "Use subscribe or unsubscribe", state)

      not is_boolean(Map.get(command, "latest", false)) ->
        reply_error("invalid_latest", "latest must be Bool", state)

      command["source"] not in state.endpoint.sources ->
        reply_error(
          "source_not_allowed",
          "Source is not allowed by this WebSocket declaration",
          state
        )

      true ->
        case OrderLab.Store.subscription(
               command["source"],
               Map.get(command, "params", %{}),
               self(),
               command["action"],
               Map.get(command, "latest", false),
               state.subscriptions
             ) do
          {:ok, key, params, latest} ->
            subscribing = command["action"] == "subscribe"

            keys =
              if subscribing,
                do: MapSet.put(state.subscriptions, key),
                else: MapSet.delete(state.subscriptions, key)

            ack = %{
              "type" => if(subscribing, do: "subscribed", else: "unsubscribed"),
              "source" => command["source"],
              "params" => params
            }

            messages =
              [{:text, Jason.encode!(ack)}] ++
                if(latest, do: [{:text, Jason.encode!(latest)}], else: [])

            {:push, messages, %{state | subscriptions: keys}}

          {:error, message} ->
            reply_error("invalid_subscription", message, state)
        end
    end
  end

  def handle_info({:flow_ws_message, key, message}, state) do
    if MapSet.member?(state.subscriptions, key),
      do: {:push, {:text, Jason.encode!(message)}, state},
      else: {:ok, state}
  end

  def handle_info(:flow_ws_overflow, state), do: {:stop, :slow_subscriber, 1013, state}
  def handle_info(_, state), do: {:ok, state}
  def terminate(_, _), do: OrderLab.PubSub.remove(self())

  defp reply_error(code, message, state),
    do:
      {:push, {:text, Jason.encode!(%{"type" => "error", "code" => code, "message" => message})},
       state}
end
