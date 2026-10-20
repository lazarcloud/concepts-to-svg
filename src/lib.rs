use std::{collections::HashSet, fs::File, io::Read, path::Path};

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
// The observed Concepts text record does not carry explicit font metrics.
const TEXT_FONT_SIZE: f32 = 24.0;
const TEXT_ORIGIN_Y_OFFSET: f32 = -9.0;

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
    closed: bool,
    filled: bool,
    line_cap: &'static str,
    line_join: &'static str,
    dasharray: Option<(f32, f32)>,
    filter: Option<&'static str>,
    points: Vec<Point>,
}

#[derive(Debug)]
struct EmbeddedImage {
    id: String,
    resource_id: String,
    bytes: Vec<u8>,
    media_type: &'static str,
    pixel_width: u32,
    pixel_height: u32,
    display_width: f32,
    display_height: f32,
    transform: PlanarTransform,
}

#[derive(Debug)]
struct Text {
    id: String,
    content: String,
    color: String,
    opacity: f32,
    transform: PlanarTransform,
}

#[derive(Debug)]
enum DrawingObject {
    Stroke(Stroke),
    Image(EmbeddedImage),
    Text(Text),
}

#[derive(Debug, Clone, Copy)]
struct PlanarTransform {
    m11: f32,
    m12: f32,
    m21: f32,
    m22: f32,
    m41: f32,
    m42: f32,
    m14: f32,
    m24: f32,
    m44: f32,
}

impl PlanarTransform {
    fn apply(self, x: f32, y: f32) -> Result<(f32, f32)> {
        let denominator = x * self.m14 + y * self.m24 + self.m44;
        if !denominator.is_finite() || denominator.abs() <= f32::EPSILON {
            bail!("transform maps a point to infinity");
        }
        let transformed = (
            (x * self.m11 + y * self.m21 + self.m41) / denominator,
            (x * self.m12 + y * self.m22 + self.m42) / denominator,
        );
        if !transformed.0.is_finite() || !transformed.1.is_finite() {
            bail!("transform produces a non-finite point");
        }
        Ok(transformed)
    }

    fn is_affine(self) -> bool {
        self.m14.abs() <= 1e-6 && self.m24.abs() <= 1e-6 && self.m44.abs() > f32::EPSILON
    }

    fn apply_svg(self, x: f32, y: f32) -> Result<(f32, f32)> {
        if !self.is_affine() {
            bail!("text has a projective transform that SVG text cannot represent");
        }
        let denominator = self.m44;
        let transformed = (
            (x * self.m11 - y * self.m21 + self.m41) / denominator,
            (-x * self.m12 + y * self.m22) / denominator + SVG_Y_FLIP - self.m42 / denominator,
        );
        if !transformed.0.is_finite() || !transformed.1.is_finite() {
            bail!("text transform produces a non-finite point");
        }
        Ok(transformed)
    }
}

#[derive(Debug)]
struct Layer {
    name: String,
    visible: bool,
    opacity: f32,
    objects: Vec<DrawingObject>,
}

#[derive(Debug)]
struct Drawing {
    background: String,
    layers: Vec<Layer>,
}

/// Converts a Concepts archive to SVG, falling back to its embedded thumbnail
/// if the vector records contain unsupported content.
pub fn convert_file(input: &Path, output: &Path) -> Result<ConversionKind> {
    convert_file_with_policy(input, output, false)
}

/// Converts an archive and optionally rejects thumbnail fallback output.
pub fn convert_file_with_policy(
    input: &Path,
    output: &Path,
    require_vector: bool,
) -> Result<ConversionKind> {
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

    if require_vector && kind != ConversionKind::Vector {
        bail!(
            "{} requires vector conversion, but only thumbnail fallback was available",
            input.display()
        );
    }

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
    parse_tree(&tree, background, |path| read_resource(archive, path))
}

fn read_resource(archive: &mut ZipArchive<File>, path: &str) -> Result<Vec<u8>> {
    const MAX_IMAGE_BYTES: u64 = 64 * 1024 * 1024;
    let mut resource = archive
        .by_name(path)
        .with_context(|| format!("reading embedded image resource {path}"))?;
    if resource.size() > MAX_IMAGE_BYTES {
        bail!("embedded image {path} is larger than the 64 MiB safety limit");
    }
    let mut bytes = Vec::with_capacity(resource.size() as usize);
    resource
        .by_ref()
        .take(MAX_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("reading embedded image resource {path}"))?;
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        bail!("embedded image {path} is larger than the 64 MiB safety limit");
    }
    Ok(bytes)
}

