# concepts-to-svg

A command-line tool that converts Concepts `.concepts` drawings to SVG. It targets
files from Concepts on Windows and Android. macOS files have not been tested.

## Download and use

Download the Windows or Linux x86-64 archive from [GitHub Releases](../../releases),
extract it, and run the executable from a terminal:

```sh
concepts-to-svg drawing.concepts
concepts-to-svg drawing.concepts --output drawing.svg
concepts-to-svg drawing.concepts --require-vector
```

On Windows, use `concepts-to-svg.exe` (or `./concepts-to-svg.exe` in
PowerShell). The default output is next to the input; an existing default output
is not overwritten. An explicitly specified output may be overwritten.

Unsupported drawing content falls back to an embedded thumbnail, which is not
vector artwork. Use `--require-vector` to fail instead. Some brushes and text
are approximated, and imported PDF pages are not drawn.

## Build from source

Install a current stable [Rust toolchain](https://rustup.rs/), then run:

```sh
cargo build --release --locked
cargo test --locked
```

The executable is in `target/release/`.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md), especially if a drawing fails to convert.

## License

This project is source-available under the [PolyForm Noncommercial License
1.0.0](LICENSE). It permits noncommercial use under its terms and is provided
as is, without warranty. It is not an open-source license because it restricts
commercial use.
