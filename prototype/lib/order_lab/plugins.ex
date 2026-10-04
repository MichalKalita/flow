defmodule OrderLab.Plugins.Payment do
  @moduledoc "Local payment adapter. Generates a demo URL; contacts no payment provider."
  def call(input) do
    cond do
      input["country"] not in ["CZ", "SK", "DE", "US", "GB"] ->
        {:error, %{"code" => "payment_country_unsupported", "message" => "Platba není dostupná v zemi #{input["country"]}."}}
      input["method"] == "bank" and input["country"] == "US" ->
        {:error, %{"code" => "payment_method_unsupported", "message" => "Bankovní převod není pro tuto zemi dostupný."}}
      true ->
        {:ok, %{"url" => "/pay/#{input["order_id"]}", "provider" => "demo", "method" => input["method"]}}
    end
  end
end

defmodule OrderLab.Plugins.Email do
  @moduledoc "Fake email adapter. Its request and response are saved in the admin timeline."
  def call(input) do
    if input["simulate_failure"] do
      {:error, %{"code" => "email_unavailable", "message" => "Simulovaná chyba emailového pluginu."}}
    else
      {:ok, %{"message_id" => "mail_#{input["order_id"]}", "delivery" => "simulated", "to" => input["to"]}}
    end
  end
end
