use crate::transform::PlanarTransform;

#[derive(Debug)]
pub(crate) struct Point {
    pub(crate) x: f32,
    pub(crate) y: f32,
}

#[derive(Debug)]
pub(crate) struct Stroke {
    pub(crate) id: String,
    pub(crate) is_eraser: bool,
    pub(crate) color: String,
    pub(crate) width: f32,
    pub(crate) opacity: f32,
    pub(crate) closed: bool,
    pub(crate) filled: bool,
    pub(crate) line_cap: &'static str,
    pub(crate) line_join: &'static str,
    pub(crate) dasharray: Option<(f32, f32)>,
    pub(crate) filter: Option<&'static str>,
    pub(crate) points: Vec<Point>,
}

#[derive(Debug)]
pub(crate) struct EmbeddedImage {
    pub(crate) id: String,
    pub(crate) resource_id: String,
    pub(crate) bytes: Vec<u8>,
    pub(crate) media_type: &'static str,
    pub(crate) pixel_width: u32,
    pub(crate) pixel_height: u32,
    pub(crate) display_width: f32,
    pub(crate) display_height: f32,
    pub(crate) transform: PlanarTransform,
}

#[derive(Debug)]
pub(crate) struct Text {
    pub(crate) id: String,
    pub(crate) content: String,
    pub(crate) color: String,
    pub(crate) opacity: f32,
    pub(crate) transform: PlanarTransform,
}

#[derive(Debug)]
pub(crate) struct PdfPage {
    pub(crate) width: f32,
    pub(crate) height: f32,
    pub(crate) transform: PlanarTransform,
}

#[derive(Debug)]
pub(crate) enum DrawingObject {
    Stroke(Stroke),
    Image(EmbeddedImage),
    Text(Text),
    PdfPage(PdfPage),
}

#[derive(Debug)]
pub(crate) struct Layer {
    pub(crate) name: String,
    pub(crate) visible: bool,
    pub(crate) opacity: f32,
    pub(crate) objects: Vec<DrawingObject>,
}

#[derive(Debug)]
pub(crate) struct Drawing {
    pub(crate) background: String,
    pub(crate) layers: Vec<Layer>,
}
