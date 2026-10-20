use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Parser;

#[derive(Debug, Parser)]
#[command(version, about = "Convert Concepts drawings to SVG")]
struct Args {
    /// Input .concepts file
    input: PathBuf,

    /// Output SVG path (defaults to the input path with a .svg extension)
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Fail instead of embedding the thumbnail when vector conversion is unsupported
    #[arg(long)]
    require_vector: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let has_explicit_output = args.output.is_some();
    let output = args
        .output
        .unwrap_or_else(|| args.input.with_extension("svg"));

    if same_file(&args.input, &output)? {
        bail!("input and output paths must be different");
    }
    if !has_explicit_output && output.exists() {
        bail!(
            "refusing to overwrite existing {} (specify --output to choose another path)",
            output.display()
        );
    }

    let kind = concepts_to_svg::convert_file_with_policy(&args.input, &output, args.require_vector)
        .with_context(|| format!("converting {}", args.input.display()))?;
    println!("Wrote {} ({kind})", output.display());
    Ok(())
}

fn same_file(input: &std::path::Path, output: &std::path::Path) -> Result<bool> {
    if output.exists() {
        return Ok(input.canonicalize()? == output.canonicalize()?);
    }
    Ok(input == output)
}
