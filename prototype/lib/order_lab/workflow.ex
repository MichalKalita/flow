defmodule OrderLab.Workflow do
  @moduledoc "Small, closed declarative DSL. Only registered operations can execute."
  @operations %{
    "SNAPSHOT Products AS order" => :snapshot,
    "DECREASE Inventory FROM order.items" => :decrease,
    "CALL Payment.create_url WITH order AS payment" => :payment,
    "QUEUE Email.send_confirmation WITH order, payment" => :email,
    "COMMIT" => :commit
  }
  @headers ["REST POST /api/orders", "INPUT user_id User.ID", "INPUT items List<OrderItem>",
            "INPUT payment_method PaymentMethod", "INPUT email_failure Bool = false"]

  def source do
    :order_lab |> :code.priv_dir() |> Path.join("workflows/create_order.flow") |> File.read!()
  end

  def compile! do
    lines = source() |> String.split("\n") |> Enum.map(&String.trim/1)
      |> Enum.reject(&(&1 == "" or String.starts_with?(&1, "#")))
    {headers, body} = Enum.split(lines, length(@headers))
    unless headers == @headers, do: raise("Invalid workflow input contract")
    case body do
      ["TRANSACTION" | rest] ->
        unless List.last(rest) == "RETURN order, payment", do: raise("Missing typed RETURN")
        steps = rest |> Enum.drop(-1) |> Enum.map(fn line ->
          Map.get(@operations, line) || raise("Unknown workflow operation: #{line}")
        end)
        # This first language version exposes exactly this transaction-safe order.
        unless steps == [:snapshot, :decrease, :payment, :email, :commit],
          do: raise("Invalid dependency or transaction boundary")
        steps
      _ -> raise("Missing TRANSACTION")
    end
  end

  def run(steps, context, handler) do
    Enum.reduce_while(steps, {:ok, context}, fn step, {:ok, context} ->
      case handler.(step, context) do
        {:ok, next} -> {:cont, {:ok, next}}
        {:error, error, next} -> {:halt, {:error, error, next}}
      end
    end)
  end
end
