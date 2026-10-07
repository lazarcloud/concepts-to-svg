# concepts-to-svg

A Rust CLI that converts Concepts `.concepts` archives into SVG files.

```sh
cargo run -- drawing.concepts
cargo run -- drawing.concepts --output drawing-preview.svg
```

The current exporter embeds the archive's `thumb.jpg` inside the SVG. This
preserves the drawing's preview appearance at thumbnail resolution and works
with the current sample files, but the result is a raster image wrapped in
SVG—not editable or resolution-independent vector paths. Concepts stores stroke
data in a packed binary format; decoding that geometry is future work. The CLI
refuses to overwrite the default `.svg` output if it already exists; pass
`--output` to choose an explicit path.

Run tests with `cargo test`.
