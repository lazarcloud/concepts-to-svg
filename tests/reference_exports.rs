use std::{collections::BTreeMap, fs, path::Path};

use concepts_to_svg::{ConversionKind, convert_file};
use roxmltree::{Document, Node};

const GEOMETRY_TOLERANCE: f64 = 1.0;

#[test]
fn generated_svgs_match_paired_exports_on_disk() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut archives = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "concepts"))
        .filter(|path| path.with_extension("svg").is_file())
        .collect::<Vec<_>>();
    archives.sort();

    if archives.is_empty() {
        eprintln!(
            "skipping reference SVG comparisons: no paired .concepts/.svg files in the project root"
        );
        return;
    }

    let output_dir = tempfile::tempdir().unwrap();
    let mut vector_comparisons = 0;

    for (index, archive) in archives.iter().enumerate() {
        let reference_path = archive.with_extension("svg");
        let generated_path = output_dir.path().join(format!("reference-{index}.svg"));
        let kind = convert_file(archive, &generated_path).unwrap();
        let reference_text = fs::read_to_string(&reference_path).unwrap();
        let generated_text = fs::read_to_string(&generated_path).unwrap();
        let reference = Document::parse(&reference_text).unwrap();
        let generated = Document::parse(&generated_text).unwrap();

        match kind {
            ConversionKind::Vector => {
                vector_comparisons += 1;
                compare_background(&reference, &generated, archive);
                compare_layers(&reference, &generated, archive);
                compare_objects(&reference, &generated, archive);
                compare_viewbox_aspect_ratio(&reference, &generated, archive);
            }
            ConversionKind::ThumbnailFallback => {
                panic!(
                    "{} converted via thumbnail fallback; paired-reference tests require vector output",
                    archive.display()
                );
            }
        }
    }

    assert!(
        vector_comparisons > 0,
        "paired reference files were found, but none exercised vector conversion"
    );
    eprintln!("compared {vector_comparisons} vector export(s) against on-disk references");
}

fn compare_background<'a, 'input>(
    reference: &Document<'input>,
    generated: &Document<'input>,
    archive: &Path,
) where
    'input: 'a,
{
    let background = |document: &Document<'_>| {
        document
            .descendants()
            .find(|node| node.tag_name().name() == "rect")
            .and_then(|node| node.attribute("fill"))
            .map(str::to_owned)
    };
    assert_eq!(
        background(reference),
        background(generated),
        "background mismatch for {}",
        archive.display()
    );
}

