defmodule Flow.SchemaTest do
  use ExUnit.Case, async: true
  alias Flow.{Schema, Value, ID}

  test "application data model compiles without losing exact prices" do
    source = File.read!(Path.expand("../priv/workflows/application.flow", __DIR__))
    assert {:ok, schema} = Schema.compile(source)
    assert {:number, "decimal", lo, hi, 2} = schema.types["Price"]
    assert Decimal.equal?(lo, Decimal.new("0.01"))
    assert Decimal.equal?(hi, Decimal.new("1000000000"))
    assert map_size(schema.operations) == 11
  end

  test "reject invalid type declarations and numeric domains" do
    for source <- [
          "[type X [integer]]",
          "[type X [decimal [range 1 2]]]",
          "[type X [integer [range 2 1]]]",
          "[type X [integer [range 0.1 2]]]",
          "[type X [list String [min 3] [max 2]]]",
          "[type X Missing]",
          "[type X String] [type X Bool]",
          "[type X Y] [type Y X]",
          "[type X [record [field a X]]]",
          "[entity X [field id String]]",
          "[type X [id Missing]]",
          "[type String Bool]",
          "[execute X]",
          "[permissions]",
          "[type X [list String]]"
        ] do
      assert {:error, %Flow.ValidationError{}} = Schema.compile(source), source
    end
  end

  test "relations must reference a matching back-reference" do
    assert {:error, %{code: :invalid_relation}} =
             Schema.compile("""
             [type AID [id A]] [type BID [id B]]
             [entity A [field id AID] [field bs [list B] [inverse B.owner]]]
             [entity B [field id BID] [field owner String]]
             """)
  end

  test "decimal input rejects rounding, zero, floats and out-of-range values" do
    schema =
      Schema.compile!("[type Price [decimal [range 0.01 999999999999999999.99] [scale 2]]]")

    assert {:ok, price} = Value.validate(schema, {:named, "Price"}, Decimal.new("123.4"))
    assert Decimal.to_string(price) == "123.40"

    for value <- [0, Decimal.new("1.001"), 1.23, Decimal.new("1e30")] do
      assert {:error, _} = Value.validate(schema, {:named, "Price"}, value)
    end

    assert {:ok, _} =
             Value.validate(schema, {:named, "Price"}, Decimal.new("999999999999999999.99"))
  end

  test "input records reject unknown and missing fields and keep brands" do
    schema =
      Schema.compile!("""
      [type AID [id A]] [type BID [id B]]
      [entity A [field id AID]] [entity B [field id BID]]
      [type Input [record [field id AID] [field label [optional String]]]]
      """)

    assert {:ok, %{"id" => %ID{entity: "A", value: "a"}, "label" => nil}} =
             Value.validate(schema, {:named, "Input"}, %{"id" => "a"})

    for input <- [%{}, %{"id" => "a", "extra" => true}, %{"id" => %ID{entity: "B", value: "b"}}] do
      assert {:error, _} = Value.validate(schema, {:named, "Input"}, input)
    end
  end

  test "bounds and nested field errors apply to actual values" do
    schema = Schema.compile!("[type Input [record [field labels [list String [min 1] [max 2]]]]]")

    assert {:error, %{message: "$.labels: List length is outside bounds"}} =
             Value.validate(schema, {:named, "Input"}, %{"labels" => []})

    assert {:error, %{message: "$.labels[0]: Expected UTF-8 string"}} =
             Value.validate(schema, {:named, "Input"}, %{"labels" => [3]})
  end
end
