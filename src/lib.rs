use std::{fs::File, io::Read, path::Path};

use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use zip::ZipArchive;

const THUMBNAIL_PATH: &str = "thumb.jpg";
const MAX_THUMBNAIL_BYTES: u64 = 64 * 1024 * 1024;
const TREE_PATH: &str = "tree.pack";
const METADATA_PATH: &str = "metadata.json";
const POINT_RECORD_SIZE: usize = 16;
const SVG_Y_FLIP: f32 = 768.0;
const SVG_MILLIMETERS_PER_UNIT: f32 = 0.192_424;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversionKind {
    Vector,
    ThumbnailFallback,
}

impl std::fmt::Display for ConversionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Vector => f.write_str("vector"),
            Self::ThumbnailFallback => f.write_str("thumbnail fallback"),
        }
    }
}

#[derive(Debug)]
struct Point {
    x: f32,
    y: f32,
}

#[derive(Debug)]
struct Stroke {
    id: String,
    color: String,
    width: f32,
    opacity: f32,
    points: Vec<Point>,
}

#[derive(Debug)]
struct Layer {
    name: String,
    visible: bool,
    opacity: f32,
    strokes: Vec<Stroke>,
}

#[derive(Debug)]
struct Drawing {
    background: String,
    layers: Vec<Layer>,
}

/// Converts a Concepts archive to SVG, falling back to its embedded thumbnail
/// if the vector records contain unsupported content.
pub fn convert_file(input: &Path, output: &Path) -> Result<ConversionKind> {
    let file = File::open(input).with_context(|| format!("opening {}", input.display()))?;
    let mut archive = ZipArchive::new(file).context("reading .concepts ZIP archive")?;
    let jpeg = read_thumbnail(&mut archive)?;
    let (width, height) = jpeg_dimensions(&jpeg).context("reading thumbnail dimensions")?;

    let (svg, kind) = match parse_vector_archive(&mut archive) {
        Ok(drawing) => match vector_svg(&drawing) {
            Ok(svg) => (svg, ConversionKind::Vector),
            Err(error) => {
                eprintln!("warning: {error:#}; using embedded thumbnail instead");
                (
                    thumbnail_svg(&jpeg, width, height),
                    ConversionKind::ThumbnailFallback,
                )
            }
        },
        Err(error) => {
            eprintln!("warning: {error:#}; using embedded thumbnail instead");
            (
                thumbnail_svg(&jpeg, width, height),
                ConversionKind::ThumbnailFallback,
            )
        }
    };

    std::fs::write(output, svg).with_context(|| format!("writing {}", output.display()))?;
    Ok(kind)
}

fn read_thumbnail(archive: &mut ZipArchive<File>) -> Result<Vec<u8>> {
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
    Ok(jpeg)
}

fn parse_vector_archive(archive: &mut ZipArchive<File>) -> Result<Drawing> {
    let metadata: serde_json::Value = {
        let mut file = archive
            .by_name(METADATA_PATH)
            .context("archive does not contain metadata.json")?;
        let mut json = String::new();
        file.read_to_string(&mut json)
            .context("reading metadata.json")?;
        serde_json::from_str(&json).context("parsing metadata.json")?
    };
    let background = metadata
        .get("backgroundSolidColor")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("#ffffff")
        .to_owned();

    let tree = {
        let mut file = archive
            .by_name(TREE_PATH)
            .context("archive does not contain tree.pack")?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).context("reading tree.pack")?;
        rmpv::decode::read_value(&mut std::io::Cursor::new(bytes))
            .context("decoding tree.pack MessagePack")?
    };
    parse_tree(&tree, background)
}

fn parse_tree(tree: &rmpv::Value, background: String) -> Result<Drawing> {
    let root = array(tree, "tree root")?;
    let document = array(required(root, 1, "tree document")?, "tree document")?;
    let layer_values = array(required(document, 4, "document layers")?, "document layers")?;
    let mut layers = Vec::with_capacity(layer_values.len());

    for (index, value) in layer_values.iter().enumerate() {
        let fields = array(value, "layer")?;
        let layer_metadata = array(required(fields, 1, "layer metadata")?, "layer metadata")?;
        let name = fields
            .get(3)
            .and_then(rmpv::Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| {
                if index == 0 {
                    "Pen".to_owned()
                } else {
                    format!("Layer {}", index + 1)
                }
            });
        let visible = layer_metadata
            .get(6)
            .and_then(rmpv::Value::as_bool)
            .unwrap_or(true);
        let opacity = layer_metadata
            .get(7)
            .map(number)
            .transpose()?
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);
        let strokes = if visible && opacity > 0.0 {
            let object_values = array(required(fields, 2, "layer objects")?, "layer objects")?;
            object_values
                .iter()
                .map(parse_stroke)
                .collect::<Result<Vec<_>>>()?
        } else {
            Vec::new()
        };
        layers.push(Layer {
            name,
            visible,
            opacity,
            strokes,
        });
    }

    if layers.iter().all(|layer| layer.strokes.is_empty()) {
        bail!("tree.pack contains no supported strokes");
    }
    Ok(Drawing { background, layers })
}

