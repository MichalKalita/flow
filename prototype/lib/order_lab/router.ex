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

  get "/api/orders/:id" do
    case OrderLab.Store.order(id) do
      nil -> send_json(conn, 404, %{"error" => "not_found"})
      result -> send_json(conn, 200, result)
    end
  end

  post "/api/orders" do
    case read_body(conn, length: 64_000) do
      {:ok, body, conn} ->
        handle_order_body(conn, body)

      {:more, body, conn} ->
        reject(
          conn,
          Map.put(raw_input(body), "truncated", true),
          413,
          "body_too_large",
          "Tělo požadavku překračuje limit 64 kB."
        )

      {:error, _} ->
        reject(conn, %{}, 400, "body_unreadable", "Tělo požadavku se nepodařilo přečíst.")
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
            "http" => Enum.map(program.endpoints, &%{"method" => &1.method, "path" => &1.path}),
            "mqtt" => Map.keys(program.mqtt)
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
    case OrderLab.Store.route(conn.method, conn.request_path) do
      nil -> send_json(conn, 404, %{"error" => "not_found"})
      {endpoint, params} -> handle_scenario(conn, endpoint, params)
    end
  end

  defp handle_scenario(conn, endpoint, params) do
    conn = fetch_query_params(conn)

    if conn.method in ["GET", "DELETE"] do
      input = Map.merge(conn.query_params, params)

      input =
        Map.new(input, fn {key, value} ->
          declaration = Enum.find(endpoint.inputs, &(&1.name == key))
          type = if declaration, do: OrderLab.Language.Checker.base(declaration.type)

          parsed =
            case type do
              {:named, name} when name in ["Bool", "Int", "Number", "Float", "JSON"] ->
                case Jason.decode(value) do
                  {:ok, parsed} -> parsed
                  _ -> value
                end

              {:record, _} ->
                case Jason.decode(value) do
                  {:ok, parsed} -> parsed
                  _ -> value
                end

              _ ->
                value
            end

          {key, parsed}
        end)

      {status, result} = OrderLab.Store.execute(endpoint, input, conn.request_path)
      send_json(conn, status, result)
    else
      case read_body(conn, length: 64_000) do
        {:ok, body, conn} ->
          case Jason.decode(body) do
            {:ok, input} when is_map(input) ->
              {status, result} =
                OrderLab.Store.execute(endpoint, Map.merge(input, params), conn.request_path)

              send_json(conn, status, result)

            _ ->
              send_json(conn, 400, %{"error" => "invalid_json"})
          end

        _ ->
          send_json(conn, 413, %{"error" => "body_too_large"})
      end
    end
  end

  defp asset(file), do: :order_lab |> :code.priv_dir() |> Path.join("static/#{file}")

  defp handle_order_body(conn, body) do
    content_type = get_req_header(conn, "content-type") |> List.first() || ""

    if String.starts_with?(content_type, "application/json") do
      case Jason.decode(body) do
        {:ok, input} ->
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
            {status, result} = OrderLab.Store.create(input, key)
            send_json(conn, status, result)
          end

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

  defp raw_input(body) do
    if String.valid?(body),
      do: %{"raw_body" => body},
      else: %{"raw_body_base64" => Base.encode64(body)}
  end

  defp reject(conn, input, status, code, message) do
    {status, response} = OrderLab.Store.reject(input, status, code, message)
    send_json(conn, status, response)
  end

  defp send_json(conn, status, body) do
    conn
    |> put_resp_content_type("application/json")
    |> put_resp_header("cache-control", "no-store")
    |> send_resp(status, Jason.encode!(body))
  end
end