fn parse_tree<F>(tree: &rmpv::Value, background: String, mut read_resource: F) -> Result<Drawing>
where
    F: FnMut(&str) -> Result<Vec<u8>>,
{
    let root = array(tree, "tree root")?;
    let document = array(required(root, 1, "tree document")?, "tree document")?;
    let layer_values = array(required(document, 4, "document layers")?, "document layers")?;
    let mut layers = Vec::with_capacity(layer_values.len());

    for (index, value) in layer_values.iter().enumerate() {
        let fields = array(value, "layer")?;
        let layer_metadata = array(required(fields, 1, "layer metadata")?, "layer metadata")?;
        let name = layer_name(fields.get(3), index);
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
        let objects = if visible && opacity > 0.0 {
            let object_values = array(required(fields, 2, "layer objects")?, "layer objects")?;
            object_values
                .iter()
                .map(|object| parse_object(object, &mut read_resource))
                .collect::<Result<Vec<_>>>()?
        } else {
            Vec::new()
        };
        layers.push(Layer {
            name,
            visible,
            opacity,
            objects,
        });
    }

    Ok(Drawing { background, layers })
}

fn layer_name(value: Option<&rmpv::Value>, index: usize) -> String {
    if let Some(name) = value.and_then(rmpv::Value::as_str) {
        return name.to_owned();
    }
    match value.and_then(rmpv::Value::as_i64) {
        Some(0) => "Image".to_owned(),
        Some(1) => "Watercolor".to_owned(),
        Some(3) => "Pencil".to_owned(),
        Some(4) => "Fill".to_owned(),
        Some(5) => "Marker".to_owned(),
        Some(7) => "Airbrush".to_owned(),
        Some(9) => "Pen".to_owned(),
        Some(10) => "Wire".to_owned(),
        Some(11) => "Text".to_owned(),
        _ if index == 0 => "Pen".to_owned(),
        _ => format!("Layer {}", index + 1),
    }
}

fn parse_object<F>(object: &rmpv::Value, read_resource: &mut F) -> Result<DrawingObject>
where
    F: FnMut(&str) -> Result<Vec<u8>>,
{
    let object_fields = array(object, "drawing object")?;
    let object_type = integer(required(object_fields, 0, "drawing object type")?)?;
    match object_type {
        3 | 4 | 5 | 6 | 9 | 10 | 11 | 15 | 4096 => parse_stroke(object).map(DrawingObject::Stroke),
        7 => parse_image(object, read_resource).map(DrawingObject::Image),
        13 => parse_text(object).map(DrawingObject::Text),
        _ => bail!("unsupported drawing object type {object_type}"),
    }
}

fn parse_text(object: &rmpv::Value) -> Result<Text> {
    let object_fields = array(object, "text object")?;
    let body = array(
        required(object_fields, 1, "text object body")?,
        "text object body",
    )?;
    let node = array(required(body, 1, "text node")?, "text node")?;
    if integer(required(node, 0, "text node type")?)? != 3 {
        bail!("unsupported text node type");
    }

    let id_bytes = extension(required(node, 2, "text UUID")?, 5, "text UUID")?;
    if id_bytes.len() != 16 {
        bail!("text UUID has invalid length {}", id_bytes.len());
    }

    let style = array(required(node, 1, "text style")?, "text style")?;
    if integer(required(style, 0, "text style type")?)? != 4 {
        bail!("unsupported text style type");
    }
    let style_body = array(required(style, 1, "text style body")?, "text style body")?;
    let color_fields = array(required(style_body, 1, "text color")?, "text color")?;
    let rgba = extension(required(color_fields, 2, "text RGBA")?, 4, "text RGBA")?;
    if rgba.len() != 16 {
        bail!("text RGBA has invalid length {}", rgba.len());
    }
    let channels = rgba
        .chunks_exact(4)
        .map(|bytes| f32::from_le_bytes(bytes.try_into().expect("four-byte channel")))
        .collect::<Vec<_>>();
    if channels.iter().any(|channel| !channel.is_finite()) {
        bail!("text color contains a non-finite channel");
    }
    let color = format!(
        "#{:02x}{:02x}{:02x}",
        color_channel(channels[0]),
        color_channel(channels[1]),
        color_channel(channels[2])
    );
    let opacity = number(required(color_fields, 3, "text opacity")?)?.clamp(0.0, 1.0)
        * channels[3].clamp(0.0, 1.0);
    let content = required(body, 2, "text content")?
        .as_str()
        .context("text content is not a UTF-8 string")?
        .to_owned();
    let transform = parse_transform(extension(
        required(node, 7, "text transform")?,
        7,
        "text transform",
    )?)?;
    if !transform.is_affine() {
        bail!("text has a projective transform that SVG text cannot represent");
    }

    Ok(Text {
        id: format_uuid(id_bytes),
        content,
        color,
        opacity,
        transform,
    })
}

