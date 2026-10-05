defmodule OrderLab.Plugins.Payment do
  @moduledoc "Local payment adapter. Generates a demo URL; contacts no payment provider."
  def call(input, context) do
    case call(input) do
      {:error, _} = error ->
        error

      {:ok, _} = demo ->
        case System.get_env("PAYMENT_PROVIDER_URL") do
          nil ->
            demo

          url ->
            headers = [{~c"idempotency-key", String.to_charlist(context["idempotency_key"])}]

            result =
              :httpc.request(
                :post,
                {String.to_charlist(url), headers, ~c"application/json", Jason.encode!(input)},
                [timeout: 5000, connect_timeout: 2000, autoredirect: false], body_format: :binary)

            case result do
              {:ok, {{_, status, _}, _, body}} when status in 200..299 ->
                case Jason.decode(body) do
                  {:ok, value} when is_map(value) ->
                    {:ok, value}

                  _ ->
                    raise OrderLab.ExternalUnknown,
                      message: "Payment provider returned an invalid response"
                end

              _ ->
                raise OrderLab.ExternalUnknown,
                  message: "Payment provider outcome is not confirmed"
            end
        end
    end
  end

  def call(input) do
    cond do
      input["country"] not in ["CZ", "SK", "DE", "US", "GB"] ->
        {:error,
         %{
           "code" => "payment_country_unsupported",
           "message" => "Platba není dostupná v zemi #{input["country"]}."
         }}

      input["method"] == "bank" and input["country"] == "US" ->
        {:error,
         %{
           "code" => "payment_method_unsupported",
           "message" => "Bankovní převod není pro tuto zemi dostupný."
         }}

      true ->
        {:ok,
         %{
           "url" => "/pay/#{input["order_id"]}",
           "provider" => "demo",
           "method" => input["method"]
         }}
    end
  end
end

defmodule OrderLab.Plugins.Email do
  @moduledoc "Fake email adapter. Its request and response are saved in the admin timeline."
  def call(input) do
    if input["simulate_failure"] do
      {:error,
       %{"code" => "email_unavailable", "message" => "Simulovaná chyba emailového pluginu."}}
    else
      {:ok,
       %{
         "message_id" => "mail_#{input["order_id"]}",
         "delivery" => "simulated",
         "to" => input["to"]
       }}
    end
  end
end