fn parse_stroke(object: &rmpv::Value) -> Result<Stroke> {
    let object_fields = array(object, "drawing object")?;
    let object_type = integer(required(object_fields, 0, "drawing object type")?)?;
    if !matches!(object_type, 10 | 11) {
        bail!("unsupported drawing object type");
    }
    let object_body = array(
        required(object_fields, 1, "drawing object body")?,
        "drawing object body",
    )?;
    let stroke_node = array(required(object_body, 1, "stroke node")?, "stroke node")?;
    if integer(required(stroke_node, 0, "stroke node type")?)? != 6 {
        bail!("unsupported stroke node type");
    }
    let fields = stroke_node;
    let style = array(required(fields, 1, "stroke style")?, "stroke style")?;
    let style_group = array(
        required(style, 1, "stroke style group")?,
        "stroke style group",
    )?;
    let parameters = array(
        required(style_group, 1, "stroke parameters")?,
        "stroke parameters",
    )?;
    let color_fields = array(required(parameters, 1, "stroke color")?, "stroke color")?;

    let uuid = extension(required(style, 2, "stroke UUID")?, 5, "stroke UUID")?;
    if uuid.len() != 16 {
        bail!("stroke UUID has invalid length {}", uuid.len());
    }
    let id = format_uuid(uuid);
    let rgba = extension(required(color_fields, 2, "stroke RGBA")?, 4, "stroke RGBA")?;
    if rgba.len() != 16 {
        bail!("stroke RGBA has invalid length {}", rgba.len());
    }
    let channels = rgba
        .chunks_exact(4)
        .map(|bytes| f32::from_le_bytes(bytes.try_into().expect("four-byte chunk")))
        .collect::<Vec<_>>();
    if channels.iter().any(|channel| !channel.is_finite()) {
        bail!("stroke color contains a non-finite channel");
    }
    let color = format!(
        "#{:02x}{:02x}{:02x}",
        color_channel(channels[0]),
        color_channel(channels[1]),
        color_channel(channels[2])
    );
    let opacity = number(required(color_fields, 3, "stroke opacity")?)?.clamp(0.0, 1.0)
        * channels[3].clamp(0.0, 1.0);
    let brush_variant = array(required(color_fields, 1, "brush variant")?, "brush variant")?;
    let brush_variant = integer(required(brush_variant, 1, "brush variant code")?)?;
    let width = number(required(parameters, 3, "stroke width")?)?
        / if brush_variant == 6 { 2.0 } else { 1.0 };
    if !width.is_finite() || width <= 0.0 {
        bail!("stroke has invalid width {width}");
    }

    let point_bytes = required(fields, 17, "stroke point data")?
        .as_slice()
        .context("stroke point data is not a MessagePack binary value")?;
    if point_bytes.is_empty() || point_bytes.len() % POINT_RECORD_SIZE != 0 {
        bail!(
            "stroke point data has unsupported length {}",
            point_bytes.len()
        );
    }
    let mut points = Vec::with_capacity(point_bytes.len() / POINT_RECORD_SIZE);
    for record in point_bytes.chunks_exact(POINT_RECORD_SIZE) {
        let x = f32::from_le_bytes(record[0..4].try_into().expect("four-byte x"));
        let source_y = f32::from_le_bytes(record[4..8].try_into().expect("four-byte y"));
        let y = SVG_Y_FLIP - source_y;
        if !x.is_finite() || !y.is_finite() {
            bail!("stroke point contains a non-finite coordinate");
        }
        points.push(Point { x, y });
    }

    Ok(Stroke {
        id,
        color,
        width,
        opacity,
        points,
    })
}

