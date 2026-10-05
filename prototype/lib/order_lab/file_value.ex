defmodule OrderLab.FileValue do
  @moduledoc "Content-bound file and image values. All metadata is derived from bytes."
  alias OrderLab.Language.Error
  alias Vix.Vips.Image
  @max_bytes 10 * 1024 * 1024
  @max_pixels 20_000_000
  @fields ~w(data name media_type byte_size sha256 format width height)

  def fields("File") do
    Map.new(~w(data name media_type sha256), &{&1, {:named, "String"}})
    |> Map.put("byte_size", {:named, "Int"})
  end

  def fields("Image"),
    do:
      Map.merge(fields("File"), %{
        "format" => {:named, "String"},
        "width" => {:named, "Int"},
        "height" => {:named, "Int"}
      })

  def max_body_bytes, do: div(@max_bytes * 4 + 2, 3) + 64_000

  def normalize!(value, type) do
    unless is_map(value) and is_binary(value["data"]),
      do: invalid!("File requires an object with base64 data")

    unless Map.keys(value) -- @fields == [], do: invalid!("Unknown file fields")
    if byte_size(value["data"]) > div(@max_bytes * 4 + 2, 3), do: invalid!("File exceeds 10 MiB")

    bytes =
      case Base.decode64(value["data"]) do
        {:ok, bytes} when byte_size(bytes) <= @max_bytes -> bytes
        _ -> invalid!("File data is not valid base64 or exceeds 10 MiB")
      end

    name = Map.get(value, "name", "upload")

    unless is_binary(name) and byte_size(name) in 1..255 and not String.contains?(name, "\0"),
      do: invalid!("Invalid file name")

    metadata = %{
      "data" => Base.encode64(bytes),
      "name" => name,
      "media_type" => media_type(bytes),
      "byte_size" => byte_size(bytes),
      "sha256" => Base.encode16(:crypto.hash(:sha256, bytes), case: :lower)
    }

    image_requested =
      type == "Image" or Enum.any?(~w(format width height), &Map.has_key?(value, &1))

    metadata =
      if image_requested do
        {image, format} = decode_image!(bytes)

        Map.merge(metadata, %{
          "format" => format,
          "width" => Image.width(image),
          "height" => Image.height(image)
        })
      else
        metadata
      end

    Enum.each(Map.drop(value, ["data", "name"]), fn {field, claimed} ->
      unless metadata[field] == claimed,
        do: invalid!("Claimed #{field} does not match file content")
    end)

    metadata
  end

  def decode_image!(bytes) do
    format =
      case media_type(bytes) do
        "image/png" -> "PNG"
        "image/jpeg" -> "JPEG"
        _ -> invalid!("Expected PNG or JPEG image content")
      end

    image =
      case Image.new_from_buffer(bytes, fail_on: :VIPS_FAIL_ON_WARNING) do
        {:ok, image} -> image
        {:error, _} -> invalid!("Image decoder rejected the content")
      end

    width = Image.width(image)
    height = Image.height(image)

    unless width in 1..8192 and height in 1..8192 and width * height <= @max_pixels,
      do: invalid!("Image dimensions exceed the supported limits")

    # libvips is lazy: force every pixel, so a valid header alone is insufficient.
    case Image.write_to_binary(image) do
      {:ok, _pixels} -> :ok
      {:error, _} -> invalid!("Image pixels cannot be decoded")
    end

    {image, format}
  end

  defp media_type(<<137, 80, 78, 71, 13, 10, 26, 10, _::binary>>), do: "image/png"
  defp media_type(<<255, 216, 255, _::binary>>), do: "image/jpeg"
  defp media_type("%PDF-" <> _), do: "application/pdf"
  defp media_type(_), do: "application/octet-stream"
  defp invalid!(message), do: Error.fail!(message, stage: :validation)
end

defmodule OrderLab.Plugins.ImageResize do
  alias Vix.Vips.{Image, Operation}

  def call(input) do
    try do
      value = input["image"]
      {image, format} = OrderLab.FileValue.decode_image!(Base.decode64!(value["data"]))

      scale =
        min(1.0, min(input["width"] / Image.width(image), input["height"] / Image.height(image)))

      with {:ok, resized} <- Operation.resize(image, scale),
           {:ok, bytes} <-
             Image.write_to_buffer(resized, if(format == "PNG", do: ".png", else: ".jpg"),
               strip: true
             ) do
        {:ok,
         OrderLab.FileValue.normalize!(
           %{"data" => Base.encode64(bytes), "name" => value["name"]},
           "Image"
         )}
      else
        _ ->
          {:error,
           %{"code" => "image_resize_failed", "message" => "Image resize or encoding failed"}}
      end
    rescue
      error -> {:error, %{"code" => "image_resize_failed", "message" => Exception.message(error)}}
    end
  end
end

defmodule OrderLab.Plugins.ImageDecode do
  def call(input) do
    try do
      {:ok, OrderLab.FileValue.normalize!(input["file"], "Image")}
    rescue
      error -> {:error, %{"code" => "image_decode_failed", "message" => Exception.message(error)}}
    end
  end
end
