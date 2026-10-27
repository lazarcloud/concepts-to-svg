use anyhow::{Context, Result, bail};

fn png_dimensions(png: &[u8]) -> Result<(u32, u32)> {
    if !png.starts_with(b"\x89PNG\r\n\x1a\n") || png.len() < 24 || &png[12..16] != b"IHDR" {
        bail!("embedded image is not a supported PNG");
    }
    let width = u32::from_be_bytes(png[16..20].try_into().expect("four-byte PNG width"));
    let height = u32::from_be_bytes(png[20..24].try_into().expect("four-byte PNG height"));
    if width == 0 || height == 0 {
        bail!("PNG image has invalid dimensions {width}x{height}");
    }
    Ok((width, height))
}

pub(crate) fn raster_image_info(bytes: &[u8]) -> Result<(u32, u32, &'static str)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        let (width, height) = png_dimensions(bytes)?;
        Ok((width, height, "image/png"))
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        let (width, height) = jpeg_dimensions(bytes)?;
        Ok((width, height, "image/jpeg"))
    } else {
        bail!("embedded image is neither PNG nor JPEG")
    }
}

/// Returns the width and height of a JPEG by scanning its marker segments.
pub(crate) fn jpeg_dimensions(jpeg: &[u8]) -> Result<(u32, u32)> {
    if !jpeg.starts_with(&[0xff, 0xd8]) {
        bail!("thumbnail is not a JPEG file");
    }

    let mut offset = 2;
    while offset < jpeg.len() {
        if jpeg[offset] != 0xff {
            offset += 1;
            continue;
        }
        while offset < jpeg.len() && jpeg[offset] == 0xff {
            offset += 1;
        }
        let Some(&marker) = jpeg.get(offset) else {
            break;
        };
        offset += 1;

        // Standalone markers have no length field.
        if marker == 0xd8 || marker == 0xd9 || marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }

        let length_bytes = jpeg
            .get(offset..offset + 2)
            .context("truncated JPEG marker length")?;
        let segment_length = u16::from_be_bytes([length_bytes[0], length_bytes[1]]) as usize;
        if segment_length < 2 {
            bail!("invalid JPEG marker length");
        }

        if is_start_of_frame(marker) {
            if segment_length < 7 {
                bail!("JPEG frame header is too short");
            }
            let dimensions = jpeg
                .get(offset + 3..offset + 7)
                .context("truncated JPEG frame header")?;
            let height = u16::from_be_bytes([dimensions[0], dimensions[1]]) as u32;
            let width = u16::from_be_bytes([dimensions[2], dimensions[3]]) as u32;
            if width == 0 || height == 0 {
                bail!("JPEG has invalid dimensions {width}x{height}");
            }
            return Ok((width, height));
        }

        offset = offset
            .checked_add(segment_length)
            .context("JPEG marker offset overflow")?;
        if offset > jpeg.len() {
            bail!("truncated JPEG marker segment");
        }
    }

    bail!("JPEG does not contain a supported frame header")
}

fn is_start_of_frame(marker: u8) -> bool {
    matches!(
        marker,
        0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf
    )
}