fn vector_svg(drawing: &Drawing) -> Result<String> {
    let mut bounds: Option<(f32, f32, f32, f32)> = None;
    let mut max_width: f32 = 0.0;
    for layer in &drawing.layers {
        if !layer.visible || layer.opacity <= 0.0 {
            continue;
        }
        for stroke in &layer.strokes {
            max_width = max_width.max(stroke.width);
            for point in &stroke.points {
                bounds = Some(match bounds {
                    None => (point.x, point.y, point.x, point.y),
                    Some((min_x, min_y, max_x, max_y)) => (
                        min_x.min(point.x),
                        min_y.min(point.y),
                        max_x.max(point.x),
                        max_y.max(point.y),
                    ),
                });
            }
        }
    }
    let (min_x, min_y, max_x, max_y) = bounds.context("drawing has no visible points")?;
    let padding = max_width / 2.0 + 4.0;
    let x = min_x - padding;
    let y = min_y - padding;
    let width = (max_x - min_x + padding * 2.0).max(1.0);
    let height = (max_y - min_y + padding * 2.0).max(1.0);

    let mut svg = String::new();
    use std::fmt::Write as _;
    writeln!(
        svg,
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}mm\" height=\"{}mm\" viewBox=\"{} {} {} {}\">",
        format_num(width * SVG_MILLIMETERS_PER_UNIT),
        format_num(height * SVG_MILLIMETERS_PER_UNIT),
        format_num(x),
        format_num(y),
        format_num(width),
        format_num(height)
    )?;
    writeln!(
        svg,
        "  <rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" stroke=\"none\" fill=\"{}\" />",
        format_num(x),
        format_num(y),
        format_num(width),
        format_num(height),
        xml_escape(&drawing.background)
    )?;

    for (layer_index, layer) in drawing.layers.iter().enumerate() {
        if !layer.visible || layer.opacity <= 0.0 {
            continue;
        }
        let id = xml_escape(&layer_id(&layer.name, layer_index));
        writeln!(
            svg,
            "  <g id=\"{id}\" opacity=\"{}\">",
            format_num(layer.opacity)
        )?;
        for stroke in &layer.strokes {
            write_stroke(&mut svg, stroke)?;
        }
        writeln!(svg, "  </g>")?;
    }
    writeln!(svg, "</svg>")?;
    Ok(svg)
}

fn write_stroke(svg: &mut String, stroke: &Stroke) -> Result<()> {
    use std::fmt::Write as _;
    let opacity = format_num(stroke.opacity);
    let id = format!("STROKE_{}", stroke.id);
    if stroke.points.len() == 1 {
        let point = &stroke.points[0];
        writeln!(
            svg,
            "    <circle id=\"{id}\" opacity=\"1.000\" fill=\"{}\" stroke=\"{}\" stroke-width=\"0.1\" stroke-opacity=\"{opacity}\" fill-opacity=\"{opacity}\" cx=\"{}\" cy=\"{}\" r=\"{}\" />",
            stroke.color,
            stroke.color,
            format_num(point.x),
            format_num(point.y),
            format_num(stroke.width / 2.0)
        )?;
        return Ok(());
    }
    if stroke.points.len() < 2 {
        bail!("stroke {} has no points", stroke.id);
    }

    write!(
        svg,
        "    <path id=\"{id}\" opacity=\"1.000\" fill=\"none\" stroke=\"{}\" stroke-width=\"{}\" stroke-opacity=\"{opacity}\" stroke-linecap=\"round\" stroke-linejoin=\"round\" d=\"M {} {}",
        stroke.color,
        format_num(stroke.width),
        format_num(stroke.points[0].x),
        format_num(stroke.points[0].y)
    )?;
    if stroke.points.len() == 2 {
        write!(
            svg,
            " L {} {}",
            format_num(stroke.points[1].x),
            format_num(stroke.points[1].y)
        )?;
    } else {
        for index in 1..stroke.points.len() - 2 {
            let control = &stroke.points[index];
            let next = &stroke.points[index + 1];
            let end_x = (control.x + next.x) / 2.0;
            let end_y = (control.y + next.y) / 2.0;
            write!(
                svg,
                " Q {} {} {} {}",
                format_num(control.x),
                format_num(control.y),
                format_num(end_x),
                format_num(end_y)
            )?;
        }
        let last_control = &stroke.points[stroke.points.len() - 2];
        let last = &stroke.points[stroke.points.len() - 1];
        write!(
            svg,
            " Q {} {} {} {}",
            format_num(last_control.x),
            format_num(last_control.y),
            format_num(last.x),
            format_num(last.y)
        )?;
    }
    writeln!(svg, "\" />")?;
    Ok(())
}

