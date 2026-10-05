defmodule Flow.QueueTest do
  use ExUnit.Case, async: true
  alias Flow.{Runtime, Store, Ref, ID}
  @key String.duplicate("queue-jwt-key", 4)
  @queue_key String.duplicate("q", 32)

  defp source, do: File.read!(Path.expand("../priv/workflows/application.flow", __DIR__))

  defp credential(exp \\ System.system_time(:second) + 3600) do
    header = Jason.encode!(%{"alg" => "HS256"}) |> Base.url_encode64(padding: false)

    payload =
      Jason.encode!(%{
        "iss" => "https://identity.example.com",
        "aud" => "application",
        "sub" => "idp:u1",
        "exp" => exp
      })
      |> Base.url_encode64(padding: false)

    signed = header <> "." <> payload

    {"user",
     signed <> "." <> Base.url_encode64(:crypto.mac(:hmac, :sha256, @key, signed), padding: false)}
  end

  defp options(overrides),
    do:
      Keyword.merge(
        [
          source: source(),
          auth: %{"user" => %{key: @key}},
          queue_key: @queue_key,
          worker_interval: :manual
        ],
        overrides
      )

  defp order(runtime, credential \\ credential(), failure \\ false) do
    Runtime.execute(
      runtime,
      "CreateOrder",
      %{
        "userId" => "u1",
        "items" => [%{"productId" => "p1", "quantity" => 1}],
        "paymentMethod" => "CARD",
        "emailFailure" => failure
      },
      credential
    )
  end

  defp handler(owner) do
    %{
      mode: :external,
      function: fn args, metadata ->
        send(owner, {:delivered, args["orderId"], metadata.idempotency_key})
        %{"state" => "SENT"}
      end
    }
  end

  test "queued execution retains typed args and reauthorizes the committed order" do
    runtime =
      start_supervised!(
        {Runtime, options(plugins: %{"Email.sendConfirmation" => handler(self())})}
      )

    assert {:ok, receipt} = order(runtime)
    id = receipt["email"]["id"].value
    assert {:ok, [%{id: ^id, state: "DONE", attempts: 1}]} = Runtime.process_jobs(runtime)
    assert_received {:delivered, order_id, ^id}
    assert order_id == receipt["order"]["id"]
    assert {:ok, []} = Runtime.process_jobs(runtime)
  end

  test "revoked ownership blocks delivery before the external callback" do
    runtime =
      start_supervised!(
        {Runtime, options(plugins: %{"Email.sendConfirmation" => handler(self())})}
      )

    {:ok, receipt} = order(runtime)
    store = :sys.get_state(runtime).store
    order = %Ref{entity: "Order", id: receipt["order"]["id"]}

    assert {:ok, :ok} =
             Store.transaction(
               store,
               &Store.update(&1, order, %{
                 "user" => %Ref{entity: "User", id: %ID{entity: "User", value: "u2"}}
               })
             )

    assert {:ok, [%{state: "BLOCKED", error: "not_found"}]} = Runtime.process_jobs(runtime)
    refute_received {:delivered, _, _}
  end

  test "expired JWT is rechecked at delivery and cannot become anonymous" do
    {:ok, clock} = Agent.start_link(fn -> DateTime.from_unix!(1000) end)

    runtime =
      start_supervised!(
        {Runtime,
         options(
           clock: fn -> Agent.get(clock, & &1) end,
           plugins: %{"Email.sendConfirmation" => handler(self())}
         )}
      )

    assert {:ok, _} = order(runtime, credential(1001))
    Agent.update(clock, fn _ -> DateTime.from_unix!(1002) end)
    assert {:ok, [%{state: "BLOCKED", error: "unauthenticated"}]} = Runtime.process_jobs(runtime)
    refute_received {:delivered, _, _}
    Agent.stop(clock)
  end

  test "failed deliveries retry with the same idempotency key and retain after the last attempt" do
    owner = self()

    failing = %{
      mode: :external,
      function: fn _, metadata ->
        send(owner, {:attempt, metadata.idempotency_key})
        raise "temporary"
      end
    }

    runtime =
      start_supervised!({Runtime, options(plugins: %{"Email.sendConfirmation" => failing})})

    {:ok, receipt} = order(runtime)
    id = receipt["email"]["id"].value
    store = :sys.get_state(runtime).store

    for attempt <- 1..3 do
      {:ok, _} =
        Store.transaction(
          store,
          &Store.query(&1, "UPDATE _flow_jobs SET next_at = 0 WHERE id = ?", [id])
        )

      assert {:ok, [%{id: ^id, attempts: ^attempt, state: state}]} = Runtime.process_jobs(runtime)
      assert state == if(attempt == 3, do: "RETAINED", else: "RETRY")
      assert_received {:attempt, ^id}
    end

    assert {:ok, []} = Runtime.process_jobs(runtime)
  end

  test "the encrypted outbox survives a runtime restart with stable keys" do
    path = Path.join(System.tmp_dir!(), "flow-queue-#{System.unique_integer([:positive])}.sqlite")
    on_exit(fn -> for suffix <- ["", "-wal", "-shm"], do: File.rm(path <> suffix) end)
    opts = options(database: path, plugins: %{"Email.sendConfirmation" => handler(self())})
    {:ok, runtime} = Runtime.start_link(opts)
    {:ok, receipt} = order(runtime)
    GenServer.stop(runtime)
    {:ok, restarted} = Runtime.start_link(opts)
    assert {:ok, [%{state: "DONE"}]} = Runtime.process_jobs(restarted)
    assert_received {:delivered, _, id}
    assert id == receipt["email"]["id"].value
    GenServer.stop(restarted)
  end

  test "persistent runtime rejects ephemeral encryption key configuration" do
    Process.flag(:trap_exit, true)

    assert {:error, %ArgumentError{}} =
             Runtime.start_link(
               options(database: "/tmp/not-created-flow.sqlite")
               |> Keyword.delete(:queue_key)
             )
  end
end