fn parse_stroke(object: &rmpv::Value) -> Result<Stroke> {
    let object_fields = array(object, "drawing object")?;
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
    let transform = parse_transform(extension(
        required(style, 7, "stroke transform")?,
        7,
        "stroke transform",
    )?)?;
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
    let filled = brush_variant == 11;
    let width_value = required(parameters, 3, "stroke width")?;
    let width = if filled && matches!(width_value, rmpv::Value::Nil) {
        0.0
    } else {
        number(width_value)?
            * match brush_variant {
                5 => 0.63,
                6 => 0.5,
                12 => 0.85,
                _ => 1.0,
            }
    };
    if !width.is_finite() || width < 0.0 || (width == 0.0 && !filled) {
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
        let source_x = f32::from_le_bytes(record[0..4].try_into().expect("four-byte x"));
        let source_y = f32::from_le_bytes(record[4..8].try_into().expect("four-byte y"));
        let (x, transformed_y) = transform
            .apply(source_x, source_y)
            .context("transforming stroke point")?;
        let y = SVG_Y_FLIP - transformed_y;
        if !x.is_finite() || !y.is_finite() {
            bail!("stroke point contains a non-finite coordinate");
        }
        points.push(Point { x, y });
    }

    let shape_data = array(
        required(fields, 16, "stroke shape metadata")?,
        "stroke shape metadata",
    )?;
    let closed = points.len() >= 3 && shape_data.len() == points.len() + 1;

    Ok(Stroke {
        id,
        color,
        width,
        opacity,
        closed,
        filled,
        line_cap: if brush_variant == 8 { "butt" } else { "round" },
        line_join: if brush_variant == 8 { "bevel" } else { "round" },
        dasharray: (brush_variant == 6).then_some((width * 0.1, width * 1.5)),
        filter: match brush_variant {
            5 | 12 => Some("FILTER_airbrush_24"),
            8 => Some("FILTER_marker"),
            14 | 15 => Some("FILTER_pencil"),
            _ => None,
        },
        points,
    })
}