fn array<'a>(value: &'a rmpv::Value, what: &str) -> Result<&'a [rmpv::Value]> {
    value
        .as_array()
        .map(Vec::as_slice)
        .with_context(|| format!("{what} is not an array"))
}

fn required<'a>(values: &'a [rmpv::Value], index: usize, what: &str) -> Result<&'a rmpv::Value> {
    values
        .get(index)
        .with_context(|| format!("{what} is missing at array index {index}"))
}

fn integer(value: &rmpv::Value) -> Result<i64> {
    value.as_i64().context("expected integer")
}

fn number(value: &rmpv::Value) -> Result<f32> {
    match value {
        rmpv::Value::F32(value) => Ok(*value),
        rmpv::Value::F64(value) => Ok(*value as f32),
        rmpv::Value::Integer(value) => value
            .as_i64()
            .map(|value| value as f32)
            .context("integer does not fit a signed 64-bit value"),
        _ => bail!("expected numeric value"),
    }
}

fn extension<'a>(value: &'a rmpv::Value, expected_kind: i8, what: &str) -> Result<&'a [u8]> {
    let (kind, data) = value
        .as_ext()
        .with_context(|| format!("{what} is not an extension"))?;
    if kind != expected_kind {
        bail!("{what} has extension type {kind}, expected {expected_kind}");
    }
    Ok(data)
}

