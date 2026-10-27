mod filters;
mod image;
mod stroke;
mod text;

use filters::write_brush_filters;
use image::write_image;
use stroke::write_stroke;
use text::write_text;

use crate::model::*;
use crate::transform::SVG_Y_FLIP;
use anyhow::{Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::collections::HashSet;

const SVG_MILLIMETERS_PER_UNIT: f32 = 0.192_424;
// The observed Concepts text record does not carry explicit font metrics.
const TEXT_FONT_SIZE: f32 = 24.0;
const TEXT_ORIGIN_Y_OFFSET: f32 = -9.0;

pub(crate) fn vector_svg(drawing: &Drawing) -> Result<String> {
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
                DrawingObject::PdfPage(page) => {
                    let half_width = page.width / 2.0;
                    let half_height = page.height / 2.0;
                    for (local_x, local_y) in [
                        (-half_width, -half_height),
                        (half_width, -half_height),
                        (half_width, half_height),
                        (-half_width, half_height),
                    ] {
                        let (x, y) = page.transform.apply(local_x, local_y)?;
                        include_point(x, SVG_Y_FLIP - y);
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
            DrawingObject::Stroke(_) | DrawingObject::Text(_) | DrawingObject::PdfPage(_) => None,
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
        if !layer.visible
            || layer.opacity <= 0.0
            || !layer
                .objects
                .iter()
                .any(|object| !matches!(object, DrawingObject::PdfPage(_)))
        {
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
                DrawingObject::PdfPage(_) => {}
            }
        }
        writeln!(svg, "  </g>")?;
    }
    writeln!(svg, "</svg>")?;
    Ok(svg)
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

pub(crate) fn thumbnail_svg(jpeg: &[u8], width: u32, height: u32) -> String {
    let encoded = STANDARD.encode(jpeg);
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}px\" height=\"{height}px\" viewBox=\"0 0 {width} {height}\">\n\
         <image width=\"{width}\" height=\"{height}\" href=\"data:image/jpeg;base64,{encoded}\"/>\n\
         </svg>\n"
    )
}
