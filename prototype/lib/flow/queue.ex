defmodule Flow.Queue do
  @moduledoc "Transactional encrypted outbox. Retries must reauthenticate and reauthorize the original actor."
  alias Flow.{Syntax, Types, Checker, ID, Transaction, Store}

  def stage(session, method, args, retry, reads \\ []) do
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
      delay: Checker.duration!(options["delay"]),
      reads: Flow.Reads.encode(reads)
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
        "reads" => job.reads,
        "creates" => proof,
        "updates" =>
          Enum.filter(Transaction.changes(session), &(&1.action == "UPDATE"))
          |> Enum.map(fn change ->
            before =
              Map.new(change.changed, fn field ->
                {field,
                 Flow.Access.raw_field(
                   session,
                   %Flow.Version{reference: change.reference, state: :before},
                   field
                 )}
              end)

            %{
              "entity" => change.reference.entity,
              "id" => change.reference.id.value,
              "before" => before,
              "after" => change.after
            }
          end)
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

  def work(db, runtime) do
    jobs =
      Store.query(
        db,
        "SELECT id,attempts,max_attempts,delay,nonce,tag,payload FROM _flow_jobs WHERE state IN ('QUEUED','RETRY') AND next_at <= ? ORDER BY next_at,id LIMIT 100",
        [System.system_time(:millisecond)]
      )

    Enum.map(jobs, fn [id, attempts, maximum, delay, nonce, tag, cipher] ->
      Store.query(db, "SAVEPOINT flow_job", [])

      outcome =
        try do
          payload = decode!(runtime.queue_key, id, nonce, tag, cipher)
          credential = payload["credential"]
          lookup = fn entity, field, value -> Store.lookup(db, entity, field, value) end
          principal = reauthenticate!(runtime.program.auth, credential, lookup, runtime.clock.())

          context =
            Flow.Context.build!(
              runtime.program.schema,
              principal,
              credential["transport"],
              runtime.context_provider,
              runtime.clock.()
            )

          Flow.Access.with_session(
            db,
            runtime.program.permissions,
            principal,
            context,
            fn session ->
              Transaction.restore_proof(session, payload)
              Flow.Reads.authorize!(session, payload["reads"] || [])
              :ets.insert(session.cache, {:credential, credential})
              method = payload["method"]
              contract = Map.fetch!(runtime.program.plugins, method)

              input_type =
                {:record,
                 Map.new(contract.inputs, fn {name, input} ->
                   {name, %{type: input.type, options: %{}}}
                 end)}

              restored_args =
                Flow.Codec.decode(runtime.program.schema, input_type, payload["args"])
                |> Flow.Codec.unembed()

              args = Transaction.invocation(session, method, restored_args, contract)
              Transaction.authorize!(session)
              handler = Map.fetch!(runtime.handlers, method)

              result =
                case contract.mode do
                  :transactional ->
                    handler.function.(session, args)

                  :external ->
                    if is_function(handler.function, 2),
                      do: handler.function.(args, %{idempotency_key: id}),
                      else: handler.function.(args)

                  :pure ->
                    handler.function.(args)
                end

              Flow.Value.validate!(runtime.program.schema, contract.output, result)
              Transaction.authorize!(session)
              Transaction.apply!(session)
              Flow.Plugins.persist!(session)
              persist!(session, runtime.queue_key)
              :done
            end
          )
        rescue
          error in Flow.ValidationError -> {:failure, error.code}
          _error -> {:failure, :plugin_failure}
        end

      attempts = attempts + 1

      case outcome do
        :done ->
          Store.query(db, "RELEASE flow_job", [])

        _ ->
          Store.query(db, "ROLLBACK TO flow_job", [])
          Store.query(db, "RELEASE flow_job", [])
      end

      {state, error} =
        case outcome do
          :done ->
            {"DONE", nil}

          {:failure, code}
          when code in [:unauthenticated, :forbidden, :invalid_job, :not_found] ->
            {"BLOCKED", Atom.to_string(code)}

          {:failure, code} ->
            {if(attempts >= maximum, do: "RETAINED", else: "RETRY"), Atom.to_string(code)}
        end

      Store.query(
        db,
        "UPDATE _flow_jobs SET state = ?, attempts = ?, next_at = ?, error = ? WHERE id = ?",
        [state, attempts, System.system_time(:millisecond) + delay, error, id]
      )

      %{id: id, state: state, attempts: attempts, error: error}
    end)
  end

  defp reauthenticate!(auth, credential, lookup, now) do
    result =
      if binding = credential["certificate_binding"] do
        # This evidence was sealed by the runtime after a verified TLS handshake.
        # It is accepted only here, never by the client credential API.
        name = credential["adapter"]
        adapter = auth.adapters[name]

        if adapter && adapter.kind == :certificate &&
             name in Map.get(auth.transports, credential["transport"], []) do
          case lookup.(adapter.entity, adapter.field, binding) do
            nil ->
              {:error, :unauthenticated}

            identity ->
              {:ok,
               %{
                 type: adapter.entity,
                 identity: identity,
                 adapter: name,
                 binding: binding,
                 facts: %{}
               }}
          end
        else
          {:error, :unauthenticated}
        end
      else
        value =
          if credential["adapter"], do: {credential["adapter"], credential["secret"]}, else: nil

        Flow.Auth.authenticate(auth, credential["transport"], value, lookup,
          now: DateTime.to_unix(now)
        )
      end

    case result do
      {:ok, principal} ->
        principal

      _ ->
        raise Flow.ValidationError,
          code: :unauthenticated,
          message: "Queued identity no longer authenticated"
    end
  end
end
