mod image;
mod pdf;
mod stroke;
mod text;

use image::parse_image;
use pdf::parse_pdf_page;
use stroke::parse_stroke;
use text::parse_text;

use crate::model::*;
use crate::raster::raster_image_info;
use crate::transform::{SVG_Y_FLIP, parse_transform};
use anyhow::{Context, Result, bail};

const POINT_RECORD_SIZE: usize = 16;

pub(crate) fn parse_tree<F>(
    tree: &rmpv::Value,
    background: String,
    mut read_resource: F,
) -> Result<Drawing>
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
        8 => parse_pdf_page(object, read_resource).map(DrawingObject::PdfPage),
        13 => parse_text(object).map(DrawingObject::Text),
        _ => bail!("unsupported drawing object type {object_type}"),
    }
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
