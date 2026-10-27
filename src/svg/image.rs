use super::*;

pub(super) fn write_image(svg: &mut String, image: &EmbeddedImage) -> Result<()> {
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
