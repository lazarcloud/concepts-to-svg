# concepts-to-svg

A Rust CLI that converts Concepts `.concepts` archives into SVG files.

```sh
cargo run -- drawing.concepts
cargo run -- drawing.concepts --output drawing-preview.svg
```

The exporter decodes the current `tree.pack` MessagePack records and writes
stroke paths, dots, layers, colors, opacity, and a solid background as SVG.
Unknown object records fall back to the embedded `thumb.jpg`, preserving the
preview at thumbnail resolution rather than silently dropping content. Curves
are reconstructed from the stored point samples, so unusual brushes may differ
from Concepts' own SVG exporter. The CLI refuses to overwrite the default
`.svg` output if it already exists; pass `--output` to choose an explicit path.

Run tests with `cargo test`. The paired references in `tests/fixtures` are used
by an integration test that compares layer/object IDs, styles, and sampled path
geometry (within 1 drawing unit). Every fixture must convert through the vector
path; any thumbnail fallback fails the test.
