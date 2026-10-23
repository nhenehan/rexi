#!/usr/bin/env bash
# Runs every fuzz target for a fixed time (release checklist, step 5).
# Needs a nightly toolchain and cargo-fuzz (cargo install cargo-fuzz).
#
#   fuzz/run.sh            60 seconds per target
#   fuzz/run.sh 600        10 minutes per target
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SECONDS_EACH="${1:-60}"
cd "$HERE"

cargo +nightly run -q --release --bin make_seeds -- \
    ../crates/rexi-tests/fixtures/interop corpus
for target in $(cargo +nightly fuzz list | grep -v "^make_seeds$"); do
    echo "== $target (${SECONDS_EACH}s)"
    cargo +nightly fuzz run "$target" -- -max_total_time="$SECONDS_EACH" \
        -max_len=4096 2>&1 | grep -a -E '^(#[0-9]+ +DONE|Done|==[0-9]+==|SUMMARY|artifact_prefix|Test unit written)' || true
    if compgen -G "artifacts/$target/*" >/dev/null; then
        echo "FAILED: $target found a crash, see fuzz/artifacts/$target" >&2
        exit 1
    fi
done
echo "fuzzing passed"
