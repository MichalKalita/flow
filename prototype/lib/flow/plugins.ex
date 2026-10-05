defmodule Flow.Plugins do
  @moduledoc "Trusted native plugin contracts. Flow source never supplies executable host code."
  alias Flow.{Transaction, Image, ID, Store}

  def defaults do
    %{
      "Payment.createUrl" => %{
        mode: :pure,
        function: fn args ->
          %{"url" => "https://payments.example.test/pay/" <> URI.encode(args["orderId"].value)}
        end
      },
      "Image.resize" => %{
        mode: :pure,
        function: fn args -> Image.resize!(args["image"], args["width"], args["height"]) end
      },
      "Files.put" => %{
        mode: :transactional,
        function: fn session, args ->
          id = %ID{entity: "File", value: Transaction.random_id()}

          reference =
            Transaction.create(session, "File", %{"id" => id, "url" => "/api/files/" <> id.value})

          :ets.insert(session.cache, {{:blob, id.value}, args["file"].bytes})
          reference
        end
      },
      "Email.sendConfirmation" => %{
        mode: :external,
        function: fn args ->
          if args["simulateFailure"], do: raise("Simulated delivery failure")
          %{"state" => "SENT"}
        end
      }
    }
  end

  def initialize(db),
    do:
      Store.query(
        db,
        "CREATE TABLE IF NOT EXISTS _flow_blobs (id TEXT PRIMARY KEY, content BLOB NOT NULL)",
        []
      )

  def persist!(session) do
    for {{:blob, id}, bytes} <- :ets.match_object(session.cache, {{:blob, :_}, :_}) do
      Store.query(session.db, "INSERT INTO _flow_blobs (id,content) VALUES (?,?)", [
        id,
        {:blob, bytes}
      ])
    end

    :ok
  end
end
