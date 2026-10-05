defmodule Flow.ExpressionTest do
  use ExUnit.Case, async: true
  alias Flow.{Expression, Parser, ID, Ref}

  defp expression(source) do
    %{value: [node]} = Parser.parse!(source)
    node
  end

  test "quantified ownership and grant predicates use strict booleans" do
    node =
      expression(
        "[any target.access access [and [eq access.user actor] [contains access.rights READ]]]"
      )

    assert :ok = Expression.validate(node)

    env = %{
      "actor" => "u1",
      "target" => %{
        "access" => [
          %{"user" => "u2", "rights" => ["READ"]},
          %{"user" => "u1", "rights" => ["READ"]}
        ]
      }
    }

    assert Expression.evaluate(node, env, constants: %{"READ" => "READ"})
    refute Expression.evaluate(node, Map.put(env, "actor", "u3"), constants: %{"READ" => "READ"})

    assert_raise ArgumentError, fn ->
      Expression.evaluate(expression("[and \"yes\" true]"), %{})
    end
  end

  test "transactional stock predicates retain exact arithmetic and typed references" do
    product = %Ref{entity: "Product", id: %ID{entity: "Product", value: "p1"}}

    node =
      expression("""
      [and [only changed stock] [gt before.stock after.stock]
        [eq [sub before.stock after.stock]
          [sum [where [flatMap [creates transaction Order] order order.items]
            item [eq item.product target]] item item.quantity]]]
      """)

    assert :ok = Expression.validate(node)

    env = %{
      "changed" => ["stock"],
      "before" => %{"stock" => 10},
      "after" => %{"stock" => 7},
      "target" => product,
      "transaction" => :transaction
    }

    creates = fn :transaction, "Order" ->
      [%{"items" => [%{"product" => product, "quantity" => 3}]}]
    end

    assert Expression.evaluate(node, env, creates: creates)

    refute Expression.evaluate(node, Map.put(env, "changed", ["stock", "price"]),
             creates: creates
           )
  end

  test "can recursion shares a budget and cannot run unbounded" do
    node = expression("[can READ target]")
    counter = :counters.new(1, [])

    recurse = fn recurse, actor, _action, target, shared ->
      Expression.evaluate(node, %{"actor" => actor, "target" => target},
        counter: shared,
        max_steps: 30,
        can: fn a, b, c, d -> recurse.(recurse, a, b, c, d) end
      )
    end

    assert_raise Flow.ValidationError, ~r/step budget/, fn ->
      Expression.evaluate(node, %{"actor" => "u1", "target" => "p1"},
        counter: counter,
        max_steps: 30,
        can: fn a, b, c, d -> recurse.(recurse, a, b, c, d) end
      )
    end
  end

  test "quantifier work is budgeted and short circuiting avoids unnecessary access" do
    env = %{"items" => Enum.to_list(1..100)}

    assert_raise Flow.ValidationError, fn ->
      Expression.evaluate(expression("[sum items item item]"), env, max_steps: 20)
    end

    refute Expression.evaluate(expression("[and false missing.secret]"), %{})
    assert Expression.evaluate(expression("[or true missing.secret]"), %{})
  end

  test "equality cannot confuse ID brands and compares nested decimals exactly" do
    refute Expression.equal?(%ID{entity: "User", value: "1"}, %ID{entity: "Order", value: "1"})
    assert Expression.equal?(%{"values" => [Decimal.new("1.00")]}, %{"values" => [1]})
    refute Expression.equal?(%{"a" => 1}, %{"a" => 1, "secret" => 2})
  end

  test "unknown operators, malformed forms and actions are rejected" do
    for source <- [
          "[shell \"ls\"]",
          "[eq 1]",
          "[can ADMIN target]",
          "[any xs [bad] true]",
          "[only target stock]",
          "[record [a 1] [a 2]]"
        ] do
      assert {:error, %Flow.ValidationError{}} = Expression.validate(expression(source))
    end
  end
end
