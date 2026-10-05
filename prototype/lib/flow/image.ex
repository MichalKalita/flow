defmodule Flow.Image do
  @moduledoc false
  defstruct [:bytes, :width, :height]
  alias Vix.Vips.{Image, Operation}

  def decode!(%__MODULE__{bytes: bytes}, limits), do: decode!(bytes, limits)

  def decode!(bytes, limits) when is_binary(bytes) do
    unless byte_size(bytes) > 0 and byte_size(bytes) <= limits["maxBytes"], do: invalid!()

    case Image.new_from_buffer(bytes) do
      {:ok, image} ->
        width = Image.width(image)
        height = Image.height(image)
        unless width in 1..limits["maxWidth"] and height in 1..limits["maxHeight"], do: invalid!()
        %__MODULE__{bytes: bytes, width: width, height: height}

      _ ->
        invalid!()
    end
  end

  def decode!(_, _), do: invalid!()

  def resize!(%__MODULE__{bytes: bytes}, width, height) do
    {:ok, image} = Image.new_from_buffer(bytes)

    {:ok, resized} =
      Operation.thumbnail_image(image, width, height: height, size: :VIPS_SIZE_BOTH)

    {:ok, output} = Image.write_to_buffer(resized, ".png")
    %__MODULE__{bytes: output, width: Image.width(resized), height: Image.height(resized)}
  end

  defp invalid!,
    do:
      raise(Flow.ValidationError,
        code: :invalid_value,
        message: "Invalid image or image limits exceeded"
      )
end
