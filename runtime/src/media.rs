use crate::{Error, Result};
use image::{ImageFormat, ImageReader, imageops::FilterType};
use std::io::Cursor;
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}
fn reader(bytes: &[u8], width: u32, height: u32) -> Result<ImageReader<Cursor<&[u8]>>> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| Error::new("invalid_input", "Invalid image"))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(width);
    limits.max_image_height = Some(height);
    reader.limits(limits);
    Ok(reader)
}
impl Image {
    pub fn decode(bytes: Vec<u8>, max_bytes: usize, width: u32, height: u32) -> Result<Self> {
        if bytes.len() > max_bytes {
            return Err(Error::new("invalid_input", "Image exceeds byte limit"));
        };
        let decoded = reader(&bytes, width, height)?
            .decode()
            .map_err(|_| Error::new("invalid_input", "Invalid or oversized image"))?;
        Ok(Self {
            width: decoded.width(),
            height: decoded.height(),
            bytes,
        })
    }
    pub fn resize(&self, width: u32, height: u32) -> Result<Self> {
        if width == 0 || height == 0 {
            return Err(Error::new(
                "invalid_input",
                "Image dimensions must be positive",
            ));
        };
        let image = reader(&self.bytes, self.width, self.height)?
            .decode()
            .map_err(|_| Error::new("invalid_input", "Invalid image"))?
            .resize(width, height, FilterType::Triangle);
        let mut output = Cursor::new(vec![]);
        image
            .write_to(&mut output, ImageFormat::Png)
            .map_err(|_| Error::new("invalid_input", "Cannot encode image"))?;
        Ok(Self {
            bytes: output.into_inner(),
            width: image.width(),
            height: image.height(),
        })
    }
}
