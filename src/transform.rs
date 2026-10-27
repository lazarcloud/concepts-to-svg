use anyhow::{Result, bail};

pub(crate) const SVG_Y_FLIP: f32 = 768.0;

#[derive(Debug, Clone, Copy)]
pub(crate) struct PlanarTransform {
    pub(crate) m11: f32,
    pub(crate) m12: f32,
    pub(crate) m21: f32,
    pub(crate) m22: f32,
    pub(crate) m41: f32,
    pub(crate) m42: f32,
    pub(crate) m14: f32,
    pub(crate) m24: f32,
    pub(crate) m44: f32,
}

impl PlanarTransform {
    pub(crate) fn apply(self, x: f32, y: f32) -> Result<(f32, f32)> {
        let denominator = x * self.m14 + y * self.m24 + self.m44;
        if !denominator.is_finite() || denominator.abs() <= f32::EPSILON {
            bail!("transform maps a point to infinity");
        }
        let transformed = (
            (x * self.m11 + y * self.m21 + self.m41) / denominator,
            (x * self.m12 + y * self.m22 + self.m42) / denominator,
        );
        if !transformed.0.is_finite() || !transformed.1.is_finite() {
            bail!("transform produces a non-finite point");
        }
        Ok(transformed)
    }

    pub(crate) fn is_affine(self) -> bool {
        self.m14.abs() <= 1e-6 && self.m24.abs() <= 1e-6 && self.m44.abs() > f32::EPSILON
    }

    pub(crate) fn apply_svg(self, x: f32, y: f32) -> Result<(f32, f32)> {
        // SVG text cannot represent projective matrices. Concepts' SVG export
        // approximates transformed text with the affine portion of its matrix.
        let denominator = self.m44;
        let transformed = (
            (x * self.m11 - y * self.m21 + self.m41) / denominator,
            (-x * self.m12 + y * self.m22) / denominator + SVG_Y_FLIP - self.m42 / denominator,
        );
        if !transformed.0.is_finite() || !transformed.1.is_finite() {
            bail!("text transform produces a non-finite point");
        }
        Ok(transformed)
    }
}

pub(crate) fn parse_transform(bytes: &[u8]) -> Result<PlanarTransform> {
    if bytes.len() != 16 * 4 {
        bail!("stroke transform has invalid length {}", bytes.len());
    }
    let values = bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().expect("four-byte float")))
        .collect::<Vec<_>>();
    if values.iter().any(|value| !value.is_finite()) {
        bail!("stroke transform contains a non-finite value");
    }
    // Points have z=0, so their planar transform is the corresponding 3x3
    // homography in the x/y/w components of the stored row-vector matrix.
    Ok(PlanarTransform {
        m11: values[0],
        m12: values[1],
        m21: values[4],
        m22: values[5],
        m41: values[12],
        m42: values[13],
        m14: values[3],
        m24: values[7],
        m44: values[15],
    })
}
