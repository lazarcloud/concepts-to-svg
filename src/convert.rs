use crate::archive::{parse_vector_archive, read_thumbnail};
use crate::raster::jpeg_dimensions;
use crate::svg::{thumbnail_svg, vector_svg};
use anyhow::{Context, Result, bail};
use std::{fs::File, path::Path};
use zip::ZipArchive;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversionKind {
    Vector,
    ThumbnailFallback,
}

impl std::fmt::Display for ConversionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Vector => f.write_str("vector"),
            Self::ThumbnailFallback => f.write_str("thumbnail fallback"),
        }
    }
}

/// Converts a Concepts archive to SVG, falling back to its embedded thumbnail
/// if the vector records contain unsupported content.
pub fn convert_file(input: &Path, output: &Path) -> Result<ConversionKind> {
    convert_file_with_policy(input, output, false)
}

/// Converts an archive and optionally rejects thumbnail fallback output.
pub fn convert_file_with_policy(
    input: &Path,
    output: &Path,
    require_vector: bool,
) -> Result<ConversionKind> {
    let file = File::open(input).with_context(|| format!("opening {}", input.display()))?;
    let mut archive = ZipArchive::new(file).context("reading .concepts ZIP archive")?;
    let jpeg = read_thumbnail(&mut archive)?;
    let (width, height) = jpeg_dimensions(&jpeg).context("reading thumbnail dimensions")?;

    let (svg, kind) = match parse_vector_archive(&mut archive) {
        Ok(drawing) => match vector_svg(&drawing) {
            Ok(svg) => (svg, ConversionKind::Vector),
            Err(error) => {
                eprintln!("warning: {error:#}; using embedded thumbnail instead");
                (
                    thumbnail_svg(&jpeg, width, height),
                    ConversionKind::ThumbnailFallback,
                )
            }
        },
        Err(error) => {
            eprintln!("warning: {error:#}; using embedded thumbnail instead");
            (
                thumbnail_svg(&jpeg, width, height),
                ConversionKind::ThumbnailFallback,
            )
        }
    };

    if require_vector && kind != ConversionKind::Vector {
        bail!(
            "{} requires vector conversion, but only thumbnail fallback was available",
            input.display()
        );
    }

    std::fs::write(output, svg).with_context(|| format!("writing {}", output.display()))?;
    Ok(kind)
}