fn parse_image<F>(object: &rmpv::Value, read_resource: &mut F) -> Result<EmbeddedImage>
where
    F: FnMut(&str) -> Result<Vec<u8>>,
{
    let object_fields = array(object, "image object")?;
    let body = array(
        required(object_fields, 1, "image object body")?,
        "image object body",
    )?;
    let metadata = array(required(body, 1, "image metadata")?, "image metadata")?;
    if integer(required(metadata, 0, "image metadata type")?)? != 3 {
        bail!("unsupported image metadata type");
    }
    let id_bytes = extension(
        required(metadata, 2, "image object UUID")?,
        5,
        "image object UUID",
    )?;
    if id_bytes.len() != 16 {
        bail!("image object UUID has invalid length {}", id_bytes.len());
    }
    let id = format_uuid(id_bytes);

    let resource_bytes = extension(
        required(body, 2, "image resource UUID")?,
        5,
        "image resource UUID",
    )?;
    if resource_bytes.len() != 16 {
        bail!(
            "image resource UUID has invalid length {}",
            resource_bytes.len()
        );
    }
    let resource_id = format_uuid(resource_bytes);
    let mut embedded_resource = None;
    let mut resource_error = None;
    for extension in ["png", "jpg", "jpeg"] {
        let image_path = format!("resources/{resource_id}.{extension}");
        match read_resource(&image_path) {
            Ok(bytes) => {
                embedded_resource = Some(bytes);
                break;
            }
            Err(error) => resource_error = Some(error),
        }
    }
    let bytes = match embedded_resource {
        Some(bytes) => bytes,
        None => {
            return Err(resource_error
                .unwrap_or_else(|| anyhow::anyhow!("image resource {resource_id} was not found")))
            .with_context(|| format!("reading embedded image resource {resource_id}"));
        }
    };
    let (pixel_width, pixel_height, media_type) = raster_image_info(&bytes)?;

    let display_size = extension(
        required(body, 3, "image display size")?,
        1,
        "image display size",
    )?;
    if display_size.len() != 8 {
        bail!(
            "image display size has invalid length {}",
            display_size.len()
        );
    }
    let display_width = f32::from_le_bytes(display_size[0..4].try_into().expect("four-byte width"));
    let display_height =
        f32::from_le_bytes(display_size[4..8].try_into().expect("four-byte height"));
    if !display_width.is_finite()
        || !display_height.is_finite()
        || display_width <= 0.0
        || display_height <= 0.0
    {
        bail!("image has invalid display size {display_width}x{display_height}");
    }

    let transform = parse_transform(extension(
        required(body, 5, "image transform")?,
        7,
        "image transform",
    )?)?;
    if !transform.is_affine() {
        bail!("embedded image has a projective transform that SVG cannot represent");
    }
    Ok(EmbeddedImage {
        id,
        resource_id,
        bytes,
        media_type,
        pixel_width,
        pixel_height,
        display_width,
        display_height,
        transform,
    })
}

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

fn raster_image_info(bytes: &[u8]) -> Result<(u32, u32, &'static str)> {
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

fn parse_transform(bytes: &[u8]) -> Result<PlanarTransform> {
    if bytes.len() != 16 * 4 {
        bail!("stroke transform has invalid length {}", bytes.len());
    }
    let values = bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().expect("four-byte float")))
        .collect::<Vec<_>>();
    if values.iter().any(|value| !value.is_finite()) {
        bail!("stroke transform contains a non-finite value");
    }
    // Points have z=0, so their planar transform is the corresponding 3x3
    // homography in the x/y/w components of the stored row-vector matrix.
    Ok(PlanarTransform {
        m11: values[0],
        m12: values[1],
        m21: values[4],
        m22: values[5],
        m41: values[12],
        m42: values[13],
        m14: values[3],
        m24: values[7],
        m44: values[15],
    })
}

