defmodule Flow.StoreTest do
  use ExUnit.Case, async: true
  alias Flow.{Schema, Store, ID, Ref}

  setup do
    schema =
      Schema.compile!("""
      [type UserID [id User]] [type ItemID [id Item]]
      [type Price [decimal [range 0.01 999999999999999999.99] [scale 2]]]
      [entity User [field id UserID] [field name String] [field secret String]]
      [entity Item [field id ItemID] [field owner User] [field price Price]]
      """)

    %{schema: schema}
  end

  test "SQLite persists exact decimals and references, and selects only requested columns", %{
    schema: schema
  } do
    store = start_supervised!({Store, schema: schema, trace: self()})
    user = %Ref{entity: "User", id: %ID{entity: "User", value: "u1"}}

    assert {:ok, :ok} =
             Store.transaction(store, fn db ->
               Store.insert(db, "User", %{
                 "id" => user.id,
                 "name" => "Petra",
                 "secret" => "hidden"
               })

               Store.insert(db, "Item", %{
                 "id" => "i1",
                 "owner" => user,
                 "price" => Decimal.new("999999999999999999.99")
               })
             end)

    assert {:ok, %{"name" => "Petra"}} = Store.read(store, &Store.fetch(&1, user, ["name"]))
    assert_received {:flow_sql, ~s(SELECT "name" FROM "User" WHERE id = ?), ["u1"]}
    item = %Ref{entity: "Item", id: %ID{entity: "Item", value: "i1"}}

    assert {:ok, %{"price" => "999999999999999999.99"}} =
             Store.read(store, &Store.fetch(&1, item, ["price"]))

    assert {:ok, [^user]} = Store.read(store, &Store.ids(&1, "User"))
  end

  test "failed transactions roll back all changes and enforce foreign keys", %{schema: schema} do
    store = start_supervised!({Store, schema: schema})

    assert {:error, _} =
             Store.transaction(store, fn db ->
               Store.insert(db, "User", %{"id" => "u1", "name" => "Petra", "secret" => "secret"})

               Store.insert(db, "Item", %{
                 "id" => "i1",
                 "owner" => "missing",
                 "price" => Decimal.new("1.00")
               })
             end)

    assert {:ok, []} = Store.read(store, &Store.ids(&1, "User"))

    assert {:ok, :ok} =
             Store.transaction(
               store,
               &Store.insert(&1, "User", %{"id" => "u1", "name" => "Petra", "secret" => "secret"})
             )

    assert {:ok, [_]} = Store.read(store, &Store.ids(&1, "User"))
  end

  test "data remains after reopening the SQLite file", %{schema: schema} do
    path = Path.join(System.tmp_dir!(), "flow-store-#{System.unique_integer([:positive])}.sqlite")
    on_exit(fn -> for suffix <- ["", "-wal", "-shm"], do: File.rm(path <> suffix) end)
    {:ok, store} = Store.start_link(schema: schema, path: path)

    assert {:ok, :ok} =
             Store.transaction(
               store,
               &Store.insert(&1, "User", %{"id" => "u1", "name" => "Petra", "secret" => "secret"})
             )

    GenServer.stop(store)
    {:ok, store} = Store.start_link(schema: schema, path: path)
    assert {:ok, [_]} = Store.read(store, &Store.ids(&1, "User"))
    GenServer.stop(store)
  end
end