fn format_uuid(bytes: &[u8]) -> String {
    let hex = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

fn color_channel(channel: f32) -> u8 {
    (channel.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn format_num(value: f32) -> String {
    let rounded = (value * 1000.0).round() / 1000.0;
    if rounded == 0.0 {
        "0.000".to_owned()
    } else {
        format!("{rounded:.3}")
    }
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn layer_id(name: &str, index: usize) -> String {
    let mut id = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    if id.is_empty() || id.as_bytes()[0].is_ascii_digit() {
        id.insert_str(0, "layer_");
    }
    if index > 0 && id == "Layer" {
        id.push_str(&format!("_{index}"));
    }
    id
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

    use super::{
        ConversionKind, convert_file, jpeg_dimensions, parse_tree, thumbnail_svg, vector_svg,
    };
    use rmpv::Value;

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

    fn archive_with_vector_tree(jpeg: &[u8]) -> Vec<u8> {
        let mut output = Cursor::new(Vec::new());
        {
            let mut writer = ZipWriter::new(&mut output);
            writer
                .start_file("thumb.jpg", SimpleFileOptions::default())
                .unwrap();
            std::io::Write::write_all(&mut writer, jpeg).unwrap();
            writer
                .start_file("metadata.json", SimpleFileOptions::default())
                .unwrap();
            std::io::Write::write_all(&mut writer, br##"{"backgroundSolidColor":"#000000"}"##)
                .unwrap();
            writer
                .start_file("tree.pack", SimpleFileOptions::default())
                .unwrap();
            rmpv::encode::write_value(&mut writer, &synthetic_tree()).unwrap();
            writer.finish().unwrap();
        }
        output.into_inner()
    }

    fn msg_int(value: i64) -> Value {
        Value::Integer(value.into())
    }

    fn msg_array(values: impl IntoIterator<Item = Value>) -> Value {
        Value::Array(values.into_iter().collect())
    }

    fn msg_ext(kind: i8, bytes: &[u8]) -> Value {
        Value::Ext(kind, bytes.to_vec())
    }

    fn synthetic_tree() -> Value {
        synthetic_tree_with_brush(2, 3.0, 1.0)
    }

    fn synthetic_tree_with_brush(brush_variant: i64, width: f32, opacity: f32) -> Value {
        let uuid = [
            0x86, 0xb1, 0xb5, 0xe3, 0x23, 0x51, 0x45, 0x5f, 0x8f, 0x62, 0x89, 0xcb, 0x95, 0x1a,
            0x34, 0xa9,
        ];
        let mut rgba = Vec::new();
        for channel in [1.0_f32, 203.0 / 255.0, 18.0 / 255.0, 1.0] {
            rgba.extend_from_slice(&channel.to_le_bytes());
        }
        let mut points = Vec::new();
        for (x, y) in [(1.0_f32, 2.0_f32), (3.0, 4.0), (5.0, 6.0)] {
            points.extend_from_slice(&x.to_le_bytes());
            points.extend_from_slice(&y.to_le_bytes());
            points.extend_from_slice(&[0; 8]);
        }

        let color = msg_array([
            msg_int(0),
            msg_array([msg_int(0), msg_int(brush_variant)]),
            msg_ext(4, &rgba),
            Value::F32(opacity),
        ]);
        let parameters = msg_array([
            msg_int(1),
            color,
            Value::Nil,
            Value::F32(width),
            Value::F32(0.0),
        ]);
        let style_group = msg_array([msg_int(1), parameters]);
        let style = msg_array([
            msg_int(3),
            style_group,
            msg_ext(5, &uuid),
            Value::Nil,
            Value::Nil,
            msg_int(0),
            Value::F64(0.0),
            msg_ext(7, &[0; 64]),
            Value::Boolean(false),
            msg_int(17_038_118),
        ]);
        let stroke_fields = msg_array([
            msg_int(6),
            style,
            msg_int(2),
            Value::F32(1.0),
            Value::F32(0.1),
            Value::Boolean(false),
            Value::Boolean(true),
            Value::Boolean(true),
            Value::Boolean(true),
            Value::Boolean(true),
            Value::Boolean(true),
            Value::Boolean(true),
            Value::Boolean(false),
            msg_array([Value::F32(0.0), Value::F32(0.0)]),
            Value::Nil,
            Value::F32(0.0),
            msg_array([]),
            Value::Binary(points),
            Value::Binary(vec![0; 2]),
        ]);
        let object = msg_array([msg_int(11), msg_array([msg_int(0), stroke_fields])]);
        let layer_metadata = msg_array([
            msg_int(4),
            msg_ext(5, &[0; 16]),
            Value::Nil,
            Value::F64(0.0),
            msg_ext(7, &[0; 64]),
            Value::Boolean(false),
            Value::Boolean(true),
            Value::F32(1.0),
        ]);
        let layer = msg_array([
            msg_int(1),
            layer_metadata,
            msg_array([object]),
            Value::String("Pen".into()),
        ]);
        let document = msg_array([
            msg_int(0),
            msg_ext(5, &[0; 16]),
            msg_int(1),
            msg_array([
                msg_int(3),
                msg_int(0),
                msg_int(0),
                msg_int(2),
                Value::Boolean(false),
                Value::Boolean(false),
            ]),
            msg_array([layer]),
            msg_array([]),
            msg_array([]),
        ]);
        msg_array([msg_int(1027), document])
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
    fn decodes_stroke_points_color_and_canvas_y_axis() {
        let drawing = parse_tree(&synthetic_tree(), "#000000".to_owned()).unwrap();
        let svg = vector_svg(&drawing).unwrap();
        assert!(svg.contains("stroke=\"#ffcb12\""));
        assert!(svg.contains("stroke-width=\"3.000\""));
        assert!(svg.contains("M 1.000 766.000 Q 3.000 764.000 5.000 762.000"));
        assert!(svg.contains("id=\"STROKE_86b1b5e3-2351-455f-8f62-89cb951a34a9\""));
    }

    #[test]
    fn applies_observed_brush_width_scaling_and_opacity() {
        let drawing = parse_tree(
            &synthetic_tree_with_brush(6, 5.0, 0.694),
            "#000000".to_owned(),
        )
        .unwrap();
        let svg = vector_svg(&drawing).unwrap();
        assert!(svg.contains("stroke-width=\"2.500\""));
        assert!(svg.contains("stroke-opacity=\"0.694\""));
    }

    #[test]
    fn falls_back_to_thumbnail_when_tree_pack_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("drawing.concepts");
        let output = dir.path().join("drawing.svg");
        std::fs::write(&input, archive_with_thumbnail(&minimal_jpeg(1024, 593))).unwrap();

        assert_eq!(
            convert_file(&input, &output).unwrap(),
            ConversionKind::ThumbnailFallback
        );
        let svg = std::fs::read_to_string(output).unwrap();
        assert!(svg.contains("viewBox=\"0 0 1024 593\""));
        assert!(svg.contains("data:image/jpeg;base64,"));
    }

    #[test]
    fn converts_supported_tree_records_to_vector_svg() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("drawing.concepts");
        let output = dir.path().join("drawing.svg");
        std::fs::write(&input, archive_with_vector_tree(&minimal_jpeg(1024, 593))).unwrap();

        assert_eq!(
            convert_file(&input, &output).unwrap(),
            ConversionKind::Vector
        );
        let svg = std::fs::read_to_string(output).unwrap();
        assert!(svg.contains("stroke=\"#ffcb12\""));
        assert!(!svg.contains("data:image/jpeg;base64,"));
    }
}
