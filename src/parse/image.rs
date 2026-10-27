use super::*;

pub(super) fn parse_image<F>(object: &rmpv::Value, read_resource: &mut F) -> Result<EmbeddedImage>
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
        required(metadata, 7, "image transform")?,
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