fn vector_svg(drawing: &Drawing) -> Result<String> {
    let mut bounds: Option<(f32, f32, f32, f32)> = None;
    let mut max_width: f32 = 0.0;

    let mut include_point = |x: f32, y: f32| {
        bounds = Some(match bounds {
            None => (x, y, x, y),
            Some((min_x, min_y, max_x, max_y)) => {
                (min_x.min(x), min_y.min(y), max_x.max(x), max_y.max(y))
            }
        });
    };
    for layer in &drawing.layers {
        if !layer.visible || layer.opacity <= 0.0 {
            continue;
        }
        for object in &layer.objects {
            match object {
                DrawingObject::Stroke(stroke) => {
                    max_width = max_width.max(stroke.width);
                    for point in &stroke.points {
                        include_point(point.x, point.y);
                    }
                }
                DrawingObject::Image(image) => {
                    let half_width = image.display_width / 2.0;
                    let half_height = image.display_height / 2.0;
                    for (local_x, local_y) in [
                        (-half_width, half_height),
                        (half_width, half_height),
                        (half_width, -half_height),
                        (-half_width, -half_height),
                    ] {
                        let (x, y) = image.transform.apply(local_x, local_y)?;
                        include_point(x, SVG_Y_FLIP - y);
                    }
                }
                DrawingObject::Text(text) => {
                    let half_advance = text.content.chars().count() as f32 * TEXT_FONT_SIZE * 0.6;
                    for (local_x, local_y) in [
                        (-half_advance, TEXT_ORIGIN_Y_OFFSET),
                        (half_advance, TEXT_ORIGIN_Y_OFFSET),
                        (-half_advance, TEXT_ORIGIN_Y_OFFSET + TEXT_FONT_SIZE + 5.0),
                        (half_advance, TEXT_ORIGIN_Y_OFFSET + TEXT_FONT_SIZE + 5.0),
                    ] {
                        let (x, y) = text.transform.apply_svg(local_x, local_y)?;
                        include_point(x, y);
                    }
                }
            }
        }
    }
    let empty = bounds.is_none();
    let (min_x, min_y, max_x, max_y) = bounds.unwrap_or((0.0, -256.0, 1024.0, 768.0));
    let padding = if empty { 0.0 } else { max_width / 2.0 + 4.0 };
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
    let images = drawing
        .layers
        .iter()
        .flat_map(|layer| layer.objects.iter())
        .filter_map(|object| match object {
            DrawingObject::Image(image) => Some(image),
            DrawingObject::Stroke(_) | DrawingObject::Text(_) => None,
        })
        .collect::<Vec<_>>();
    let has_filters = drawing.layers.iter().any(|layer| {
        layer.objects.iter().any(
            |object| matches!(object, DrawingObject::Stroke(stroke) if stroke.filter.is_some()),
        )
    });
    if !images.is_empty() || has_filters {
        writeln!(svg, "  <defs>")?;
        if has_filters {
            write_brush_filters(&mut svg, drawing)?;
        }
        let mut defined_resources = HashSet::new();
        for image in &images {
            if !defined_resources.insert(image.resource_id.as_str()) {
                continue;
            }
            writeln!(
                svg,
                "    <image id=\"IMAGE_DEF_{}\" width=\"{}\" height=\"{}\" href=\"data:{};base64,{}\" />",
                image.resource_id,
                image.pixel_width,
                image.pixel_height,
                image.media_type,
                STANDARD.encode(&image.bytes)
            )?;
        }
        writeln!(svg, "  </defs>")?;
    }
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
        if !layer.visible || layer.opacity <= 0.0 || layer.objects.is_empty() {
            continue;
        }
        let id = xml_escape(&layer_id(&layer.name, layer_index));
        writeln!(
            svg,
            "  <g id=\"{id}\" opacity=\"{}\">",
            format_num(layer.opacity)
        )?;
        for object in &layer.objects {
            match object {
                DrawingObject::Stroke(stroke) => write_stroke(&mut svg, stroke)?,
                DrawingObject::Image(image) => write_image(&mut svg, image)?,
                DrawingObject::Text(text) => write_text(&mut svg, text)?,
            }
        }
        writeln!(svg, "  </g>")?;
    }
    writeln!(svg, "</svg>")?;
    Ok(svg)
}

fn write_image(svg: &mut String, image: &EmbeddedImage) -> Result<()> {
    use std::fmt::Write as _;

    if !image.transform.is_affine() {
        bail!("embedded image has a projective transform that SVG cannot represent");
    }
    let denominator = image.transform.m44;
    let m11 = image.transform.m11 / denominator;
    let m12 = image.transform.m12 / denominator;
    let m21 = image.transform.m21 / denominator;
    let m22 = image.transform.m22 / denominator;
    let tx = image.transform.m41 / denominator;
    let ty = image.transform.m42 / denominator;
    let scale_x = image.display_width / image.pixel_width as f32;
    let scale_y = image.display_height / image.pixel_height as f32;
    let matrix = [
        m11 * scale_x,
        -m12 * scale_x,
        -m21 * scale_y,
        m22 * scale_y,
        tx - image.display_width / 2.0 * m11 + image.display_height / 2.0 * m21,
        SVG_Y_FLIP - ty + image.display_width / 2.0 * m12 - image.display_height / 2.0 * m22,
    ];
    if matrix.iter().any(|value| !value.is_finite()) {
        bail!("embedded image produces a non-finite SVG transform");
    }
    let id = format!("IMAGE_{}", image.id);
    let matrix = matrix
        .iter()
        .map(|value| format_matrix_num(*value))
        .collect::<Vec<_>>()
        .join(" ");
    writeln!(
        svg,
        "    <g id=\"{id}\" opacity=\"1.000\" transform=\"matrix({matrix})\">"
    )?;
    writeln!(
        svg,
        "      <use href=\"#IMAGE_DEF_{}\" width=\"{}\" height=\"{}\" />",
        image.resource_id, image.pixel_width, image.pixel_height
    )?;
    writeln!(svg, "    </g>")?;
    Ok(())
}

