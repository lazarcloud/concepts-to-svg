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

Run tests with `cargo test`. When paired `.concepts`/`.svg` references are
present in the project root, an integration test compares layer/object IDs,
styles, and sampled path geometry (within 1 drawing unit); it also verifies
that every paired input converted through the vector path. Any thumbnail
fallback fails the comparison test. The test is skipped when no paired files
are available.
