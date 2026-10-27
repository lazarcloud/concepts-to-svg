use super::*;

pub(super) fn write_text(svg: &mut String, text: &Text) -> Result<()> {
    use std::fmt::Write as _;

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
