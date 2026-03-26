# Oryx

Native Rust music player built with `gpui`.

Oryx supports a local library plus configurable remote providers. Remote providers are defined by user-supplied TOML manifests and loaded through a generic runtime rather than provider-specific application code.

## License and Distribution

The source code in this repository is source-available under [PolyForm Strict 1.0.0](./LICENSE).

- official builds are published only by the maintainer
- public issue reports are welcome
- pull requests and public code contributions are not being accepted
- the `Oryx` name, logos, icons, and other brand assets are not licensed under the software license

See [TRADEMARKS.md](./TRADEMARKS.md) and [CONTRIBUTING.md](./CONTRIBUTING.md) for repository policy details.

## Current Status

- Primary packaging target: macOS
- Local library import and playback are built in
- Remote discovery/playback depends on provider manifests installed outside the repo

## Provider Configuration

Provider manifests are loaded from:

- default: `~/.config/oryx/providers/<id>.toml`
- override: `ORYX_PROVIDER_DIR`

Oryx can also read optional bundled provider directories with lower precedence:

- `bundled/providers/` in the current working directory
- `bundled/providers/` next to the executable
- `ORYX_BUNDLED_PROVIDER_DIR`

Provider manifests are treated as executable configuration:

- changed manifests are validated before activation
- the last validated config remains active if a new manifest fails validation
- cached audio remains playable even if a provider config is missing or invalid

Provider config import/export is supported from the app menu using a compact provider link format or raw TOML.

This repository does not ship provider manifests.

See [docs/provider-config.md](./docs/provider-config.md) for the manifest format.

## Prerequisites

- Rust toolchain from `rustup`
- `cargo packager` installed locally: `cargo install cargo-packager --locked`
- `ffmpeg` and `ffprobe` on `PATH` for import normalization and media probing
- `yt-dlp` available at runtime for the `Open Media...` flow to resolve downloadable media URLs

## Development

Check the project:

```bash
cargo check
```

Run tests:

```bash
cargo test
```

## Open Media

`Open Media...` resolves media URLs with `yt-dlp`, downloads them into `~/Downloads`, and opens completed files with the operating system's default app for that media type.

Oryx is not bound to `mpv` here. The external opener is intentionally generic, and a user-configurable opener can be added later.

## Packaging

Build the macOS release package:

```bash
cargo packager --release
```

Artifacts are written under `target/release`.