fn compare_layers(reference: &Document<'_>, generated: &Document<'_>, archive: &Path) {
    let layers = |document: &Document<'_>| {
        document
            .descendants()
            .filter(|node| node.tag_name().name() == "g")
            .map(|node| {
                (
                    node.attribute("id").unwrap_or_default().to_owned(),
                    node.attribute("opacity").unwrap_or_default().to_owned(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        layers(reference),
        layers(generated),
        "layer mismatch for {}",
        archive.display()
    );
}

fn compare_objects(reference: &Document<'_>, generated: &Document<'_>, archive: &Path) {
    let reference_objects = object_map(reference);
    let generated_objects = object_map(generated);
    assert_eq!(
        reference_objects.keys().collect::<Vec<_>>(),
        generated_objects.keys().collect::<Vec<_>>(),
        "object IDs differ for {}",
        archive.display()
    );

    for (id, expected) in &reference_objects {
        let actual = &generated_objects[id];
        assert_eq!(
            expected.tag_name().name(),
            actual.tag_name().name(),
            "object {id}"
        );
        for attribute in [
            "fill",
            "fill-opacity",
            "stroke",
            "stroke-width",
            "stroke-opacity",
            "stroke-linecap",
            "stroke-linejoin",
            "opacity",
        ] {
            assert_eq!(
                expected.attribute(attribute),
                actual.attribute(attribute),
                "{attribute} mismatch for object {id} in {}",
                archive.display()
            );
        }

        if expected.tag_name().name() == "path" {
            let expected_points = sample_path(expected.attribute("d").unwrap());
            let actual_points = sample_path(actual.attribute("d").unwrap());
            let discrepancy = symmetric_polyline_distance(&expected_points, &actual_points);
            assert!(
                discrepancy <= GEOMETRY_TOLERANCE,
                "geometry mismatch for object {id} in {}: sampled Hausdorff distance {discrepancy:.3} exceeds {GEOMETRY_TOLERANCE}",
                archive.display()
            );
        } else if expected.tag_name().name() == "circle" {
            for attribute in ["cx", "cy", "r"] {
                let expected = expected
                    .attribute(attribute)
                    .unwrap()
                    .parse::<f64>()
                    .unwrap();
                let actual = actual.attribute(attribute).unwrap().parse::<f64>().unwrap();
                assert!(
                    (expected - actual).abs() <= 0.01,
                    "{attribute} mismatch for object {id}: expected {expected}, got {actual}"
                );
            }
        }
    }
}

fn object_map<'a, 'input>(document: &'a Document<'input>) -> BTreeMap<String, Node<'a, 'input>> {
    document
        .descendants()
        .filter(|node| matches!(node.tag_name().name(), "path" | "circle"))
        .map(|node| (node.attribute("id").unwrap_or_default().to_owned(), node))
        .collect()
}

fn compare_viewbox_aspect_ratio(
    reference: &Document<'_>,
    generated: &Document<'_>,
    archive: &Path,
) {
    let parse = |document: &Document<'_>| {
        let values = document
            .root_element()
            .attribute("viewBox")
            .unwrap()
            .split_whitespace()
            .map(|value| value.parse::<f64>().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(values.len(), 4);
        values[2] / values[3]
    };
    let reference_ratio = parse(reference);
    let difference = (reference_ratio - parse(generated)).abs() / reference_ratio;
    assert!(
        difference < 0.02,
        "viewBox aspect ratio mismatch for {}: relative difference {difference:.4}",
        archive.display()
    );
}

fn sample_path(data: &str) -> Vec<(f64, f64)> {
    let tokens = data
        .replace(',', " ")
        .split_whitespace()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let mut points = Vec::new();
    let mut current = (0.0, 0.0);
    let mut subpath_start = current;
    let mut index = 0;
    while index < tokens.len() {
        match tokens[index].as_str() {
            "M" => {
                current = (parse_num(&tokens[index + 1]), parse_num(&tokens[index + 2]));
                subpath_start = current;
                points.push(current);
                index += 3;
            }
            "L" => {
                let end = (parse_num(&tokens[index + 1]), parse_num(&tokens[index + 2]));
                sample_line(current, end, &mut points);
                current = end;
                index += 3;
            }
            "Q" => {
                let control = (parse_num(&tokens[index + 1]), parse_num(&tokens[index + 2]));
                let end = (parse_num(&tokens[index + 3]), parse_num(&tokens[index + 4]));
                sample_quadratic(current, control, end, &mut points);
                current = end;
                index += 5;
            }
            "Z" => {
                sample_line(current, subpath_start, &mut points);
                current = subpath_start;
                index += 1;
            }
            command => panic!("unsupported path command {command:?} in reference test"),
        }
    }
    points
}

fn parse_num(value: &str) -> f64 {
    value.parse().unwrap()
}

fn sample_line(start: (f64, f64), end: (f64, f64), out: &mut Vec<(f64, f64)>) {
    const STEPS: usize = 12;
    for step in 1..=STEPS {
        let t = step as f64 / STEPS as f64;
        out.push((
            start.0 + (end.0 - start.0) * t,
            start.1 + (end.1 - start.1) * t,
        ));
    }
}

fn sample_quadratic(
    start: (f64, f64),
    control: (f64, f64),
    end: (f64, f64),
    out: &mut Vec<(f64, f64)>,
) {
    const STEPS: usize = 12;
    for step in 1..=STEPS {
        let t = step as f64 / STEPS as f64;
        let inverse = 1.0 - t;
        out.push((
            inverse * inverse * start.0 + 2.0 * inverse * t * control.0 + t * t * end.0,
            inverse * inverse * start.1 + 2.0 * inverse * t * control.1 + t * t * end.1,
        ));
    }
}

fn symmetric_polyline_distance(first: &[(f64, f64)], second: &[(f64, f64)]) -> f64 {
    directed_polyline_distance(first, second).max(directed_polyline_distance(second, first))
}

fn directed_polyline_distance(points: &[(f64, f64)], polyline: &[(f64, f64)]) -> f64 {
    points
        .iter()
        .map(|point| {
            polyline
                .windows(2)
                .map(|segment| point_to_segment_distance(*point, segment[0], segment[1]))
                .fold(f64::INFINITY, f64::min)
        })
        .fold(0.0, f64::max)
}

fn point_to_segment_distance(point: (f64, f64), start: (f64, f64), end: (f64, f64)) -> f64 {
    let dx = end.0 - start.0;
    let dy = end.1 - start.1;
    let length_squared = dx * dx + dy * dy;
    if length_squared == 0.0 {
        return euclidean_distance(point, start);
    }
    let t =
        (((point.0 - start.0) * dx + (point.1 - start.1) * dy) / length_squared).clamp(0.0, 1.0);
    euclidean_distance(point, (start.0 + t * dx, start.1 + t * dy))
}

fn euclidean_distance(first: (f64, f64), second: (f64, f64)) -> f64 {
    ((first.0 - second.0).powi(2) + (first.1 - second.1).powi(2)).sqrt()
}
