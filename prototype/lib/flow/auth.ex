defmodule Flow.Auth do
  @moduledoc "Credential verification followed by a unique domain identity lookup."
  alias Flow.Syntax, as: S
  defstruct adapters: %{}, transports: %{}

  def compile(schema, configuration \\ %{}) do
    try do
      adapters =
        case schema.auth do
          nil ->
            %{}

          node ->
            {"auth", definitions} = S.form(node)

            definitions
            |> S.unique(fn definition -> elem(S.form(definition), 0) end)
            |> Map.new(fn {name, node} ->
              {name, adapter!(schema, node, configuration[name] || %{})}
            end)
        end

      transports =
        Map.new(schema.transports, fn {name, node} ->
          option =
            case S.form(node) do
              {"transport", [_, option]} -> option
              _ -> S.fail(node, :invalid_auth, "Transport requires one auth declaration")
            end

          aliases =
            case S.form(option) do
              {"auth", aliases} -> aliases
              _ -> S.fail(option, :invalid_auth, "Expected transport auth declaration")
            end

          aliases = Enum.map(aliases, &S.symbol/1)

          if aliases == [] or Enum.any?(aliases, &(not Map.has_key?(adapters, &1))),
            do: S.fail(node, :invalid_auth, "Transport references an unknown auth adapter")

          {name, aliases}
        end)

      {:ok, %__MODULE__{adapters: adapters, transports: transports}}
    rescue
      error in Flow.ValidationError -> {:error, error}
    end
  end

  def authenticate(auth, transport, credential, lookup, options \\ []) do
    try do
      aliases = Map.fetch!(auth.transports, transport)

      case credential do
        nil ->
          name = Enum.find(aliases, &(auth.adapters[&1].kind == :anonymous)) || invalid!()
          {:ok, %{type: "Anonymous", identity: nil, adapter: name}}

        {name, secret} when is_binary(name) ->
          unless name in aliases, do: invalid!()
          adapter = Map.fetch!(auth.adapters, name)
          value = verify!(adapter, secret, options)
          identity = lookup.(adapter.entity, adapter.field, value)
          if identity == nil, do: invalid!()
          {:ok, %{type: adapter.entity, identity: identity, adapter: name}}

        _ ->
          invalid!()
      end
    rescue
      _ -> {:error, :unauthenticated}
    catch
      _, _ -> {:error, :unauthenticated}
    end
  end

  defp adapter!(schema, node, config) do
    {_, args} = S.form(node)

    case args do
      [%{kind: :symbol, value: "Anonymous"}] ->
        %{kind: :anonymous}

      [entity, credential, binding] ->
        entity = S.identifier(entity)

        model =
          schema.entities[entity] ||
            S.fail(node, :invalid_auth, "Auth requires a declared entity")

        {kind, options} = S.form(credential)

        {claim, method} =
          case kind do
            "jwt" -> {"claims.sub", :jwt}
            "apiKey" -> {"credential.hash", :api_key}
            "certificate" -> {"certificate.fingerprint", :certificate}
            _ -> S.fail(credential, :invalid_auth, "Unsupported credential adapter")
          end

        [predicate] = S.args(binding, "entity", 1)
        [path, source] = S.args(predicate, "eq", 2)

        field =
          case String.split(S.symbol(path), ".") do
            [^entity, field] -> field
            _ -> S.fail(path, :invalid_auth, "Auth lookup must reference its entity field")
          end

        unless S.symbol(source) == claim,
          do: S.fail(source, :invalid_auth, "Unsupported credential binding")

        definition =
          model.fields[field] || S.fail(path, :invalid_auth, "Unknown auth lookup field")

        unless definition.options["unique"],
          do: S.fail(path, :invalid_auth, "Identity lookup field must be unique")

        unless definition.type in [{:named, "String"}, {:optional, {:named, "String"}}],
          do: S.fail(path, :invalid_auth, "Identity lookup requires a string field")

        settings =
          if method == :jwt do
            options = S.options(options, ~w(issuer audience))

            Map.new(~w(issuer audience), fn key ->
              option = options[key] || S.fail(credential, :invalid_auth, "JWT requires #{key}")
              [value] = S.args(option, key, 1)
              {key, S.string(value)}
            end)
          else
            if options != [],
              do: S.fail(credential, :invalid_auth, "Credential adapter takes no options")

            %{}
          end

        %{kind: method, entity: entity, field: field, settings: settings, config: config}

      _ ->
        S.fail(node, :invalid_auth, "Malformed auth adapter")
    end
  end

  defp verify!(%{kind: :api_key}, secret, _)
       when is_binary(secret) and byte_size(secret) in 32..4096,
       do: :crypto.hash(:sha256, secret) |> Base.encode16(case: :lower)

  defp verify!(%{kind: :certificate, config: config}, connection, _) do
    # The server injects this extractor for a verified TLS connection. Request
    # bodies, headers and DSL values never supply certificate facts.
    extractor = Map.fetch!(config, :verified_certificate)

    case extractor.(connection) do
      {:ok, der} when is_binary(der) -> :crypto.hash(:sha256, der) |> Base.encode16(case: :lower)
      _ -> invalid!()
    end
  end

  defp verify!(%{kind: :jwt} = adapter, token, options)
       when is_binary(token) and byte_size(token) <= 65_536 do
    [header, payload, signature] = String.split(token, ".")
    protected = decode_json!(header)
    claims = decode_json!(payload)
    algorithm = Map.get(adapter.config, :algorithm, "HS256")
    unless protected["alg"] == algorithm and not Map.has_key?(protected, "crit"), do: invalid!()
    signed = header <> "." <> payload
    signature = decode!(signature)

    valid =
      case algorithm do
        "HS256" ->
          key = Map.fetch!(adapter.config, :key)
          unless is_binary(key) and byte_size(key) >= 32, do: invalid!()
          Plug.Crypto.secure_compare(signature, :crypto.mac(:hmac, :sha256, key, signed))

        "RS256" ->
          :public_key.verify(signed, :sha256, signature, Map.fetch!(adapter.config, :key))

        _ ->
          false
      end

    unless valid, do: invalid!()
    now = Keyword.get_lazy(options, :now, fn -> System.system_time(:second) end)
    unless claims["iss"] == adapter.settings["issuer"], do: invalid!()
    audience = claims["aud"]

    unless audience == adapter.settings["audience"] or
             (is_list(audience) and adapter.settings["audience"] in audience),
           do: invalid!()

    unless is_integer(claims["exp"]) and claims["exp"] > now, do: invalid!()

    if Map.has_key?(claims, "nbf") and not (is_integer(claims["nbf"]) and claims["nbf"] <= now),
      do: invalid!()

    unless is_binary(claims["sub"]) and byte_size(claims["sub"]) in 1..256, do: invalid!()
    claims["sub"]
  end

  defp verify!(_, _, _), do: invalid!()

  defp decode_json!(part) do
    value = part |> decode!() |> Jason.decode!(floats: :decimals)
    unless is_map(value), do: invalid!()
    value
  end

  defp decode!(part) do
    unless Regex.match?(~r/\A[A-Za-z0-9_-]+\z/, part), do: invalid!()

    case Base.url_decode64(part, padding: false) do
      {:ok, value} -> value
      _ -> invalid!()
    end
  end

  defp invalid!, do: raise(ArgumentError, "Invalid credentials")
end
