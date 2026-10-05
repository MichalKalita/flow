defmodule Flow.AccessTest do
  use ExUnit.Case, async: true
  alias Flow.{Store, Schema, Permissions, Ref, ID, Access, Projection}

  setup do
    schema =
      Schema.compile!("""
      [type UserID [id User]] [type PostID [id Post]]
      [entity User [field id UserID] [field name String]
        [field posts [list Post] [inverse Post.author]]]
      [entity Post [field id PostID] [field author User] [field public Bool]
        [field title String] [field secret String]]
      [type PostSummary [record [field id PostID] [field title String]]]
      [permissions User
        [User [READ [when [eq target actor]]]]
        [Post [READ [when [or target.public [eq target.author actor]]]]]
        [Post.secret [READ [when [eq target.author actor]]]]]
      """)

    {:ok, policy} = Permissions.compile(schema)
    store = start_supervised!({Store, schema: schema, trace: self()})

    {:ok, :ok} =
      Store.transaction(store, fn db ->
        Store.insert(db, "User", %{"id" => "u1", "name" => "Petra"})
        Store.insert(db, "User", %{"id" => "u2", "name" => "David"})

        Store.insert(db, "Post", %{
          "id" => "p1",
          "author" => "u1",
          "public" => false,
          "title" => "Petra private",
          "secret" => "one"
        })

        Store.insert(db, "Post", %{
          "id" => "p2",
          "author" => "u2",
          "public" => false,
          "title" => "David private",
          "secret" => "two"
        })

        Store.insert(db, "Post", %{
          "id" => "p3",
          "author" => "u2",
          "public" => true,
          "title" => "Public",
          "secret" => "three"
        })
      end)

    %{store: store, policy: policy}
  end

  defp ref(entity, value), do: %Ref{entity: entity, id: %ID{entity: entity, value: value}}

  defp session(store, policy, user, function) do
    Store.read(store, fn db ->
      Access.with_session(db, policy, %{type: "User", identity: ref("User", user)}, %{}, function)
    end)
  end

  test "output projection uses policy columns and selected output columns only", %{
    store: store,
    policy: policy
  } do
    assert {:ok, %{"title" => "Petra private", "id" => %ID{value: "p1"}}} =
             session(store, policy, "u1", fn s ->
               Projection.project(s, {:named, "PostSummary"}, ref("Post", "p1"))
             end)

    assert_received {:flow_sql, ~s(SELECT "title" FROM "Post" WHERE id = ?), ["p1"]}
    assert_received {:flow_sql, ~s(SELECT "public" FROM "Post" WHERE id = ?), ["p1"]}
    assert_received {:flow_sql, ~s(SELECT "author" FROM "Post" WHERE id = ?), ["p1"]}
    refute_received {:flow_sql, ~s(SELECT "secret" FROM "Post" WHERE id = ?), _}
  end

  test "private rows are removed before a caller applies limits", %{store: store, policy: policy} do
    assert {:ok, ["p1", "p3"]} =
             session(store, policy, "u1", fn s ->
               Store.ids(s.db, "Post") |> Access.visible(s) |> Enum.map(& &1.id.value)
             end)

    assert {:ok, ["p2"]} =
             session(store, policy, "u2", fn s ->
               Store.ids(s.db, "Post")
               |> Access.visible(s)
               |> Enum.take(1)
               |> Enum.map(& &1.id.value)
             end)
  end

  test "public entity access does not expose protected fields or inverse relations", %{
    store: store,
    policy: policy
  } do
    assert {:error, %{code: :not_found}} =
             session(store, policy, "u1", &Access.field(&1, ref("Post", "p3"), "secret"))

    assert {:error, %{code: :not_found}} =
             session(store, policy, "u1", &Access.field(&1, ref("User", "u2"), "posts"))

    assert {:ok, [_]} =
             session(store, policy, "u1", &Access.field(&1, ref("User", "u1"), "posts"))
  end

  test "nonexistent and unauthorized direct reads have the same external error", %{
    store: store,
    policy: policy
  } do
    for id <- ["p2", "missing"] do
      assert {:error, %{code: :not_found, message: "Resource unavailable"}} =
               session(store, policy, "u1", &Access.field(&1, ref("Post", id), "title"))
    end
  end

  test "a new delivery reads current rights and does not reuse the previous session cache", %{
    store: store,
    policy: policy
  } do
    assert {:ok, "Public"} =
             session(store, policy, "u1", &Access.field(&1, ref("Post", "p3"), "title"))

    assert {:ok, :ok} =
             Store.transaction(store, &Store.update(&1, ref("Post", "p3"), %{"public" => false}))

    assert {:error, %{code: :not_found}} =
             session(store, policy, "u1", &Access.field(&1, ref("Post", "p3"), "title"))
  end
end
