mod archive;
mod convert;
mod model;
mod parse;
mod raster;
mod svg;
mod transform;

pub use convert::{ConversionKind, convert_file, convert_file_with_policy};

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use zip::{ZipWriter, write::SimpleFileOptions};

    use super::{ConversionKind, convert_file, convert_file_with_policy};
    use crate::model::{Drawing, Layer};
    use crate::parse::parse_tree;
    use crate::raster::{jpeg_dimensions, raster_image_info};
    use crate::svg::{thumbnail_svg, vector_svg};
    use crate::transform::{SVG_Y_FLIP, parse_transform};
    use rmpv::Value;

    fn minimal_jpeg(width: u16, height: u16) -> Vec<u8> {
        // SOI, a baseline SOF segment with three components, then EOI.
        let mut jpeg = vec![
            0xff,
            0xd8,
            0xff,
            0xc0,
            0x00,
            0x11,
            0x08,
            (height >> 8) as u8,
            height as u8,
            (width >> 8) as u8,
            width as u8,
            0x03,
            0x01,
            0x11,
            0x00,
            0x02,
            0x11,
            0x00,
            0x03,
            0x11,
            0x00,
            0xff,
            0xd9,
        ];
        jpeg.shrink_to_fit();
        jpeg
    }

    fn archive_with_thumbnail(jpeg: &[u8]) -> Vec<u8> {
        let mut output = Cursor::new(Vec::new());
        {
            let mut writer = ZipWriter::new(&mut output);
            writer
                .start_file("thumb.jpg", SimpleFileOptions::default())
                .unwrap();
            std::io::Write::write_all(&mut writer, jpeg).unwrap();
            writer.finish().unwrap();
        }
        output.into_inner()
    }

    fn archive_with_vector_tree(jpeg: &[u8]) -> Vec<u8> {
        let mut output = Cursor::new(Vec::new());
        {
            let mut writer = ZipWriter::new(&mut output);
            writer
                .start_file("thumb.jpg", SimpleFileOptions::default())
                .unwrap();
            std::io::Write::write_all(&mut writer, jpeg).unwrap();
            writer
                .start_file("metadata.json", SimpleFileOptions::default())
                .unwrap();
            std::io::Write::write_all(&mut writer, br##"{"backgroundSolidColor":"#000000"}"##)
                .unwrap();
            writer
                .start_file("tree.pack", SimpleFileOptions::default())
                .unwrap();
            rmpv::encode::write_value(&mut writer, &synthetic_tree()).unwrap();
            writer.finish().unwrap();
        }
        output.into_inner()
    }

    fn msg_int(value: i64) -> Value {
        Value::Integer(value.into())
    }

    fn msg_array(values: impl IntoIterator<Item = Value>) -> Value {
        Value::Array(values.into_iter().collect())
    }

    fn msg_ext(kind: i8, bytes: &[u8]) -> Value {
        Value::Ext(kind, bytes.to_vec())
    }

    fn synthetic_tree() -> Value {
        synthetic_tree_with_brush(2, 3.0, 1.0)
    }

    fn synthetic_tree_with_brush(brush_variant: i64, width: f32, opacity: f32) -> Value {
        let uuid = [
            0x86, 0xb1, 0xb5, 0xe3, 0x23, 0x51, 0x45, 0x5f, 0x8f, 0x62, 0x89, 0xcb, 0x95, 0x1a,
            0x34, 0xa9,
        ];
        let mut rgba = Vec::new();
        for channel in [1.0_f32, 203.0 / 255.0, 18.0 / 255.0, 1.0] {
            rgba.extend_from_slice(&channel.to_le_bytes());
        }
        let mut points = Vec::new();
        for (x, y) in [(1.0_f32, 2.0_f32), (3.0, 4.0), (5.0, 6.0)] {
            points.extend_from_slice(&x.to_le_bytes());
            points.extend_from_slice(&y.to_le_bytes());
            points.extend_from_slice(&[0; 8]);
        }

        let color = msg_array([
            msg_int(0),
            msg_array([msg_int(0), msg_int(brush_variant)]),
            msg_ext(4, &rgba),
            Value::F32(opacity),
        ]);
        let parameters = msg_array([
            msg_int(1),
            color,
            Value::Nil,
            Value::F32(width),
            Value::F32(0.0),
        ]);
        let style_group = msg_array([msg_int(1), parameters]);
        let identity = [
            1.0_f32, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect::<Vec<_>>();
        let style = msg_array([
            msg_int(3),
            style_group,
            msg_ext(5, &uuid),
            Value::Nil,
            Value::Nil,
            msg_int(0),
            Value::F64(0.0),
            msg_ext(7, &identity),
            Value::Boolean(false),
            msg_int(17_038_118),
        ]);
        let stroke_fields = msg_array([
            msg_int(6),
            style,
            msg_int(2),
            Value::F32(1.0),
            Value::F32(0.1),
            Value::Boolean(false),
            Value::Boolean(true),
            Value::Boolean(true),
            Value::Boolean(true),
            Value::Boolean(true),
            Value::Boolean(true),
            Value::Boolean(true),
            Value::Boolean(false),
            msg_array([Value::F32(0.0), Value::F32(0.0)]),
            Value::Nil,
            Value::F32(0.0),
            msg_array([]),
            Value::Binary(points),
            Value::Binary(vec![0; 2]),
        ]);
        let object = msg_array([msg_int(11), msg_array([msg_int(0), stroke_fields])]);
        let layer_metadata = msg_array([
            msg_int(4),
            msg_ext(5, &[0; 16]),
            Value::Nil,
            Value::F64(0.0),
            msg_ext(7, &[0; 64]),
            Value::Boolean(false),
            Value::Boolean(true),
            Value::F32(1.0),
        ]);
        let layer = msg_array([
            msg_int(1),
            layer_metadata,
            msg_array([object]),
            Value::String("Pen".into()),
        ]);
        let document = msg_array([
            msg_int(0),
            msg_ext(5, &[0; 16]),
            msg_int(1),
            msg_array([
                msg_int(3),
                msg_int(0),
                msg_int(0),
                msg_int(2),
                Value::Boolean(false),
                Value::Boolean(false),
            ]),
            msg_array([layer]),
            msg_array([]),
            msg_array([]),
        ]);
        msg_array([msg_int(1027), document])
    }

    #[test]
    fn reads_dimensions_from_jpeg_frame() {
        assert_eq!(
            jpeg_dimensions(&minimal_jpeg(1024, 593)).unwrap(),
            (1024, 593)
        );
    }

    #[test]
    fn rejects_non_jpeg_thumbnail() {
        assert!(jpeg_dimensions(b"not a jpeg").is_err());
    }

    #[test]
    fn detects_embedded_jpeg_media_type_and_dimensions() {
        assert_eq!(
            raster_image_info(&minimal_jpeg(288, 407)).unwrap(),
            (288, 407, "image/jpeg")
        );
    }

    #[test]
    fn svg_embeds_thumbnail_without_reencoding_it() {
        let jpeg = minimal_jpeg(320, 200);
        let svg = thumbnail_svg(&jpeg, 320, 200);
        assert!(svg.contains("width=\"320px\" height=\"200px\""));
        assert!(svg.contains("viewBox=\"0 0 320 200\""));
        let data = svg
            .split("data:image/jpeg;base64,")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        assert_eq!(STANDARD.decode(data).unwrap(), jpeg);
    }

    #[test]
    fn decodes_stroke_points_color_and_canvas_y_axis() {
        let drawing =
            parse_tree(&synthetic_tree(), "#000000".to_owned(), |_| Ok(Vec::new())).unwrap();
        let svg = vector_svg(&drawing).unwrap();
        assert!(svg.contains("stroke=\"#ffcb12\""));
        assert!(svg.contains("stroke-width=\"3.000\""));
        assert!(svg.contains("M 1.000 766.000 Q 3.000 764.000 5.000 762.000"));
        assert!(svg.contains("id=\"STROKE_86b1b5e3-2351-455f-8f62-89cb951a34a9\""));
    }

    #[test]
    fn applies_observed_brush_width_scaling_and_opacity() {
        let drawing = parse_tree(
            &synthetic_tree_with_brush(6, 5.0, 0.694),
            "#000000".to_owned(),
            |_| Ok(Vec::new()),
        )
        .unwrap();
        let svg = vector_svg(&drawing).unwrap();
        assert!(svg.contains("stroke-width=\"2.500\""));
        assert!(svg.contains("stroke-opacity=\"0.694\""));
    }

    #[test]
    fn applies_stored_row_vector_affine_transform_before_svg_y_flip() {
        let values = [
            1.096_863_6_f32,
            -0.831_043_24,
            0.0,
            0.0,
            0.831_043_24,
            1.096_863_6,
            0.0,
            0.0,
            0.0,
            0.0,
            1.0,
            0.0,
            271.291_6,
            69.725_91,
            0.0,
            1.0,
        ];
        let bytes = values
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect::<Vec<_>>();
        let transform = parse_transform(&bytes).unwrap();
        let (x, y) = transform.apply(66.481_05, -343.740_4).unwrap();
        assert!((x - 58.549).abs() < 0.001);
        assert!((SVG_Y_FLIP - y - 1130.559).abs() < 0.001);
    }

    #[test]
    fn applies_planar_projective_transform_with_perspective_divide() {
        let values = [
            1.0_f32, 0.0, 0.0, 0.1, 0.0, 1.0, 0.0, 0.2, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 2.0,
        ];
        let bytes = values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        let transform = parse_transform(&bytes).unwrap();
        assert!(!transform.is_affine());
        let (x, y) = transform.apply(3.0, 1.0).unwrap();
        assert!((x - 1.2).abs() < 0.000_001);
        assert!((y - 0.4).abs() < 0.000_001);
    }

    #[test]
    fn renders_empty_drawing_as_background_only_vector_svg() {
        let drawing = Drawing {
            background: "#000000".to_owned(),
            layers: vec![Layer {
                name: "Pen".to_owned(),
                visible: true,
                opacity: 1.0,
                objects: Vec::new(),
            }],
        };
        let svg = vector_svg(&drawing).unwrap();
        assert!(svg.contains("viewBox=\"0.000 -256.000 1024.000 1024.000\""));
        assert!(svg.contains("fill=\"#000000\""));
        assert!(!svg.contains("<g "));
        assert!(!svg.contains("<image "));
    }

    #[test]
    fn falls_back_to_thumbnail_when_tree_pack_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("drawing.concepts");
        let output = dir.path().join("drawing.svg");
        std::fs::write(&input, archive_with_thumbnail(&minimal_jpeg(1024, 593))).unwrap();

        assert_eq!(
            convert_file(&input, &output).unwrap(),
            ConversionKind::ThumbnailFallback
        );
        let svg = std::fs::read_to_string(output).unwrap();
        assert!(svg.contains("viewBox=\"0 0 1024 593\""));
        assert!(svg.contains("data:image/jpeg;base64,"));
    }

    #[test]
    fn strict_conversion_rejects_fallback_without_writing_output() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("drawing.concepts");
        let output = dir.path().join("drawing.svg");
        std::fs::write(&input, archive_with_thumbnail(&minimal_jpeg(100, 80))).unwrap();

        assert!(convert_file_with_policy(&input, &output, true).is_err());
        assert!(!output.exists());
    }

    #[test]
    fn converts_supported_tree_records_to_vector_svg() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("drawing.concepts");
        let output = dir.path().join("drawing.svg");
        std::fs::write(&input, archive_with_vector_tree(&minimal_jpeg(1024, 593))).unwrap();

        assert_eq!(
            convert_file(&input, &output).unwrap(),
            ConversionKind::Vector
        );
        let svg = std::fs::read_to_string(output).unwrap();
        assert!(svg.contains("stroke=\"#ffcb12\""));
        assert!(!svg.contains("data:image/jpeg;base64,"));
    }
}
