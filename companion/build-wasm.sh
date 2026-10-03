#!/usr/bin/env bash
# Builds the library builder for the browser companion.
#
# Needs the wasm target once:
#   rustup target add wasm32-unknown-unknown
set -euo pipefail
cd "$(dirname "$0")"

TARGET=wasm32-unknown-unknown
cargo build --release --target "$TARGET" --no-default-features --features wasm
OUT=target/$TARGET/release/ipdb.wasm
cp "$OUT" web/ipdb.wasm
printf 'wrote web/ipdb.wasm (%s bytes)\n' "$(wc -c < web/ipdb.wasm)"
