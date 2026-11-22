#!/usr/bin/env bash
# Everything a commit must pass: formatting, clippy (no warnings), rustdoc
# (every public item documented), tests (std, no_std, and the oldest supported
# Rust), the Cortex-M build, generated tables and EXIficient fixtures up to date.
#
#   scripts/gate.sh           full gate
#   scripts/gate.sh --quick   skip the EXIficient fixture check (needs Java)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
QUICK=0
[[ "${1:-}" == "--quick" ]] && QUICK=1
NOSTD=(-p rexi-core -p rexi-serde -p rexi-nostd-check -p rexi)
# Optional library features (off by default), each tested on its own and then
# all together.
FEATURE_RUNS=(
    rexi-xml/wildcard
    rexi-xml/lexical-values
    rexi-xml/wildcard,rexi-xml/lexical-values
)

step() { printf '\n== %s\n' "$*"; }

step "standards (W3C EXI 1.0, fetched, not committed)"
scripts/fetch-standards.sh --check

step "rustfmt"
cargo fmt --all --check

step "clippy"
cargo clippy --workspace --all-targets --no-deps -- -D warnings
cargo clippy "${NOSTD[@]}" --all-targets --no-deps --no-default-features -- -D warnings
for f in "${FEATURE_RUNS[@]}"; do
    cargo clippy --workspace --all-targets --no-deps --features "$f" -- -D warnings
done

step "rustdoc (no warnings, every public item documented)"
RUSTDOCFLAGS="-D warnings" cargo doc -q --workspace --no-deps

step "tests"
cargo test --workspace -q
cargo test "${NOSTD[@]}" -q --no-default-features
for f in "${FEATURE_RUNS[@]}"; do
    cargo test --workspace -q --features "$f"
done

step "tests on the oldest supported Rust (1.88)"
# Own target directory, so the two compilers do not rebuild each other's files.
CARGO_TARGET_DIR=target/msrv cargo +1.88 test --workspace -q \
    || { echo "needs: rustup toolchain install 1.88" >&2; exit 1; }

step "no_std build (thumbv7em-none-eabihf)"
cargo build -q -p rexi-nostd-check --target thumbv7em-none-eabihf
for f in rexi-core/wildcard rexi-core/lexical-values rexi-core/wildcard,rexi-core/lexical-values; do
    cargo build -q -p rexi-nostd-check --features "$f" --target thumbv7em-none-eabihf
done
cargo build -q -p rexi --no-default-features --features serde --target thumbv7em-none-eabihf

step "generated tables"
scripts/gen-tables.sh --check

if [[ $QUICK -eq 0 ]]; then
    step "EXIficient fixtures"
    scripts/gen-fixtures.sh --check
fi

printf '\ngate passed\n'
