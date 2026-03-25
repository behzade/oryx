# Oryx

Native Rust music player built with `gpui`.

## Platform support

The packaged app in this repo currently targets macOS.

## Common prerequisites

- Rust toolchain from `rustup`
- `cargo packager` installed locally: `cargo install cargo-packager --locked`
- `ffmpeg` and `ffprobe` on `PATH` if you want import normalization and media probing to work

## Development builds

```bash
cargo check
```

## Packaging

Build the macOS release package:

```bash
cargo packager --release
```

This produces:

- macOS: `.dmg`

Output artifacts are written under `target/release`.
