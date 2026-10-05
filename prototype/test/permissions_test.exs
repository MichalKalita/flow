defmodule Flow.PermissionsTest do
  use ExUnit.Case, async: true
  alias Flow.{Permissions, Schema}

  defp policy(grants) do
    schema =
      Schema.compile!("""
      [type UserID [id User]] [type PostID [id Post]]
      [entity User [field id UserID]]
      [entity Post [field id PostID] [field secret String]]
      #{grants}
      """)

    Permissions.compile(schema)
  end

  test "action inheritance is transitive and retains the original grant condition" do
    assert {:ok, policy} =
             policy("""
             [permissions User [Post
               [READ [when false]]
               [UPDATE [includes READ] [when false]]
               [DELETE [includes UPDATE] [when true]]]]
             """)

    for action <- ~w(READ UPDATE DELETE) do
      assert Permissions.allowed?(policy, "User", "Post", action, &(&1.value == "true"))
    end

    refute Permissions.allowed?(policy, "User", "Post", "CREATE", fn _ -> true end)
    refute Permissions.allowed?(policy, "Anonymous", "Post", "READ", fn _ -> true end)
  end

  test "grant composition is OR, wildcard includes anonymous, and write has no implicit read" do
    {:ok, policy} =
      policy("""
      [permissions * [Post [READ [when true]]]]
      [permissions User [Post [UPDATE [when true]] [UPDATE [when false]]]]
      """)

    assert Permissions.allowed?(policy, "Anonymous", "Post", "READ", fn _ -> true end)
    assert Permissions.allowed?(policy, "User", "Post", "UPDATE", &(&1.value == "true"))
    {:ok, write_only} = policy("[permissions User [Post [UPDATE [when true]]]]")
    refute Permissions.allowed?(write_only, "User", "Post", "READ", fn _ -> true end)
  end

  test "field grants restrict entity grants for every actor" do
    {:ok, policy} =
      policy("""
      [permissions * [Post [READ [when true]]]]
      [permissions User [Post.secret [READ [when false]]]]
      """)

    assert Permissions.field_allowed?(policy, "User", "Post", "id", "READ", &(&1.value == "true"))

    refute Permissions.field_allowed?(
             policy,
             "User",
             "Post",
             "secret",
             "READ",
             &(&1.value == "true")
           )

    refute Permissions.field_allowed?(policy, "Anonymous", "Post", "secret", "READ", fn _ ->
             true
           end)

    {:ok, field_only} = policy("[permissions User [Post.secret [READ [when true]]]]")

    refute Permissions.field_allowed?(field_only, "User", "Post", "secret", "READ", fn _ ->
             true
           end)
  end

  test "cycles, nonexistent fields and change-dependent read inheritance are rejected" do
    for grants <- [
          "[permissions User [Post [READ [includes UPDATE] [when true]] [UPDATE [includes READ] [when true]]]]",
          "[permissions User [Post.typo [READ [when true]]]]",
          "[permissions User [Post [UPDATE [includes READ] [when after.secret]]]]"
        ] do
      assert {:error, %Flow.ValidationError{}} = policy(grants)
    end
  end

  test "nonboolean conditions and evaluator failures do not grant access" do
    {:ok, policy} = policy("[permissions User [Post [READ [when true]]]]")

    for evaluate <- [
          fn _ -> nil end,
          fn _ -> "true" end,
          fn _ -> {:ok, true} end,
          fn _ -> raise "missing fact" end
        ] do
      refute Permissions.allowed?(policy, "User", "Post", "READ", evaluate)
    end
  end
end
