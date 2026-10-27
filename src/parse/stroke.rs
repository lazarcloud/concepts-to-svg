use super::*;

pub(super) fn parse_stroke(object: &rmpv::Value) -> Result<Stroke> {
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
    let rgba_value = required(color_fields, 2, "stroke RGBA")?;
    let is_eraser = matches!(rgba_value, rmpv::Value::Nil);
    let channels = if is_eraser {
        // Concepts' mask-like stroke records may omit RGBA; its SVG export
        // represents those records as white strokes.
        vec![1.0; 4]
    } else {
        let rgba = extension(rgba_value, 4, "stroke RGBA")?;
        if rgba.len() != 16 {
            bail!("stroke RGBA has invalid length {}", rgba.len());
        }
        rgba.chunks_exact(4)
            .map(|bytes| f32::from_le_bytes(bytes.try_into().expect("four-byte chunk")))
            .collect::<Vec<_>>()
    };
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
                13 => 0.63,
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
        is_eraser,
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
