# Repository Guide

Oryx is a Rust music player built with `gpui`. App and UI code lives in `src/app/`, audio code in `src/audio/`, library code in `src/library/`, metadata code in `src/metadata/`, providers in `src/provider/`, and platform code in `src/platform/`. Assets live in `assets/`.

Use the nightly toolchain in `rust-toolchain.toml`. Run `cargo fmt`, `cargo check`, and `cargo test` for code changes. Runtime tools are `ffmpeg`, `ffprobe`, and `yt-dlp`.

Follow standard Rust style. Keep code in its domain module and keep functions small. Put focused tests next to the code.

Use short commit subjects such as `fix: cache updates`. Do not open a pull request unless the maintainer asks for one.
