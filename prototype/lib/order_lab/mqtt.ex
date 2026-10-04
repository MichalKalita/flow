defmodule OrderLab.MQTT do
  @moduledoc "Local MQTT 3.1.1 broker: typed PUBLISH, QoS 0/1, subscriptions and retained messages."
  use GenServer
  import Bitwise
  def start_link(_), do: GenServer.start_link(__MODULE__, nil, name: __MODULE__)

  def init(_) do
    port = System.get_env("MQTT_PORT", "1883") |> String.to_integer()

    {:ok, listener} =
      :gen_tcp.listen(port, [
        :binary,
        active: false,
        packet: :raw,
        ip: {127, 0, 0, 1},
        reuseaddr: true
      ])

    owner = self()
    acceptor = spawn_link(fn -> accept(listener, owner) end)
    {:ok, %{listener: listener, acceptor: acceptor, clients: %{}}}
  end

  def handle_call({:subscribe, socket, filters}, _, state) do
    clients = Map.update(state.clients, socket, filters, &Enum.uniq(&1 ++ filters))
    {:reply, :ok, %{state | clients: clients}}
  end

  def handle_call({:unsubscribe, socket, filters}, _, state) do
    {:reply, :ok, %{state | clients: Map.update(state.clients, socket, [], &(&1 -- filters))}}
  end

  def handle_call({:publish, topic, payload}, _, state) do
    Enum.each(state.clients, fn {socket, filters} ->
      if Enum.any?(filters, &matches?(&1, topic)),
        do: :gen_tcp.send(socket, packet(0x30, string(topic) <> payload))
    end)

    {:reply, :ok, state}
  end

  def handle_info({:closed, socket}, state),
    do: {:noreply, %{state | clients: Map.delete(state.clients, socket)}}

  def terminate(_, state), do: :gen_tcp.close(state.listener)

  defp accept(listener, owner) do
    case :gen_tcp.accept(listener) do
      {:ok, socket} ->
        {:ok, pid} =
          Task.start(fn ->
            receive do
              :ready ->
                try do
                  connect(socket)
                  loop(socket, %{})
                rescue
                  _ -> :ok
                after
                  :gen_tcp.close(socket)
                  send(owner, {:closed, socket})
                end
            end
          end)

        :ok = :gen_tcp.controlling_process(socket, pid)
        send(pid, :ready)
        accept(listener, owner)

      {:error, :closed} ->
        :ok

      {:error, reason} ->
        exit(reason)
    end
  end

  defp connect(socket) do
    {0x10, body} = read_packet(socket, 10_000)
    {"MQTT", <<4, flags, _keepalive::16, rest::binary>>} = take_string(body)
    unless (flags &&& 1) == 0 and (flags &&& 4) == 0, do: raise("Will messages are not supported")
    {client_id, rest} = take_string(rest)
    unless client_id != "" or (flags &&& 2) != 0, do: raise("Client ID is required")
    rest = if (flags &&& 128) != 0, do: elem(take_string(rest), 1), else: rest
    rest = if (flags &&& 64) != 0, do: elem(take_string(rest), 1), else: rest
    unless rest == "", do: raise("Invalid CONNECT")
    :ok = :gen_tcp.send(socket, <<0x20, 2, 0, 0>>)
  end

  defp loop(socket, seen) do
    {header, body} = read_packet(socket, 60_000)

    case header >>> 4 do
      3 ->
        qos = header >>> 1 &&& 3
        unless qos in [0, 1], do: raise("QoS 2 unsupported")
        {topic, rest} = take_string(body)

        unless topic != "" and not String.contains?(topic, ["#", "+", "\0"]),
          do: raise("Invalid topic")

        {identifier, payload} =
          if qos == 1,
            do:
              (
                <<id::16, payload::binary>> = rest
                {id, payload}
              ),
            else: {nil, rest}

        if qos == 1 and identifier == 0, do: raise("Invalid packet identifier")
        fingerprint = :crypto.hash(:sha256, topic <> <<0>> <> payload)
        duplicate = (header &&& 8) != 0 and Map.get(seen, identifier) == fingerprint

        unless duplicate do
          :ok = OrderLab.Store.mqtt_publish(topic, payload, (header &&& 1) != 0)
          :ok = GenServer.call(__MODULE__, {:publish, topic, payload})
        end

        if qos == 1 do
          if identifier == 0, do: raise("Invalid packet identifier")
          :ok = :gen_tcp.send(socket, <<0x40, 2, identifier::16>>)
        end

        seen = if map_size(seen) >= 100, do: %{}, else: seen
        loop(socket, if(qos == 1, do: Map.put(seen, identifier, fingerprint), else: seen))

      8 ->
        unless header == 0x82, do: raise("Invalid SUBSCRIBE flags")
        <<id::16, filters::binary>> = body
        if id == 0, do: raise("Invalid packet identifier")
        subscriptions = subscriptions(filters, [])
        if subscriptions == [], do: raise("Empty SUBSCRIBE")
        accepted = for {filter, qos} <- subscriptions, qos in [0, 1], do: filter
        :ok = GenServer.call(__MODULE__, {:subscribe, socket, accepted})

        codes =
          Enum.map(subscriptions, fn {_, qos} -> if qos in [0, 1], do: 0, else: 128 end)
          |> :erlang.list_to_binary()

        :ok = :gen_tcp.send(socket, packet(0x90, <<id::16>> <> codes))

        Enum.each(OrderLab.Store.mqtt_retained(), fn row ->
          if Enum.any?(accepted, &matches?(&1, row["topic"])),
            do: :gen_tcp.send(socket, packet(0x31, string(row["topic"]) <> row["payload_json"]))
        end)

        loop(socket, seen)

      10 ->
        unless header == 0xA2, do: raise("Invalid UNSUBSCRIBE flags")
        <<id::16, filters::binary>> = body
        names = unsubscribe_filters(filters, [])
        :ok = GenServer.call(__MODULE__, {:unsubscribe, socket, names})
        :ok = :gen_tcp.send(socket, <<0xB0, 2, id::16>>)
        loop(socket, seen)

      12 ->
        unless header == 0xC0 and body == "", do: raise("Invalid PINGREQ")
        :ok = :gen_tcp.send(socket, <<0xD0, 0>>)
        loop(socket, seen)

      14 ->
        unless header == 0xE0 and body == "", do: raise("Invalid DISCONNECT")
        :ok

      _ ->
        raise("Unsupported MQTT packet")
    end
  end

  defp subscriptions("", acc), do: Enum.reverse(acc)

  defp subscriptions(data, acc) do
    {filter, <<qos, rest::binary>>} = take_string(data)
    validate_filter!(filter)
    subscriptions(rest, [{filter, qos} | acc])
  end

  defp unsubscribe_filters("", acc), do: Enum.reverse(acc)

  defp unsubscribe_filters(data, acc) do
    {filter, rest} = take_string(data)
    validate_filter!(filter)
    unsubscribe_filters(rest, [filter | acc])
  end

  defp validate_filter!(filter) do
    levels = String.split(filter, "/")

    valid =
      filter != "" and not String.contains?(filter, "\0") and
        Enum.with_index(levels)
        |> Enum.all?(fn {part, i} ->
          part == "+" or (part == "#" and i == length(levels) - 1) or
            not String.contains?(part, ["+", "#"])
        end)

    unless valid, do: raise("Invalid subscription filter")
  end

  def matches?(filter, topic),
    do: match_levels(String.split(filter, "/"), String.split(topic, "/"))

  defp match_levels(["#"], _), do: true
  defp match_levels([], []), do: true
  defp match_levels(["+" | a], [_ | b]), do: match_levels(a, b)
  defp match_levels([same | a], [same | b]), do: match_levels(a, b)
  defp match_levels(_, _), do: false

  defp take_string(<<length::16, value::binary-size(length), rest::binary>>) do
    unless String.valid?(value), do: raise("Invalid MQTT UTF-8")
    {value, rest}
  end

  defp read_packet(socket, timeout) do
    {:ok, <<header>>} = :gen_tcp.recv(socket, 1, timeout)
    size = remaining_length(socket, 0, 1, 0, timeout)
    if size > 64_000, do: raise("MQTT packet exceeds 64 kB")

    body =
      if size == 0,
        do: "",
        else:
          (
            {:ok, body} = :gen_tcp.recv(socket, size, timeout)
            body
          )

    {header, body}
  end

  defp remaining_length(_, _, _, 4, _), do: raise("Invalid remaining length")

  defp remaining_length(socket, value, multiplier, count, timeout) do
    {:ok, <<byte>>} = :gen_tcp.recv(socket, 1, timeout)
    value = value + (byte &&& 127) * multiplier

    if (byte &&& 128) == 0,
      do: value,
      else: remaining_length(socket, value, multiplier * 128, count + 1, timeout)
  end

  defp string(value), do: <<byte_size(value)::16>> <> value
  defp packet(header, body), do: <<header>> <> encode_length(byte_size(body)) <> body
  defp encode_length(length) when length < 128, do: <<length>>
  defp encode_length(length), do: <<rem(length, 128) ||| 128>> <> encode_length(div(length, 128))
end
