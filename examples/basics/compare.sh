#!/usr/bin/env bash
# Runs each example in Rust and in Julia (EXIFormat.jl/examples/basics) and
# checks that both print exactly the same thing. Timings go to stderr, so only
# stdout is compared.
#   ./compare.sh path/to/EXIFormat.jl   or set $EXIFORMAT to the checkout
set -euo pipefail
EXIFORMAT="${1:-${EXIFORMAT:-}}"
if [[ -z "$EXIFORMAT" || ! -d "$EXIFORMAT/examples/basics" ]]; then
    [[ -n "$EXIFORMAT" ]] && echo "not an EXIFormat.jl checkout: $EXIFORMAT" >&2
    echo "usage: $0 path/to/EXIFormat.jl   (or set EXIFORMAT=path/to/EXIFormat.jl)" >&2
    echo "Pass the path to an EXIFormat.jl checkout, or get one first:" >&2
    echo "  git clone https://github.com/nhenehan/EXIFormat.jl" >&2
    exit 2
fi
EXIFORMAT="$(cd "$EXIFORMAT" && pwd)"
cd "$(dirname "${BASH_SOURCE[0]}")"
PAIRED=(roundtrip options read_events write_events structs prebuilt_tables)
cargo build --quiet --bins
(cd "$EXIFORMAT" && julia --startup-file=no --project=examples/basics -e 'using Pkg; Pkg.develop(path=".")') >/dev/null 2>&1
OUT="$(mktemp -d)"
trap 'rm -rf "$OUT"' EXIT
failed=0
for name in "${PAIRED[@]}"; do
    "target/debug/$name" >"$OUT/$name.rs" 2>/dev/null
    julia --startup-file=no --project="$EXIFORMAT/examples/basics" "$EXIFORMAT/examples/basics/$name.jl" \
        >"$OUT/$name.jl" 2>/dev/null
    if diff -u "$OUT/$name.rs" "$OUT/$name.jl"; then
        echo "same: $name"
    else
        echo "DIFFERENT: $name"
        failed=1
    fi
done
exit $failed
