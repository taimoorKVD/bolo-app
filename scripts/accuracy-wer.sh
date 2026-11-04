#!/bin/sh
# Text-level word error rate for the release phrase set. No API key, no audio.
set -e
cd "$(dirname "$0")/.."
cargo test --manifest-path src-tauri/Cargo.toml --lib regression::release_phrases -- --nocapture
