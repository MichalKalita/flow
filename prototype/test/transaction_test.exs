defmodule Flow.TransactionTest do
  use ExUnit.Case, async: true
  alias Flow.{Schema, Store, Permissions, Access, Transaction, ID, Ref}

  setup do
    schema =
      Schema.compile!("""
      [type UserID [id User]] [type PostID [id Post]]
      [enum Role ADMIN]
      [entity User [field id UserID] [field roles [list Role [max 1]]]]
      [entity Post [field id PostID] [field owner User] [field title String]]
      [permissions User
        [User [UPDATE [when [eq actor target]]] [READ [when [eq actor target]]]]
        [Post [CREATE [when [eq actor after.owner]]]
          [READ [when [eq actor target.owner]]]
          [UPDATE [when [contains actor.roles ADMIN]]]
          [DELETE [when [contains actor.roles ADMIN]]]]]
      """)

    {:ok, policy} = Permissions.compile(schema)
    store = start_supervised!({Store, schema: schema})

    {:ok, :ok} =
      Store.transaction(store, fn db ->
        Store.insert(db, "User", %{"id" => "u1", "roles" => []})
        Store.insert(db, "User", %{"id" => "u2", "roles" => ["ADMIN"]})
        Store.insert(db, "Post", %{"id" => "p1", "owner" => "u2", "title" => "private"})
      end)

    %{store: store, policy: policy}
  end

  defp ref(type, id), do: %Ref{entity: type, id: %ID{entity: type, value: id}}

  defp run(store, policy, user, function) do
    Store.transaction(store, fn db ->
      Access.with_session(db, policy, %{type: "User", identity: ref("User", user)}, %{}, function)
    end)
  end

  test "a proposed role change cannot authorize another write in the same transaction", %{
    store: store,
    policy: policy
  } do
    assert {:error, %{code: :forbidden}} =
             run(store, policy, "u1", fn s ->
               Transaction.set(s, ref("User", "u1"), "roles", ["ADMIN"])
               Transaction.set(s, ref("Post", "p1"), "title", "stolen")
               Transaction.authorize!(s)
               Transaction.apply!(s)
             end)

    assert {:ok, %{"roles" => "[]"}} =
             Store.read(store, &Store.fetch(&1, ref("User", "u1"), ["roles"]))

    assert {:ok, %{"title" => "private"}} =
             Store.read(store, &Store.fetch(&1, ref("Post", "p1"), ["title"]))
  end

  test "writes hidden from the result are all authorized and committed together", %{
    store: store,
    policy: policy
  } do
    assert {:ok, :ok} =
             run(store, policy, "u2", fn s ->
               Transaction.set(s, ref("Post", "p1"), "title", "changed")

               Transaction.create(s, "Post", %{
                 "id" => "p2",
                 "owner" => ref("User", "u2"),
                 "title" => "new"
               })

               Transaction.authorize!(s)
               Transaction.apply!(s)
             end)

    assert {:ok, [_, _]} = Store.read(store, &Store.ids(&1, "Post"))

    assert {:ok, %{"title" => "changed"}} =
             Store.read(store, &Store.fetch(&1, ref("Post", "p1"), ["title"]))
  end

  test "a denied create leaves no row even if other writes were permitted", %{
    store: store,
    policy: policy
  } do
    assert {:error, %{code: :forbidden}} =
             run(store, policy, "u2", fn s ->
               Transaction.set(s, ref("Post", "p1"), "title", "changed")

               Transaction.create(s, "Post", %{
                 "id" => "p2",
                 "owner" => ref("User", "u1"),
                 "title" => "new"
               })

               Transaction.authorize!(s)
               Transaction.apply!(s)
             end)

    assert {:ok, [_]} = Store.read(store, &Store.ids(&1, "Post"))

    assert {:ok, %{"title" => "private"}} =
             Store.read(store, &Store.fetch(&1, ref("Post", "p1"), ["title"]))
  end

  test "authorized deletion and repeated staged updates retain original policy data", %{
    store: store,
    policy: policy
  } do
    assert {:ok, :ok} =
             run(store, policy, "u2", fn s ->
               Transaction.set(s, ref("User", "u2"), "roles", [])
               Transaction.set(s, ref("Post", "p1"), "title", "one")
               Transaction.set(s, ref("Post", "p1"), "title", "two")
               Transaction.delete(s, ref("Post", "p1"))
               Transaction.authorize!(s)
               Transaction.apply!(s)
             end)

    assert {:ok, []} = Store.read(store, &Store.ids(&1, "Post"))
  end
end
