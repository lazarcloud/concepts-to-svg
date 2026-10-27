use super::*;

pub(super) fn parse_text(object: &rmpv::Value) -> Result<Text> {
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
        // Concepts stores a trailing line break for text finalized with Enter,
        // but omits that final break from its SVG export.
        .trim_end_matches(['\r', '\n'])
        .to_owned();
    let transform = parse_transform(extension(
        required(node, 7, "text transform")?,
        7,
        "text transform",
    )?)?;

    Ok(Text {
        id: format_uuid(id_bytes),
        content,
        color,
        opacity,
        transform,
    })
}
