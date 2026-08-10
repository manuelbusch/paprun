#!/usr/bin/env bash
# Baut das WebAssembly-Paket und legt es unter web/pkg ab.
#
# Voraussetzungen:
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-bindgen-cli   (Version muss zur Crate-Version passen)
set -euo pipefail

cd "$(dirname "$0")/.."

cargo build --release --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir web/pkg --no-typescript \
    target/wasm32-unknown-unknown/release/paprun.wasm

# Die Demo lädt die Programmablaufpläne zur Laufzeit.
cp tests/data/Lohnsteuer2025.xml tests/data/Lohnsteuer2026.xml web/

echo
echo "Fertig. Zum Ausprobieren:"
echo "  python3 -m http.server --directory web 8000"
echo "  http://localhost:8000/"