fn write_text(svg: &mut String, text: &Text) -> Result<()> {
    use std::fmt::Write as _;

    if !text.transform.is_affine() {
        bail!("text has a projective transform that SVG text cannot represent");
    }
    let denominator = text.transform.m44;
    let matrix = [
        text.transform.m11 / denominator,
        -text.transform.m12 / denominator,
        -text.transform.m21 / denominator,
        text.transform.m22 / denominator,
        text.transform.m41 / denominator,
        SVG_Y_FLIP - text.transform.m42 / denominator,
    ]
    .iter()
    .map(|value| format_matrix_num(*value))
    .collect::<Vec<_>>()
    .join(" ");
    let id = format!("TEXT_{}", text.id);
    let opacity = format_num(text.opacity);
    writeln!(
        svg,
        "    <g id=\"{id}\" opacity=\"{opacity}\" transform=\"matrix({matrix})\">"
    )?;
    writeln!(
        svg,
        "      <text x=\"0\" y=\"{}\" text-anchor=\"middle\" alignment-baseline=\"mathematical\" font-size=\"{}\" font-weight=\"lighter\" fill=\"{}\" font-family=\"'Helvetica Neue', Helvetica, Arial, sans-serif\">",
        format_num(TEXT_ORIGIN_Y_OFFSET),
        format_num(TEXT_FONT_SIZE),
        xml_escape(&text.color)
    )?;
    writeln!(
        svg,
        "        <tspan x=\"0\" dy=\"1.000em\">{}</tspan>",
        xml_escape(&text.content)
    )?;
    writeln!(svg, "      </text>")?;
    writeln!(svg, "    </g>")?;
    Ok(())
}

fn write_brush_filters(svg: &mut String, drawing: &Drawing) -> Result<()> {
    use std::fmt::Write as _;

    let filters = drawing
        .layers
        .iter()
        .flat_map(|layer| layer.objects.iter())
        .filter_map(|object| match object {
            DrawingObject::Stroke(stroke) => stroke.filter,
            DrawingObject::Image(_) | DrawingObject::Text(_) => None,
        })
        .collect::<HashSet<_>>();
    if filters.contains("FILTER_airbrush_24") {
        writeln!(
            svg,
            "    <filter id=\"FILTER_airbrush_24\" filterUnits=\"userSpaceOnUse\"><feGaussianBlur in=\"SourceGraphic\" stdDeviation=\"8\" /></filter>"
        )?;
    }
    if filters.contains("FILTER_pencil") {
        // Concepts exports a raster texture for pencil. Keep the SVG vector-only
        // and preserve a filter hook while approximating that texture as smooth ink.
        writeln!(
            svg,
            "    <filter id=\"FILTER_pencil\" filterUnits=\"objectBoundingBox\" x=\"-8\" y=\"-8\" width=\"16\" height=\"16\" />"
        )?;
    }
    if filters.contains("FILTER_marker") {
        writeln!(
            svg,
            "    <filter id=\"FILTER_marker\" x=\"-5\" y=\"-5\" width=\"10\" height=\"10\"><feTurbulence baseFrequency=\"10\" type=\"fractalNoise\" seed=\"1\" numOctaves=\"7\" result=\"turbulence\" /><feColorMatrix type=\"luminanceToAlpha\" result=\"alpha\" in=\"turbulence\" /><feComposite operator=\"atop\" in=\"alpha\" in2=\"SourceGraphic\" /></filter>"
        )?;
    }
    Ok(())
}

