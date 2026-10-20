# concepts-to-svg

A Rust CLI that converts Concepts `.concepts` archives into SVG files.

```sh
cargo run -- drawing.concepts
cargo run -- drawing.concepts --output drawing-preview.svg
cargo run -- drawing.concepts --output drawing.svg --require-vector
```

The exporter decodes the current `tree.pack` MessagePack records and writes
stroke paths, dots, layers, colors, opacity, embedded PNG images, and a solid
background as SVG. It supports planar projective transforms on strokes and
affine transforms on images; an empty drawing is exported as an empty,
background-only vector SVG. Unknown or unsupported object records fall back to
the embedded `thumb.jpg`, preserving the preview at thumbnail resolution rather
than silently dropping content. Curves are reconstructed from the stored point
samples, so unusual brushes may differ from Concepts' own SVG exporter. The CLI
refuses to overwrite the default `.svg` output if it already exists; pass
`--output` to choose an explicit path.

Run tests with `cargo test`. The paired references in `tests/fixtures` are used
by an integration test that compares layer/object IDs, styles, embedded image
data and transforms, and sampled path geometry (within 1 drawing unit). Every
fixture must convert through the vector path; any thumbnail fallback fails the
test.

Pass `--require-vector` to make the CLI return an error instead of writing an
SVG that uses the thumbnail fallback. This is useful when vector output is a
hard requirement; some real-world drawings contain object types not yet
supported by the vector decoder.
