use std::{fs::File, io::Read, path::Path};

use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use zip::ZipArchive;

const THUMBNAIL_PATH: &str = "thumb.jpg";
const MAX_THUMBNAIL_BYTES: u64 = 64 * 1024 * 1024;

/// Converts a Concepts archive to an SVG containing its embedded thumbnail.
///
/// Concepts' current `.concepts` stroke geometry uses a packed binary encoding
/// that is not yet decoded here. The archive thumbnail is embedded as a JPEG,
/// which retains the drawing's appearance but is not an editable/scalable
/// vector representation.
pub fn convert_file(input: &Path, output: &Path) -> Result<()> {
    let file = File::open(input).with_context(|| format!("opening {}", input.display()))?;
    let mut archive = ZipArchive::new(file).context("reading .concepts ZIP archive")?;
    let mut thumbnail = archive
        .by_name(THUMBNAIL_PATH)
        .context("archive does not contain thumb.jpg")?;

    if thumbnail.size() > MAX_THUMBNAIL_BYTES {
        bail!("thumbnail is larger than the 64 MiB safety limit");
    }

    let mut jpeg = Vec::with_capacity(thumbnail.size() as usize);
    thumbnail
        .by_ref()
        .take(MAX_THUMBNAIL_BYTES + 1)
        .read_to_end(&mut jpeg)
        .context("reading thumb.jpg")?;
    if jpeg.len() as u64 > MAX_THUMBNAIL_BYTES {
        bail!("thumbnail is larger than the 64 MiB safety limit");
    }
    let (width, height) = jpeg_dimensions(&jpeg).context("reading thumbnail dimensions")?;
    let svg = thumbnail_svg(&jpeg, width, height);
    std::fs::write(output, svg).with_context(|| format!("writing {}", output.display()))
}

/// Returns the width and height of a JPEG by scanning its marker segments.
fn jpeg_dimensions(jpeg: &[u8]) -> Result<(u32, u32)> {
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

fn thumbnail_svg(jpeg: &[u8], width: u32, height: u32) -> String {
    let encoded = STANDARD.encode(jpeg);
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}px\" height=\"{height}px\" viewBox=\"0 0 {width} {height}\">\n\
         <image width=\"{width}\" height=\"{height}\" href=\"data:image/jpeg;base64,{encoded}\"/>\n\
         </svg>\n"
    )
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use zip::{ZipWriter, write::SimpleFileOptions};

    use super::{convert_file, jpeg_dimensions, thumbnail_svg};

    fn minimal_jpeg(width: u16, height: u16) -> Vec<u8> {
        // SOI, a baseline SOF segment with three components, then EOI.
        let mut jpeg = vec![
            0xff,
            0xd8,
            0xff,
            0xc0,
            0x00,
            0x11,
            0x08,
            (height >> 8) as u8,
            height as u8,
            (width >> 8) as u8,
            width as u8,
            0x03,
            0x01,
            0x11,
            0x00,
            0x02,
            0x11,
            0x00,
            0x03,
            0x11,
            0x00,
            0xff,
            0xd9,
        ];
        jpeg.shrink_to_fit();
        jpeg
    }

    fn archive_with_thumbnail(jpeg: &[u8]) -> Vec<u8> {
        let mut output = Cursor::new(Vec::new());
        {
            let mut writer = ZipWriter::new(&mut output);
            writer
                .start_file("thumb.jpg", SimpleFileOptions::default())
                .unwrap();
            std::io::Write::write_all(&mut writer, jpeg).unwrap();
            writer.finish().unwrap();
        }
        output.into_inner()
    }

    #[test]
    fn reads_dimensions_from_jpeg_frame() {
        assert_eq!(
            jpeg_dimensions(&minimal_jpeg(1024, 593)).unwrap(),
            (1024, 593)
        );
    }

    #[test]
    fn rejects_non_jpeg_thumbnail() {
        assert!(jpeg_dimensions(b"not a jpeg").is_err());
    }

    #[test]
    fn svg_embeds_thumbnail_without_reencoding_it() {
        let jpeg = minimal_jpeg(320, 200);
        let svg = thumbnail_svg(&jpeg, 320, 200);
        assert!(svg.contains("width=\"320px\" height=\"200px\""));
        assert!(svg.contains("viewBox=\"0 0 320 200\""));
        let data = svg
            .split("data:image/jpeg;base64,")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        assert_eq!(STANDARD.decode(data).unwrap(), jpeg);
    }

    #[test]
    fn converts_a_concepts_zip_archive() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("drawing.concepts");
        let output = dir.path().join("drawing.svg");
        std::fs::write(&input, archive_with_thumbnail(&minimal_jpeg(1024, 593))).unwrap();

        convert_file(&input, &output).unwrap();
        let svg = std::fs::read_to_string(output).unwrap();
        assert!(svg.contains("viewBox=\"0 0 1024 593\""));
        assert!(svg.contains("data:image/jpeg;base64,"));
    }
}
