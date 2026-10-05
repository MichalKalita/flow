defmodule OrderLab.Checkpoint do
  @moduledoc "Persisted interpreter continuations; external effects need a separate retry contract."
  def fingerprint(program) do
    :crypto.hash(
      :sha256,
      :erlang.term_to_binary({1, program.source, OrderLab.Language.Native.operations()}, [
        :deterministic
      ])
    )
    |> Base.encode16(case: :lower)
  end

  def encode(ctx), do: ctx |> :erlang.term_to_binary([:compressed]) |> Base.encode64()

  def decode(value) do
    Code.ensure_loaded!(OrderLab.Language.Runtime)
    value |> Base.decode64!() |> :erlang.binary_to_term([:safe])
  end

  def safe?(ctx), do: safe_term?(ctx.continuation)

  defp safe_term?(%{kind: :call, operation: name}) do
    case Map.fetch(OrderLab.Language.Native.operations(), name) do
      {:ok, operation} ->
        Map.has_key?(operation, :native) or Map.get(operation, :effect) in [:pure, :read]

      :error ->
        false
    end
  end

  defp safe_term?(value) when is_map(value),
    do: Enum.all?(value, fn {_, child} -> safe_term?(child) end)

  defp safe_term?(value) when is_list(value), do: Enum.all?(value, &safe_term?/1)

  defp safe_term?(value) when is_tuple(value),
    do: value |> Tuple.to_list() |> Enum.all?(&safe_term?/1)

  defp safe_term?(_), do: true

  # Only enabled by the E2E runner. The runner observes the marker, then SIGKILLs
  # the process group; this exposes SQLite crash behavior rather than mocking it.
  def probe(request, phase) do
    marker = System.get_env("FLOW_E2E_CRASH_MARKER")

    if marker && System.get_env("FLOW_E2E_CRASH_PHASE") == to_string(phase) &&
         System.get_env("FLOW_E2E_CRASH_ROUTE") == request["path"] do
      File.write!(
        marker,
        Jason.encode!(%{phase: phase, request_id: request["id"], request_time: request["time"]})
      )

      receive do
        :resume_e2e -> :ok
      end
    end
  end
end
