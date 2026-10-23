#!/usr/bin/env bash
# Runs each example in Rust and in Julia (jexi/examples/basics) and checks that
# both print exactly the same thing. Timings go to stderr, so only stdout is
# compared.
#   ./compare.sh path/to/jexi     or set $JEXI to the jexi checkout
set -euo pipefail
JEXI="${1:-${JEXI:-}}"
if [[ -z "$JEXI" || ! -d "$JEXI/examples/basics" ]]; then
    [[ -n "$JEXI" ]] && echo "not a jexi checkout: $JEXI" >&2
    echo "usage: $0 path/to/jexi   (or set JEXI=path/to/jexi)" >&2
    echo "Pass the path to a jexi checkout, or get one first:" >&2
    echo "  git clone https://github.com/nhenehan/jexi" >&2
    exit 2
fi
JEXI="$(cd "$JEXI" && pwd)"
cd "$(dirname "${BASH_SOURCE[0]}")"
PAIRED=(roundtrip options read_events write_events structs prebuilt_tables)
cargo build --quiet --bins
(cd "$JEXI" && julia --startup-file=no --project=examples/basics -e 'using Pkg; Pkg.develop(path=".")') >/dev/null 2>&1
OUT="$(mktemp -d)"
trap 'rm -rf "$OUT"' EXIT
failed=0
for name in "${PAIRED[@]}"; do
    "target/debug/$name" >"$OUT/$name.rs" 2>/dev/null
    julia --startup-file=no --project="$JEXI/examples/basics" "$JEXI/examples/basics/$name.jl" \
        >"$OUT/$name.jl" 2>/dev/null
    if diff -u "$OUT/$name.rs" "$OUT/$name.jl"; then
        echo "same: $name"
    else
        echo "DIFFERENT: $name"
        failed=1
    fi
done
exit $failed
