defmodule Flow.Queue do
  @moduledoc "Transactional encrypted outbox. Retries must reauthenticate and reauthorize the original actor."
  alias Flow.{Syntax, Types, Checker, ID, Transaction, Store}

  def stage(session, method, args, retry) do
    {"retry", options} = Syntax.form(retry)

    options =
      Map.new(options, fn node ->
        {name, [value]} = Syntax.form(node)
        {name, value}
      end)

    id = Transaction.random_id()

    job = %{
      id: id,
      method: method,
      args: args,
      max_attempts: Decimal.to_integer(Types.constant(options["attempts"])),
      delay: Checker.duration!(options["delay"])
    }

    :ets.insert(session.cache, {{:queue, id}, job})
    %{"id" => %ID{entity: "Job", value: id}, "state" => "QUEUED"}
  end

  def initialize(db) do
    Store.query(
      db,
      "CREATE TABLE IF NOT EXISTS _flow_jobs (id TEXT PRIMARY KEY, state TEXT NOT NULL, attempts INTEGER NOT NULL, max_attempts INTEGER NOT NULL, next_at INTEGER NOT NULL, delay INTEGER NOT NULL, nonce BLOB NOT NULL, tag BLOB NOT NULL, payload BLOB NOT NULL, error TEXT)",
      []
    )
  end

  def persist!(session, key) do
    [{_, credential}] = :ets.lookup(session.cache, :credential)
    jobs = :ets.match_object(session.cache, {{:queue, :_}, :_})

    for {_, job} <- jobs do
      proof =
        Transaction.changes(session)
        |> Enum.filter(&(&1.action == "CREATE"))
        |> Enum.map(fn change ->
          %{"entity" => change.reference.entity, "id" => change.reference.id.value}
        end)

      payload = %{
        "method" => job.method,
        "args" => job.args,
        "credential" => credential,
        "creates" => proof
      }

      encoded = Store.encode(payload)
      nonce = :crypto.strong_rand_bytes(12)

      {cipher, tag} =
        :crypto.crypto_one_time_aead(:aes_256_gcm, key, nonce, encoded, job.id, true)

      Store.query(
        session.db,
        "INSERT INTO _flow_jobs (id,state,attempts,max_attempts,next_at,delay,nonce,tag,payload) VALUES (?,?,?,?,?,?,?,?,?)",
        [
          job.id,
          "QUEUED",
          0,
          job.max_attempts,
          System.system_time(:millisecond),
          job.delay,
          {:blob, nonce},
          {:blob, tag},
          {:blob, cipher}
        ]
      )
    end

    :ok
  end

  def decode!(key, id, nonce, tag, cipher) do
    case :crypto.crypto_one_time_aead(:aes_256_gcm, key, nonce, cipher, id, tag, false) do
      :error -> raise Flow.ValidationError, code: :invalid_job, message: "Invalid encrypted job"
      plain -> Jason.decode!(plain, floats: :decimals)
    end
  end
end
