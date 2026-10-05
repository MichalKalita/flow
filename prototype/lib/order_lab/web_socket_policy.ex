defmodule OrderLab.WebSocketPolicy do
  alias OrderLab.Language.{Types, Evaluator, Failure}

  def authenticate!(db, program, endpoint, input) do
    unless is_map(input), do: deny!("invalid_credentials", "Credentials must be an object")
    names = Enum.map(endpoint.inputs, & &1.name)

    if Map.keys(input) -- names != [],
      do: deny!("invalid_credentials", "Unknown credential fields")

    credentials =
      Enum.reduce(endpoint.inputs, %{}, fn declaration, env ->
        value =
          case Map.fetch(input, declaration.name) do
            {:ok, value} ->
              value

            :error ->
              if declaration.default != :missing,
                do: Evaluator.eval(declaration.default, env),
                else: deny!("invalid_credentials", "Missing credential #{declaration.name}")
          end

        Map.put(env, declaration.name, Types.validate!(value, declaration.type))
      end)

    if not allowed?(db, program, endpoint.authorization, credentials),
      do: deny!("unauthorized", "Invalid credentials")

    credentials
  end

  def subscription!(db, program, endpoint, credentials, source, params) do
    unless source in endpoint.sources, do: deny!("source_not_allowed", "Source is not allowed")

    if not allowed?(db, program, endpoint.authorization, credentials),
      do: deny!("unauthorized", "Credentials are no longer valid")

    unless allowed?(
             db,
             program,
             Map.fetch!(endpoint.policies, source),
             Map.merge(credentials, params)
           ),
           do: deny!("forbidden", "Access to this device is denied")

    :ok
  end

  defp allowed?(db, program, expression, env) do
    env = Map.put(env, "$source_names", Map.keys(program.mqtt))

    Evaluator.eval(expression, env, fn name ->
      OrderLab.Language.Native.source(
        db,
        program,
        name,
        DateTime.utc_now() |> DateTime.to_iso8601()
      )
    end)
    |> Evaluator.boolean!()
  end

  defp deny!(code, message), do: raise(Failure, status: 403, code: code, message: message)
end
