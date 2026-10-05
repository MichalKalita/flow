defmodule Flow.AuthTest do
  use ExUnit.Case, async: true
  alias Flow.{Auth, Schema}
  @key String.duplicate("secret", 8)

  setup do
    schema =
      Schema.compile!("""
      [type UserID [id User]] [type ServiceID [id Service]]
      [entity User [field id UserID] [field subject String [unique]]]
      [entity Service [field id ServiceID] [field keyHash String [unique]]]
      [auth [user User [jwt [issuer "https://issuer"] [audience "flow"]]
        [entity [eq User.subject claims.sub]]]
        [service Service [apiKey] [entity [eq Service.keyHash credential.hash]]]
        [anonymous Anonymous]]
      [transport HTTP [auth user service anonymous]]
      [transport MQTT [auth service]]
      """)

    {:ok, auth} = Auth.compile(schema, %{"user" => %{key: @key}})
    %{schema: schema, auth: auth}
  end

  defp token(claims, header \\ %{"alg" => "HS256"}, key \\ @key) do
    header = header |> Jason.encode!() |> Base.url_encode64(padding: false)
    payload = claims |> Jason.encode!() |> Base.url_encode64(padding: false)
    signed = header <> "." <> payload
    signed <> "." <> Base.url_encode64(:crypto.mac(:hmac, :sha256, key, signed), padding: false)
  end

  defp claims, do: %{"sub" => "idp:u1", "iss" => "https://issuer", "aud" => "flow", "exp" => 2000}

  test "verified subject is bound to a unique domain identity", %{auth: auth} do
    lookup = fn "User", "subject", "idp:u1" -> :user_identity end

    assert {:ok, %{type: "User", identity: :user_identity}} =
             Auth.authenticate(auth, "HTTP", {"user", token(claims())}, lookup, now: 1000)

    assert {:error, :unauthenticated} =
             Auth.authenticate(auth, "HTTP", {"user", token(claims())}, fn _, _, _ -> nil end,
               now: 1000
             )
  end

  test "invalid tokens never perform a lookup or become anonymous", %{auth: auth} do
    lookup = fn _, _, _ -> flunk("Unverified token reached identity lookup") end

    invalid = [
      token(Map.put(claims(), "iss", "other")),
      token(Map.put(claims(), "aud", "other")),
      token(Map.put(claims(), "exp", 1000)),
      token(Map.put(claims(), "exp", "2000")),
      token(Map.delete(claims(), "exp")),
      token(Map.put(claims(), "nbf", 1001)),
      token(claims(), %{"alg" => "none"}),
      token(claims(), %{"alg" => "HS256", "crit" => ["custom"]}),
      token(claims(), %{"alg" => "HS256"}, "forged"),
      "malformed"
    ]

    for token <- invalid do
      assert {:error, :unauthenticated} =
               Auth.authenticate(auth, "HTTP", {"user", token}, lookup, now: 1000)
    end

    assert {:ok, %{type: "Anonymous"}} = Auth.authenticate(auth, "HTTP", nil, lookup)

    assert {:error, :unauthenticated} =
             Auth.authenticate(auth, "HTTP", {"anonymous", "invalid"}, lookup)
  end

  test "transport cannot use a disallowed adapter", %{auth: auth} do
    lookup = fn _, _, _ -> flunk("lookup") end

    assert {:error, :unauthenticated} =
             Auth.authenticate(auth, "MQTT", {"user", token(claims())}, lookup, now: 1000)

    assert {:error, :unauthenticated} =
             Auth.authenticate(auth, "MQTT", nil, lookup)
  end

  test "API keys are hashed before lookup", %{auth: auth} do
    key = String.duplicate("api-key", 8)
    hash = :crypto.hash(:sha256, key) |> Base.encode16(case: :lower)

    lookup = fn "Service", "keyHash", value ->
      assert value == hash
      :service
    end

    assert {:ok, %{type: "Service", identity: :service}} =
             Auth.authenticate(auth, "MQTT", {"service", key}, lookup)

    assert {:error, :unauthenticated} =
             Auth.authenticate(auth, "MQTT", {"service", "short"}, lookup)
  end

  test "compiler refuses nonunique identity lookup and wrong claim bindings" do
    for binding <- ["[eq User.subject claims.sub]", "[eq User.subject claims.role]"] do
      schema =
        Schema.compile!("""
        [type UserID [id User]] [entity User [field id UserID] [field subject String]]
        [auth [user User [jwt [issuer "issuer"] [audience "audience"]] [entity #{binding}]]]
        """)

      assert {:error, %{code: :invalid_auth}} = Auth.compile(schema)
    end
  end
end
