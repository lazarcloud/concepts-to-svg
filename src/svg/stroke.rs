use super::*;

pub(super) fn write_stroke(svg: &mut String, stroke: &Stroke) -> Result<()> {
    use std::fmt::Write as _;
    let opacity = format_num(stroke.opacity);
    let id = format!(
        "{}_{}",
        if stroke.is_eraser { "ERASER" } else { "STROKE" },
        stroke.id
    );
    if stroke.points.len() == 1 {
        let point = &stroke.points[0];
        write!(
            svg,
            "    <circle id=\"{id}\" opacity=\"{opacity}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"0.1\" stroke-opacity=\"{opacity}\" fill-opacity=\"{opacity}\" cx=\"{}\" cy=\"{}\" r=\"{}\"",
            stroke.color,
            stroke.color,
            format_num(point.x),
            format_num(point.y),
            format_num(stroke.width / 2.0)
        )?;
        if let Some(filter) = stroke.filter {
            write!(svg, " filter=\"url(#{filter})\"")?;
        }
        writeln!(svg, " />")?;
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
