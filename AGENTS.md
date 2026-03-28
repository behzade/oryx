# Repository Guidelines

## Project Structure & Module Organization
`oryx` is a native Rust music player built with `gpui`. Core code lives in `src/`: UI and app flow under `src/app/`, audio playback under `src/audio/`, library state under `src/library/`, metadata ingestion under `src/metadata/`, provider loading under `src/provider/`, and platform-specific code under `src/platform/`. Static assets and icons live in `assets/`. Supporting docs are in `docs/`, and release/bootstrap scripts are in `scripts/`.

## Build, Test, and Development Commands
- `cargo check`: fast compile pass for day-to-day validation.
- `cargo test`: runs unit tests across the crate.
- `cargo run`: starts the desktop app with the current profile.
- `cargo build --release --locked`: produces a release binary matching packaging inputs.
- `cargo packager --release --formats <dmg|appimage|deb|nsis>`: builds installable artifacts.
- `./scripts/release.sh <major|minor|patch>`: bumps versioning and prepares a release tag workflow.

Use the nightly Rust toolchain declared in `rust-toolchain.toml`. Local development also expects `ffmpeg`, `ffprobe`, and `yt-dlp` on `PATH`.

## Coding Style & Naming Conventions
Follow standard Rust formatting: four-space indentation, trailing commas where rustfmt expects them, `snake_case` for modules/functions, `PascalCase` for types, and compact `mod` trees. Keep modules focused by domain; new provider logic should live under `src/provider/`, not in UI code. Prefer small helpers over deeply nested UI handlers.

## Testing Guidelines
Tests are colocated with implementation. Use inline `mod tests` blocks for focused unit coverage, or adjacent files such as `src/app_tests.rs` and `src/app/library/actions_tests.rs` when test code is large. Name tests by behavior, for example `imports_album_artwork_when_metadata_is_present`. Run `cargo test` before submitting any change.

## Commit & Pull Request Guidelines
Recent history follows short conventional subjects such as `fix: cache updates`, `feat: better quality display`, and `release: v0.1.6`. Keep commits scoped and imperative. This repository is source-available, not open collaboration: do not open unsolicited pull requests unless the maintainer explicitly asks for one. Issues should include platform details, reproduction steps, and logs or screenshots when relevant.
