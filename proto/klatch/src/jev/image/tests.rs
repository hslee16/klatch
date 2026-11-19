use super::*;

/// The leading bytes of a PNG with the given size: signature, IHDR, then an IDAT chunk of `pad` bytes and IEND. Not a
/// decodable picture; the adapter reads the header only (the API is the one that decodes).
fn png(w: u32, h: u32, pad: usize, animated: bool) -> Vec<u8> {
    let mut v = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let chunk = |v: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]| {
        v.extend((data.len() as u32).to_be_bytes());
        v.extend(kind);
        v.extend(data);
        v.extend([0, 0, 0, 0]); // crc, not checked
    };
    let mut ihdr = Vec::new();
    ihdr.extend(w.to_be_bytes());
    ihdr.extend(h.to_be_bytes());
    ihdr.extend([8, 2, 0, 0, 0]);
    chunk(&mut v, b"IHDR", &ihdr);
    if animated {
        chunk(&mut v, b"acTL", &[0, 0, 0, 2, 0, 0, 0, 0]);
    }
    chunk(&mut v, b"IDAT", &vec![0u8; pad]);
    chunk(&mut v, b"IEND", &[]);
    v
}

/// A JPEG that is a start marker, one filler segment and a start-of-frame segment with the given marker byte.
fn jpeg(w: u16, h: u16, sof: u8) -> Vec<u8> {
    let mut v = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00];
    v.extend([0xFF, sof, 0x00, 0x0B, 8]);
    v.extend(h.to_be_bytes());
    v.extend(w.to_be_bytes());
    v.extend([1, 1, 0x11, 0]);
    v.extend([0xFF, 0xD9]);
    v
}

#[test]
fn base64_matches_the_rfc_4648_vectors() {
    for (raw, enc) in [
        ("", ""),
        ("f", "Zg=="),
        ("fo", "Zm8="),
        ("foo", "Zm9v"),
        ("foob", "Zm9vYg=="),
        ("fooba", "Zm9vYmE="),
        ("foobar", "Zm9vYmFy"),
    ] {
        assert_eq!(base64_encode(raw.as_bytes()), enc);
    }
    // the standard alphabet, padding, no line breaks
    let all: Vec<u8> = (0..=255).collect();
    let e = base64_encode(&all);
    assert!(e.contains('+') && e.contains('/') && !e.contains('\n') && e.len().is_multiple_of(4));
}

#[test]
fn a_png_and_a_jpeg_in_range_are_accepted_and_measured() {
    let p = Image::from_bytes(png(640, 480, 10, false)).unwrap();
    assert_eq!(
        (p.format(), p.width(), p.height()),
        (ImageFormat::Png, 640, 480)
    );
    assert_eq!(p.format().name(), "png");
    for sof in [0xC0, 0xC1, 0xC2] {
        let j = Image::from_bytes(jpeg(1568, 1176, sof)).unwrap();
        assert_eq!(
            (j.format(), j.width(), j.height()),
            (ImageFormat::Jpeg, 1568, 1176)
        );
        assert_eq!(j.format().name(), "jpg");
    }
}

#[test]
fn the_limits_include_both_ends_of_the_range() {
    assert!(Image::from_bytes(png(280, 280, 0, false)).is_ok());
    assert!(Image::from_bytes(png(1568, 1568, 0, false)).is_ok());
    assert_eq!(
        Image::from_bytes(png(279, 400, 0, false)).unwrap_err(),
        ImageError::TooSmall
    );
    assert_eq!(
        Image::from_bytes(png(400, 279, 0, false)).unwrap_err(),
        ImageError::TooSmall
    );
    assert_eq!(
        Image::from_bytes(png(1569, 400, 0, false)).unwrap_err(),
        ImageError::TooLarge
    );
    assert_eq!(
        Image::from_bytes(jpeg(400, 1569, 0xC0)).unwrap_err(),
        ImageError::TooLarge
    );
}

#[test]
fn the_file_is_at_most_16_mib() {
    let at = png(400, 400, MAX_FILE_BYTES - 57, false);
    assert_eq!(at.len(), MAX_FILE_BYTES);
    assert!(Image::from_bytes(at).is_ok());
    let over = png(400, 400, MAX_FILE_BYTES - 56, false);
    assert_eq!(Image::from_bytes(over).unwrap_err(), ImageError::TooBig);
}

#[test]
fn other_formats_animation_and_damage_are_refused() {
    let gif = b"GIF89a\x10\x00\x10\x00".to_vec();
    assert_eq!(Image::from_bytes(gif).unwrap_err(), ImageError::Format);
    assert_eq!(Image::from_bytes(vec![]).unwrap_err(), ImageError::Format);
    assert_eq!(
        Image::from_bytes(b"RIFF....WEBPVP8 ".to_vec()).unwrap_err(),
        ImageError::Format
    );
    assert_eq!(
        Image::from_bytes(png(400, 400, 4, true)).unwrap_err(),
        ImageError::Animated
    );
    // truncated before the size is known
    assert_eq!(
        Image::from_bytes(png(400, 400, 0, false)[..20].to_vec()).unwrap_err(),
        ImageError::Damaged
    );
    assert_eq!(
        Image::from_bytes(vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00]).unwrap_err(),
        ImageError::Damaged
    );
    // a jpeg with no start-of-frame segment before the end
    assert_eq!(
        Image::from_bytes(vec![0xFF, 0xD8, 0xFF, 0xD9]).unwrap_err(),
        ImageError::Damaged
    );
}

#[test]
fn the_base64_of_an_image_is_its_bytes_encoded() {
    let bytes = png(300, 300, 3, false);
    let img = Image::from_bytes(bytes.clone()).unwrap();
    assert_eq!(img.to_base64(), base64_encode(&bytes));
    assert_eq!(img.len(), bytes.len());
}
