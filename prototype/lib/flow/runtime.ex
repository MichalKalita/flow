defmodule Flow.Runtime do
  @moduledoc "Runs compiled Flow operations with verified credentials and atomic permission enforcement."
  use GenServer

  alias Flow.{
    Program,
    Store,
    Auth,
    Access,
    Value,
    Evaluator,
    Transaction,
    Projection,
    Plugins,
    Queue,
    Live
  }

  def start_link(options), do: GenServer.start_link(__MODULE__, options)

  def execute(runtime, operation, input, credential \\ nil, options \\ []) do
    options = Keyword.validate!(options, transport: "HTTP")

    GenServer.call(
      runtime,
      {:execute, operation, input, credential, options[:transport]},
      :infinity
    )
  end

  def program(runtime), do: GenServer.call(runtime, :program)

  @impl true
  def init(options) do
    with {:ok, program} <-
           Program.compile(Keyword.fetch!(options, :source),
             auth: Keyword.get(options, :auth, %{})
           ),
         :ok <- configuration(program, options),
         {:ok, store} <-
           Store.start_link(
             schema: program.schema,
             path: Keyword.get(options, :database, ":memory:"),
             trace: options[:trace]
           ) do
      handlers = Map.merge(Plugins.defaults(), Keyword.get(options, :plugins, %{}))

      try do
        for {name, contract} <- program.plugins do
          handler = handlers[name] || raise ArgumentError, "Missing plugin implementation #{name}"

          unless handler.mode == contract.mode and is_function(handler.function),
            do: raise(ArgumentError, "Plugin contract mismatch #{name}")
        end

        {:ok, :ok} =
          Store.transaction(store, fn db ->
            Plugins.initialize(db)
            Queue.initialize(db)

            for {entity, rows} <- program.seeds, row <- rows do
              reference = %Flow.Ref{entity: entity, id: row["id"]}
              unless Store.fetch(db, reference, ["id"]), do: Store.insert(db, entity, row)
            end

            for {entity, rows} <- Keyword.get(options, :identities, %{}), row <- rows do
              row = Value.validate!(program.schema, {:record, Store.fields(db, entity)}, row)
              reference = %Flow.Ref{entity: entity, id: row["id"]}
              unless Store.fetch(db, reference, ["id"]), do: Store.insert(db, entity, row)
            end

            :ok
          end)

        {:ok,
         %{
           program: program,
           store: store,
           handlers: handlers,
           context_provider: Keyword.get(options, :context, fn _, _ -> %{} end),
           queue_key: Keyword.get(options, :queue_key, :crypto.strong_rand_bytes(32))
         }}
      rescue
        error ->
          GenServer.stop(store)
          {:stop, error}
      end
    else
      {:error, error} -> {:stop, error}
    end
  end

  defp configuration(program, options) do
    try do
      for {name, adapter} <- program.auth.adapters, adapter.kind == :jwt do
        algorithm = Map.get(adapter.config, :algorithm, "HS256")
        key = Map.fetch!(adapter.config, :key)

        unless algorithm in ~w(HS256 RS256),
          do: raise(ArgumentError, "Unsupported JWT algorithm for #{name}")

        if algorithm == "HS256" and not (is_binary(key) and byte_size(key) >= 32),
          do: raise(ArgumentError, "JWT HMAC key must have at least 32 bytes")
      end

      if key = options[:queue_key],
        do:
          unless(is_binary(key) and byte_size(key) == 32,
            do: raise(ArgumentError, "queue_key must have 32 bytes")
          )

      :ok
    rescue
      error -> {:error, error}
    end
  end

  @impl true
  def handle_call(:program, _from, state), do: {:reply, state.program, state}

  def handle_call({:execute, name, input, credential, transport}, _from, state) do
    operation = state.program.operations[name]

    result =
      if operation do
        Store.transaction(state.store, fn db ->
          principal = authenticate!(state, db, transport, credential)
          inputs = inputs!(state.program.schema, operation, input)

          context =
            Flow.Context.build!(
              state.program.schema,
              principal,
              transport,
              state.context_provider
            )

          Access.with_session(db, state.program.permissions, principal, context, fn session ->
            :ets.insert(
              session.cache,
              {:credential,
               %{
                 "transport" => transport,
                 "adapter" => if(credential, do: elem(credential, 0)),
                 "secret" =>
                   if(
                     credential &&
                       state.program.auth.adapters[elem(credential, 0)].kind != :certificate,
                     do: elem(credential, 1)
                   ),
                 "certificate_binding" =>
                   if(
                     Map.get(principal, :binding) &&
                       state.program.auth.adapters[principal.adapter].kind == :certificate,
                     do: principal.binding
                   )
               }}
            )

            output = Evaluator.run(session, state.program, operation, inputs, state.handlers)
            Transaction.authorize!(session)

            result =
              case output do
                %Live{values: values} ->
                  Enum.map(values, &Projection.project(session, operation.output, &1))

                value ->
                  Projection.project(session, operation.output, value)
              end

            Transaction.apply!(session)
            Plugins.persist!(session)
            Queue.persist!(session, state.queue_key)
            result
          end)
        end)
      else
        {:error, %Flow.ValidationError{code: :unknown_operation, message: "Unknown operation"}}
      end

    {:reply, result, state}
  end

  defp authenticate!(state, db, transport, credential) do
    case Auth.authenticate(state.program.auth, transport, credential, fn entity, field, value ->
           Store.lookup(db, entity, field, value)
         end) do
      {:ok, principal} ->
        principal

      {:error, _} ->
        raise Flow.ValidationError, code: :unauthenticated, message: "Authentication required"
    end
  end

  defp inputs!(schema, operation, input) when is_map(input) and not is_struct(input) do
    extras = Map.keys(input) -- Map.keys(operation.inputs)

    if extras != [],
      do: raise(Flow.ValidationError, code: :invalid_value, message: "Unknown input fields")

    Map.new(operation.inputs, fn {name, definition} ->
      value =
        case Map.fetch(input, name) do
          {:ok, value} ->
            Value.validate!(schema, definition.type, value)

          :error ->
            case definition.default do
              {:default, value} ->
                value

              :required ->
                raise Flow.ValidationError, code: :invalid_value, message: "Missing input #{name}"
            end
        end

      {name, value}
    end)
  end

  defp inputs!(_, _, _),
    do: raise(Flow.ValidationError, code: :invalid_value, message: "Expected input record")

  @impl true
  def terminate(_reason, state) do
    if Process.alive?(state.store), do: GenServer.stop(state.store)
  end
end
