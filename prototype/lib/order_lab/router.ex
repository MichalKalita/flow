defmodule OrderLab.Router do
  use Plug.Router
  plug :match
  plug :dispatch

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
    with true <- String.starts_with?(get_req_header(conn, "content-type") |> List.first() || "", "application/json"),
         {:ok, body, conn} <- read_body(conn, length: 64_000),
         {:ok, input} <- Jason.decode(body) do
      key = get_req_header(conn, "idempotency-key") |> List.first()
      if key && (byte_size(key) > 128 or key == "") do
        send_json(conn, 422, %{"error" => %{"code" => "invalid_idempotency_key"}})
      else
        {status, result} = OrderLab.Store.create(input, key)
        send_json(conn, status, result)
      end
    else
      false -> send_json(conn, 415, %{"error" => %{"code" => "json_required"}})
      {:more, _, conn} -> send_json(conn, 413, %{"error" => %{"code" => "body_too_large"}})
      _ -> send_json(conn, 400, %{"error" => %{"code" => "invalid_json"}})
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
      nil -> send_json(conn, 404, %{"error" => "order_not_found"})
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

  match _ do
    send_json(conn, 404, %{"error" => "not_found"})
  end

  defp asset(file), do: :order_lab |> :code.priv_dir() |> Path.join("static/#{file}")
  defp send_json(conn, status, body) do
    conn |> put_resp_content_type("application/json") |> put_resp_header("cache-control", "no-store") |> send_resp(status, Jason.encode!(body))
  end
end