fn write_stroke(svg: &mut String, stroke: &Stroke) -> Result<()> {
    use std::fmt::Write as _;
    let opacity = format_num(stroke.opacity);
    let id = format!("STROKE_{}", stroke.id);
    if stroke.points.len() == 1 {
        let point = &stroke.points[0];
        writeln!(
            svg,
            "    <circle id=\"{id}\" opacity=\"{opacity}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"0.1\" stroke-opacity=\"{opacity}\" fill-opacity=\"{opacity}\" cx=\"{}\" cy=\"{}\" r=\"{}\" />",
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
        "    <path id=\"{id}\" opacity=\"{opacity}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"{}\" stroke-opacity=\"{opacity}\" stroke-linecap=\"{}\" stroke-linejoin=\"{}\"",
        if stroke.filled {
            stroke.color.as_str()
        } else {
            "none"
        },
        stroke.color,
        format_num(stroke.width),
        stroke.line_cap,
        stroke.line_join
    )?;
    if let Some((dash, gap)) = stroke.dasharray {
        write!(
            svg,
            " stroke-dasharray=\"{},{}\"",
            format_num(dash),
            format_num(gap)
        )?;
    }
    if let Some(filter) = stroke.filter {
        write!(svg, " filter=\"url(#{filter})\"")?;
    }
    write!(
        svg,
        " d=\"M {} {}",
        format_num(stroke.points[0].x),
        format_num(stroke.points[0].y)
    )?;
    if stroke.closed {
        for point in stroke.points.iter().skip(1) {
            write!(svg, " L {} {}", format_num(point.x), format_num(point.y))?;
        }
        write!(svg, " Z")?;
    } else if stroke.points.len() == 2 {
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

fn format_matrix_num(value: f32) -> String {
    if value.abs() < 0.000_000_5 {
        "0.000000".to_owned()
    } else {
        format!("{value:.6}")
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
        ConversionKind, Drawing, Layer, SVG_Y_FLIP, convert_file, convert_file_with_policy,
        jpeg_dimensions, parse_transform, parse_tree, raster_image_info, thumbnail_svg, vector_svg,
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
        let identity = [
            1.0_f32, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect::<Vec<_>>();
        let style = msg_array([
            msg_int(3),
            style_group,
            msg_ext(5, &uuid),
            Value::Nil,
            Value::Nil,
            msg_int(0),
            Value::F64(0.0),
            msg_ext(7, &identity),
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
    fn detects_embedded_jpeg_media_type_and_dimensions() {
        assert_eq!(
            raster_image_info(&minimal_jpeg(288, 407)).unwrap(),
            (288, 407, "image/jpeg")
        );
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
        let drawing =
            parse_tree(&synthetic_tree(), "#000000".to_owned(), |_| Ok(Vec::new())).unwrap();
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
            |_| Ok(Vec::new()),
        )
        .unwrap();
        let svg = vector_svg(&drawing).unwrap();
        assert!(svg.contains("stroke-width=\"2.500\""));
        assert!(svg.contains("stroke-opacity=\"0.694\""));
    }

    #[test]
    fn applies_stored_row_vector_affine_transform_before_svg_y_flip() {
        let values = [
            1.096_863_6_f32,
            -0.831_043_24,
            0.0,
            0.0,
            0.831_043_24,
            1.096_863_6,
            0.0,
            0.0,
            0.0,
            0.0,
            1.0,
            0.0,
            271.291_6,
            69.725_91,
            0.0,
            1.0,
        ];
        let bytes = values
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect::<Vec<_>>();
        let transform = parse_transform(&bytes).unwrap();
        let (x, y) = transform.apply(66.481_05, -343.740_4).unwrap();
        assert!((x - 58.549).abs() < 0.001);
        assert!((SVG_Y_FLIP - y - 1130.559).abs() < 0.001);
    }

    #[test]
    fn applies_planar_projective_transform_with_perspective_divide() {
        let values = [
            1.0_f32, 0.0, 0.0, 0.1, 0.0, 1.0, 0.0, 0.2, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 2.0,
        ];
        let bytes = values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        let transform = parse_transform(&bytes).unwrap();
        assert!(!transform.is_affine());
        let (x, y) = transform.apply(3.0, 1.0).unwrap();
        assert!((x - 1.2).abs() < 0.000_001);
        assert!((y - 0.4).abs() < 0.000_001);
    }

    #[test]
    fn renders_empty_drawing_as_background_only_vector_svg() {
        let drawing = Drawing {
            background: "#000000".to_owned(),
            layers: vec![Layer {
                name: "Pen".to_owned(),
                visible: true,
                opacity: 1.0,
                objects: Vec::new(),
            }],
        };
        let svg = vector_svg(&drawing).unwrap();
        assert!(svg.contains("viewBox=\"0.000 -256.000 1024.000 1024.000\""));
        assert!(svg.contains("fill=\"#000000\""));
        assert!(!svg.contains("<g "));
        assert!(!svg.contains("<image "));
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
    fn strict_conversion_rejects_fallback_without_writing_output() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("drawing.concepts");
        let output = dir.path().join("drawing.svg");
        std::fs::write(&input, archive_with_thumbnail(&minimal_jpeg(100, 80))).unwrap();

        assert!(convert_file_with_policy(&input, &output, true).is_err());
        assert!(!output.exists());
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
