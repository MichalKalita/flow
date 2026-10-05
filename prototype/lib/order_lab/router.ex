defmodule OrderLab.Router do
  use Plug.Router
  plug(:match)
  plug(:dispatch)

  get "/health" do
    send_json(conn, 200, %{"status" => "ok"})
  end

  get "/" do
    conn |> put_resp_header("location", "/admin") |> send_resp(302, "")
  end

  get "/admin" do
    send_file(conn |> put_resp_content_type("text/html"), 200, asset("index.html"))
  end

  get "/assets/:file" do
    if file in ["app.js", "style.css"] do
      type = if file == "app.js", do: "text/javascript", else: "text/css"
      send_file(conn |> put_resp_content_type(type), 200, asset(file))
    else
      send_json(conn, 404, %{"error" => "not_found"})
    end
  end

  get "/api/admin" do
    send_json(conn, 200, OrderLab.Store.snapshot())
  end

  get "/api/requests/:id" do
    case OrderLab.Store.detail(id) do
      nil -> send_json(conn, 404, %{"error" => "not_found"})
      result -> send_json(conn, 200, result)
    end
  end

  post "/api/email-jobs/:id/retry" do
    case OrderLab.Store.retry_email(id) do
      :ok -> send_json(conn, 202, %{"state" => "queued"})
      :not_found -> send_json(conn, 404, %{"error" => "failed_job_not_found"})
    end
  end

  get "/pay/:id" do
    case OrderLab.Store.order(id) do
      nil ->
        send_json(conn, 404, %{"error" => "order_not_found"})

      order ->
        html = """
        <!doctype html><html lang="cs"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
        <link rel="stylesheet" href="/assets/style.css"><body class="payment-page"><main class="payment-card">
        <span class="eyebrow">DEMO PLATEBNÍ BRÁNA</span><h1>Objednávka připravena.</h1>
        <p>Částka: <strong>#{div(order["total_cents"], 100)} Kč</strong></p>
        <p>Tento odkaz vytvořil lokální plugin. Žádná skutečná platba se neprovádí.</p>
        <a class="button primary" href="/admin">Zpět do Order Lab</a></main></body></html>
        """

        conn |> put_resp_content_type("text/html") |> send_resp(200, html)
    end
  end

  get "/files/:id" do
    case OrderLab.Store.file(id) do
      {:ok, file} ->
        conn
        |> put_resp_content_type(file["media_type"])
        |> put_resp_header("x-content-type-options", "nosniff")
        |> put_resp_header(
          "content-disposition",
          "inline; filename*=UTF-8''" <> URI.encode(file["name"], &URI.char_unreserved?/1)
        )
        |> send_resp(200, Base.decode64!(file["data"]))

      {:error, error} ->
        send_json(conn, if(error["code"] == "file_not_found", do: 404, else: 500), %{
          "error" => error
        })
    end
  end

  post "/api/mqtt-outbox/:id/retry" do
    case OrderLab.Store.retry_mqtt(id) do
      {:ok, state} -> send_json(conn, 202, %{"id" => id, "state" => state})
      :not_found -> send_json(conn, 404, %{"error" => "failed_publish_not_found"})
    end
  end

  post "/api/language/check" do
    case read_body(conn, length: 256_000) do
      {:ok, body, conn} ->
        try do
          program =
            OrderLab.Language.Compiler.compile!(
              body,
              "<request>",
              OrderLab.Language.Native.operations()
            )

          send_json(conn, 200, %{
            "valid" => true,
            "http" =>
              Enum.map(
                Enum.filter(program.endpoints, &(&1.method != "MQTT")),
                &%{"method" => &1.method, "path" => &1.path}
              ),
            "mqtt" => Map.keys(program.mqtt),
            "websocket" =>
              Enum.map(program.websockets, &%{"path" => &1.path, "sources" => &1.sources})
          })
        rescue
          e in OrderLab.Language.Error ->
            send_json(conn, 422, %{
              "valid" => false,
              "diagnostic" => OrderLab.Language.Error.diagnostic(e)
            })

          e ->
            send_json(conn, 422, %{
              "valid" => false,
              "diagnostic" => %{"message" => Exception.message(e)}
            })
        end

      _ ->
        send_json(conn, 413, %{"error" => "body_too_large"})
    end
  end

  match _ do
    websocket = if conn.method == "GET", do: OrderLab.Store.websocket(conn.request_path)

    if websocket do
      try do
        WebSockAdapter.upgrade(conn, OrderLab.WebSocket, websocket,
          timeout: 60_000,
          max_frame_size: 8192
        )
      rescue
        _ -> send_json(conn, 400, %{"error" => "websocket_upgrade_required"})
      end
    else
      case OrderLab.Store.route(conn.method, conn.request_path) do
        nil -> send_json(conn, 404, %{"error" => "not_found"})
        {endpoint, params} -> handle_scenario(conn, endpoint, params)
      end
    end
  end

  defp handle_scenario(conn, endpoint, params) do
    conn = fetch_query_params(conn)

    if conn.method in ["GET", "DELETE"] do
      input =
        Map.merge(conn.query_params, params)
        |> Map.new(fn {key, value} ->
          declaration = Enum.find(endpoint.inputs, &(&1.name == key))
          type = if declaration, do: query_type(declaration.type)

          parsed =
            if type == {:named, "String"} or is_nil(type) do
              value
            else
              case Jason.decode(value) do
                {:ok, decoded} -> decoded
                _ -> value
              end
            end

          {key, parsed}
        end)

      dispatch_scenario(conn, endpoint, input)
    else
      body_limit =
        if Enum.any?(endpoint.inputs, &OrderLab.Language.Types.has_file?(&1.type)),
          do: OrderLab.FileValue.max_body_bytes(),
          else: 64_000

      case read_body(conn, length: body_limit, read_length: body_limit) do
        {:ok, body, conn} ->
          handle_json(conn, endpoint, params, body)

        {:more, body, conn} ->
          reject(
            conn,
            Map.put(raw_input(body), "truncated", true),
            413,
            "body_too_large",
            "Tělo požadavku překračuje limit #{body_limit} bajtů."
          )

        {:error, _} ->
          reject(conn, %{}, 400, "body_unreadable", "Tělo požadavku se nepodařilo přečíst.")
      end
    end
  end

  defp query_type({:refined, type, _}), do: query_type(type)
  defp query_type({:optional, type}), do: query_type(type)
  defp query_type(type), do: type

  defp handle_json(conn, endpoint, params, body) do
    content_type = get_req_header(conn, "content-type") |> List.first() || ""

    if content_type |> String.split(";") |> List.first() |> String.trim() == "application/json" do
      case Jason.decode(body) do
        {:ok, input} ->
          dispatch_scenario(
            conn,
            endpoint,
            if(is_map(input), do: Map.merge(input, params), else: input)
          )

        {:error, _} ->
          reject(conn, raw_input(body), 400, "invalid_json", "Požadavek neobsahuje platný JSON.")
      end
    else
      reject(
        conn,
        raw_input(body),
        415,
        "json_required",
        "Použijte Content-Type application/json."
      )
    end
  end

  defp dispatch_scenario(conn, endpoint, input) do
    key = get_req_header(conn, "idempotency-key") |> List.first()

    if key && (byte_size(key) > 128 or key == "") do
      reject(
        conn,
        input,
        422,
        "invalid_idempotency_key",
        "Idempotency-Key musí mít 1 až 128 bajtů."
      )
    else
      {status, result} = OrderLab.Store.execute(endpoint, input, conn.request_path, key)
      send_json(conn, status, result)
    end
  end

  defp asset(file), do: :order_lab |> :code.priv_dir() |> Path.join("static/#{file}")

  defp raw_input(body) do
    if String.valid?(body),
      do: %{"raw_body" => body},
      else: %{"raw_body_base64" => Base.encode64(body)}
  end

  defp reject(conn, input, status, code, message) do
    {status, response} =
      OrderLab.Store.reject(input, status, code, message, conn.method, conn.request_path)

    send_json(conn, status, response)
  end

  defp send_json(conn, status, body) do
    conn
    |> put_resp_content_type("application/json")
    |> put_resp_header("cache-control", "no-store")
    |> send_resp(status, Jason.encode!(body))
  end
end
