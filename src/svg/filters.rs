use super::*;

pub(super) fn write_brush_filters(svg: &mut String, drawing: &Drawing) -> Result<()> {
    use std::fmt::Write as _;

    let filters = drawing
        .layers
        .iter()
        .flat_map(|layer| layer.objects.iter())
        .filter_map(|object| match object {
            DrawingObject::Stroke(stroke) => stroke.filter,
            DrawingObject::Image(_) | DrawingObject::Text(_) | DrawingObject::PdfPage(_) => None,
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
