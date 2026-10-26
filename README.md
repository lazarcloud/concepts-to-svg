# concepts-to-svg

A Rust CLI that converts Concepts `.concepts` archives into SVG files.

```sh
cargo run -- drawing.concepts
cargo run -- drawing.concepts --output drawing-preview.svg
cargo run -- drawing.concepts --output drawing.svg --require-vector
```

The exporter decodes the current `tree.pack` MessagePack records and writes
stroke paths, dots, layers, colors, opacity, embedded PNG/JPEG images, and a
solid background as SVG. It supports planar projective transforms on strokes,
affine transforms on images, and empty drawings as background-only vector SVGs.
Common watercolor, airbrush, marker, pencil, fill, and RGBA-less eraser records
are emitted as SVG paths. Text records are editable, center-aligned SVG text;
projective text transforms are approximated by their affine components because
SVG text cannot represent perspective. Imported PDF pages are not drawn (matching
Concepts' SVG export), but their page bounds contribute to the output crop. The
Concepts-specific pencil texture is approximated as smooth ink. Unknown or
unsupported records fall back to the embedded `thumb.jpg`, preserving the
preview at thumbnail resolution rather than silently dropping content. Curves
are reconstructed from stored point samples, so some brush paths may differ from
Concepts' own SVG exporter. The CLI refuses to overwrite the default `.svg`
output if it already exists; pass `--output` to choose an explicit path.

Run tests with `cargo test`. The paired references in `tests/fixtures` are used
by an integration test that compares layer/object IDs, styles, brush filters,
embedded image data and transforms, and sampled path geometry. Most fixtures
use a 1-unit geometry tolerance. Testfile 6 allows 30 units for brush-specific
smoothing; Testfiles 7 and 9 allow 35 units for text-drawing stroke resampling;
Testfile 10 allows 2 units for eraser paths. The PDF/text Testfile 8 allows a
30% viewBox-ratio difference because its centered text layout is approximate.
Every fixture must convert through the vector path; any thumbnail fallback
fails the test.

Pass `--require-vector` to make the CLI return an error instead of writing an
SVG that uses the thumbnail fallback. This is useful when vector output is a
hard requirement; some real-world drawings contain object types not yet
supported by the vector decoder.
