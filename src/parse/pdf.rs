use super::*;

pub(super) fn parse_pdf_page<F>(object: &rmpv::Value, read_resource: &mut F) -> Result<PdfPage>
where
    F: FnMut(&str) -> Result<Vec<u8>>,
{
    let object_fields = array(object, "PDF page object")?;
    let body = array(
        required(object_fields, 1, "PDF page body")?,
        "PDF page body",
    )?;
    let resource_id = extension(
        required(body, 2, "PDF resource UUID")?,
        5,
        "PDF resource UUID",
    )?;
    if resource_id.len() != 16 {
        bail!("PDF resource UUID has invalid length {}", resource_id.len());
    }
    let page_index = integer(required(body, 3, "PDF page index")?)?;
    if page_index < 0 {
        bail!("PDF page index is negative: {page_index}");
    }
    let path = format!("resources/{}.pdf", format_uuid(resource_id));
    let _pdf = read_resource(&path)?;

    let dimensions = extension(
        required(body, 4, "PDF page dimensions")?,
        1,
        "PDF page dimensions",
    )?;
    if dimensions.len() != 8 {
        bail!(
            "PDF page dimensions have invalid length {}",
            dimensions.len()
        );
    }
    let width = f32::from_le_bytes(dimensions[0..4].try_into().expect("four-byte width"));
    let height = f32::from_le_bytes(dimensions[4..8].try_into().expect("four-byte height"));
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        bail!("PDF page has invalid dimensions {width}x{height}");
    }
    let node = array(required(body, 1, "PDF page node")?, "PDF page node")?;
    let transform = parse_transform(extension(
        required(node, 7, "PDF page transform")?,
        7,
        "PDF page transform",
    )?)?;

    // Concepts' own SVG export omits imported PDF pages. Keep validating them
    // so that neighboring vector objects and the page's export bounds survive.
    Ok(PdfPage {
        width,
        height,
        transform,
    })
}
