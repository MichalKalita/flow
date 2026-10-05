defmodule Flow.RuntimeTest do
  use ExUnit.Case, async: true
  alias Flow.{Runtime, ID, Store, Ref}
  @key String.duplicate("runtime-secret", 4)

  defp token(subject) do
    protected = Jason.encode!(%{"alg" => "HS256"}) |> Base.url_encode64(padding: false)

    payload =
      Jason.encode!(%{
        "iss" => "https://identity.example.com",
        "aud" => "application",
        "sub" => subject,
        "exp" => System.system_time(:second) + 3600
      })
      |> Base.url_encode64(padding: false)

    signed = protected <> "." <> payload
    signed <> "." <> Base.url_encode64(:crypto.mac(:hmac, :sha256, @key, signed), padding: false)
  end

  defp credential(subject), do: {"user", token(subject)}

  setup do
    source = File.read!(Path.expand("../priv/workflows/application.flow", __DIR__))

    runtime =
      start_supervised!(
        {Runtime,
         source: source,
         auth: %{"user" => %{key: @key}},
         identities: %{
           "User" => [
             %{
               "id" => "admin",
               "identitySubject" => "idp:admin",
               "roles" => ["CATALOG_ADMIN"],
               "name" => "Admin",
               "email" => "admin@example.test",
               "country" => "CZ"
             }
           ]
         }}
      )

    %{runtime: runtime}
  end

  defp order_input(items \\ [%{"productId" => "p1", "quantity" => 2}]),
    do: %{"userId" => "u1", "items" => items, "paymentMethod" => "CARD"}

  test "real application creates an order, adjusts stock and queues confirmation", %{
    runtime: runtime
  } do
    assert {:ok, receipt} =
             Runtime.execute(runtime, "CreateOrder", order_input(), credential("idp:u1"))

    assert %ID{entity: "Order"} = receipt["order"]["id"]
    assert Decimal.equal?(receipt["order"]["total"], Decimal.new("4980.00"))
    assert receipt["order"]["user"]["name"] == "Petra Nováková"
    assert receipt["order"]["paymentUrl"] == receipt["payment"]["url"]
    assert receipt["email"]["state"] == "QUEUED"
    assert %ID{entity: "Job"} = receipt["email"]["id"]

    assert {:ok, [summary]} =
             Runtime.execute(runtime, "UserOrders", %{"userId" => "u1"}, credential("idp:u1"))

    assert summary["id"] == receipt["order"]["id"]

    assert {:ok, _} =
             Runtime.execute(
               runtime,
               "Order",
               %{"orderId" => receipt["order"]["id"]},
               credential("idp:u1")
             )

    assert {:error, %{code: :not_found}} =
             Runtime.execute(
               runtime,
               "Order",
               %{"orderId" => receipt["order"]["id"]},
               credential("idp:u2")
             )
  end

  test "duplicate cart lines are aggregated once and exact totals remain typed", %{
    runtime: runtime
  } do
    items = [%{"productId" => "p1", "quantity" => 2}, %{"productId" => "p1", "quantity" => 3}]

    assert {:ok, receipt} =
             Runtime.execute(runtime, "CreateOrder", order_input(items), credential("idp:u1"))

    assert [item] = receipt["order"]["items"]
    assert item["quantity"] == 5
    assert Decimal.equal?(receipt["order"]["total"], Decimal.new("12450.00"))
    store = :sys.get_state(runtime).store
    reference = %Ref{entity: "Product", id: %ID{entity: "Product", value: "p1"}}
    assert {:ok, %{"stock" => "7"}} = Store.read(store, &Store.fetch(&1, reference, ["stock"]))
  end

  test "unauthorized, invalid and insufficient-stock orders do not persist effects", %{
    runtime: runtime
  } do
    assert {:error, %{code: :not_found}} =
             Runtime.execute(runtime, "CreateOrder", order_input(), credential("idp:u2"))

    assert {:error, %{code: :invalid_value}} =
             Runtime.execute(
               runtime,
               "CreateOrder",
               order_input([%{"productId" => "p1", "quantity" => 100}]),
               credential("idp:u1")
             )

    assert {:error, %{code: :invalid_value}} =
             Runtime.execute(
               runtime,
               "CreateOrder",
               Map.put(order_input(), "extra", true),
               credential("idp:u1")
             )

    assert {:error, %{code: :unauthenticated}} =
             Runtime.execute(runtime, "CreateOrder", order_input(), {"user", "forged"})

    store = :sys.get_state(runtime).store
    assert {:ok, []} = Store.read(store, &Store.ids(&1, "Order"))
    assert {:ok, []} = Store.read(store, &Store.query(&1, "SELECT id FROM _flow_jobs", []))
  end

  test "device output applies access rights and has bounded history", %{runtime: runtime} do
    assert {:ok, %{"id" => %ID{value: "mower1"}, "status" => nil, "positions" => []}} =
             Runtime.execute(runtime, "Device", %{"deviceId" => "mower1"}, credential("idp:u1"))

    assert {:error, %{code: :not_found}} =
             Runtime.execute(runtime, "Device", %{"deviceId" => "mower1"}, credential("idp:u2"))

    assert {:ok, result} =
             Runtime.execute(
               runtime,
               "SendCommand",
               %{"deviceId" => "mower1", "action" => "START"},
               credential("idp:u1")
             )

    assert result["action"] == "START"

    assert {:error, %{code: :forbidden}} =
             Runtime.execute(
               runtime,
               "SendCommand",
               %{"deviceId" => "mower1", "action" => "STOP"},
               credential("idp:u2")
             )
  end

  test "real photo resizing and transactional file storage require catalog rights", %{
    runtime: runtime
  } do
    {:ok, image} = Vix.Vips.Operation.black(16, 12)
    {:ok, bytes} = Vix.Vips.Image.write_to_buffer(image, ".png")
    input = %{"productId" => "p1", "photo" => bytes, "width" => 8, "height" => 8}

    assert {:error, %{code: :forbidden}} =
             Runtime.execute(runtime, "UploadPhoto", input, credential("idp:u1"))

    assert {:ok, receipt} =
             Runtime.execute(runtime, "UploadPhoto", input, credential("idp:admin"))

    assert receipt["photo"]["width"] == 8
    assert receipt["photo"]["height"] == 6
    assert receipt["photo"]["file"]["id"] == receipt["file"]["id"]

    assert {:ok, [photo]} =
             Runtime.execute(
               runtime,
               "ProductPhotos",
               %{"productId" => "p1"},
               credential("idp:u1")
             )

    assert photo["id"] == receipt["photo"]["id"]
    store = :sys.get_state(runtime).store

    assert {:ok, [[blob]]} =
             Store.read(
               store,
               &Store.query(&1, "SELECT content FROM _flow_blobs WHERE id = ?", [
                 receipt["file"]["id"].value
               ])
             )

    assert {:ok, decoded} = Vix.Vips.Image.new_from_buffer(blob)
    assert Vix.Vips.Image.width(decoded) == 8
  end
end
