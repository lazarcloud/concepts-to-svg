# Contributing

Issues and pull requests are welcome. Run `cargo test --locked` before submitting
code changes.

## Conversion problems

If a `.concepts` file fails to convert or the SVG looks wrong, please open an
issue and provide the problematic `.concepts` file so the format and issue can
be reverse engineered. Include:

- The Concepts platform (Windows or Android) and the CLI version and platform.
- The command you ran, any error message, and what you expected to see.
- If possible, the resulting SVG and a screenshot or reference export showing
  the expected appearance.

Please only share files you have permission to distribute. GitHub issues and
attachments are public: remove sensitive or private content, or create a small
reproducible drawing that demonstrates the problem. Don't post a private file
unless you are comfortable making it public.

## Publishing a release

Update the version in `Cargo.toml` and `Cargo.lock`, commit the changes, then
push a matching tag, for example:

```sh
git tag v0.1.0
git push origin v0.1.0
```

The release workflow tests and builds on Windows and Linux and publishes the
archives to a GitHub Release. The repository's Actions settings must allow the
workflow to write contents.
