//! Images for a Jev request (images.md): PNG or JPEG only, checked locally against the documented limits before anything
//! is sent, so a request the API would refuse is never built. Nothing here decodes pixels: the size comes from the
//! header, an animated PNG is told by its `acTL` chunk, and the API stays the one that decodes. No dependency; the
//! base64 encoder is the standard alphabet with padding and no line breaks, as the API asks.

use std::fmt;

/// The largest image file (images.md: 16 MiB per image file; for base64 the size after decoding).
pub const MAX_FILE_BYTES: usize = 16 * 1024 * 1024;
/// Width and height, each, including both ends.
pub const MIN_SIDE: u32 = 280;
pub const MAX_SIDE: u32 = 1568;
/// Total pixels, including both ends (a 280 x 280 and a 1568 x 1568 square).
pub const MIN_PIXELS: u64 = 78_400;
pub const MAX_PIXELS: u64 = 2_458_624;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
}

impl ImageFormat {
    /// The `format` an asset carries: lowercase `png` or `jpg`.
    pub fn name(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
        }
    }
}

/// Why an image was refused locally. No variant holds any bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageError {
    /// Not a PNG or a JPEG.
    Format,
    /// Cut short before its size could be read.
    Damaged,
    /// An animated PNG.
    Animated,
    /// A side under 280 pixels, or fewer than 78,400 pixels.
    TooSmall,
    /// A side over 1568 pixels, or more than 2,458,624 pixels.
    TooLarge,
    /// The file is over 16 MiB.
    TooBig,
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ImageError::Format => "an image must be a PNG or a JPEG",
            ImageError::Damaged => "the image is cut short or damaged",
            ImageError::Animated => "an animated image is not accepted",
            ImageError::TooSmall => "the image is below the minimum size",
            ImageError::TooLarge => "the image is above the maximum size",
            ImageError::TooBig => "the image file is over 16 MiB",
        })
    }
}

impl std::error::Error for ImageError {}

/// An image that passed the local checks. Debug shows the format and the size, never the bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct Image {
    format: ImageFormat,
    width: u32,
    height: u32,
    bytes: Vec<u8>,
}

impl fmt::Debug for Image {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Image({} {}x{}, {} bytes)",
            self.format.name(),
            self.width,
            self.height,
            self.bytes.len()
        )
    }
}

impl Image {
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Image, ImageError> {
        if bytes.len() > MAX_FILE_BYTES {
            return Err(ImageError::TooBig);
        }
        let (format, width, height) =
            if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
                let (w, h) = png_size(&bytes)?;
                (ImageFormat::Png, w, h)
            } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
                let (w, h) = jpeg_size(&bytes)?;
                (ImageFormat::Jpeg, w, h)
            } else {
                return Err(ImageError::Format);
            };
        if width < MIN_SIDE
            || height < MIN_SIDE
            || u64::from(width) * u64::from(height) < MIN_PIXELS
        {
            return Err(ImageError::TooSmall);
        }
        if width > MAX_SIDE
            || height > MAX_SIDE
            || u64::from(width) * u64::from(height) > MAX_PIXELS
        {
            return Err(ImageError::TooLarge);
        }
        Ok(Image {
            format,
            width,
            height,
            bytes,
        })
    }

    pub fn format(&self) -> ImageFormat {
        self.format
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// The file size in bytes.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn to_base64(&self) -> String {
        base64_encode(&self.bytes)
    }
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

/// Width and height from the IHDR chunk; walks the chunks up to the first IDAT to refuse an animated PNG (`acTL`).
fn png_size(b: &[u8]) -> Result<(u32, u32), ImageError> {
    let mut at = 8;
    let mut size = None;
    loop {
        let head = b.get(at..at + 8).ok_or(ImageError::Damaged)?;
        let len = be32(head) as usize;
        let kind = &head[4..8];
        let data = at + 8;
        match kind {
            b"IHDR" => {
                if len != 13 || size.is_some() || at != 8 {
                    return Err(ImageError::Damaged);
                }
                let d = b.get(data..data + 8).ok_or(ImageError::Damaged)?;
                size = Some((be32(d), be32(&d[4..])));
            }
            b"acTL" => return Err(ImageError::Animated),
            b"IDAT" => return size.ok_or(ImageError::Damaged),
            _ => {}
        }
        at = data
            .checked_add(len)
            .and_then(|x| x.checked_add(4))
            .ok_or(ImageError::Damaged)?;
    }
}

/// Width and height from the first start-of-frame segment.
fn jpeg_size(b: &[u8]) -> Result<(u32, u32), ImageError> {
    let mut at = 2;
    loop {
        // skip to the next marker (any run of 0xFF is fill)
        while b.get(at).ok_or(ImageError::Damaged)? != &0xFF {
            at += 1;
        }
        while b.get(at).ok_or(ImageError::Damaged)? == &0xFF {
            at += 1;
        }
        let marker = b[at];
        at += 1;
        match marker {
            0x00 | 0x01 | 0xD0..=0xD8 => continue, // stuffed zero, TEM, restart and start markers carry no length
            0xD9 | 0xDA => return Err(ImageError::Damaged), // end or scan before any frame header
            _ => {}
        }
        let len = b.get(at..at + 2).ok_or(ImageError::Damaged)?;
        let len = usize::from(u16::from_be_bytes([len[0], len[1]]));
        if len < 2 {
            return Err(ImageError::Damaged);
        }
        if matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            let d = b.get(at + 3..at + 7).ok_or(ImageError::Damaged)?;
            let h = u32::from(u16::from_be_bytes([d[0], d[1]]));
            let w = u32::from(u16::from_be_bytes([d[2], d[3]]));
            return Ok((w, h));
        }
        at += len;
    }
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with `=` padding and no line breaks.
pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests;
