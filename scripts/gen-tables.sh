#!/usr/bin/env bash
# Generate (or verify) the static grammar tables under
# crates/rexi-tests/src/generated with rexi-schema-gen.
#
#   scripts/gen-tables.sh          regenerate in place
#   scripts/gen-tables.sh --check  exit 1 if any generated file is stale
#
# Inputs:  fixtures/interop/cases.tsv (one module per case),
#          fixtures/spec/exi-options.xsd (module exi_options)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TESTS="$ROOT/crates/rexi-tests"
FIXTURES="$TESTS/fixtures/interop"
OUT="$TESTS/src/generated"

CHECK=()
case "${1:-}" in
    --check) CHECK=(--check) ;;
    "") ;;
    *) echo "usage: $0 [--check]" >&2; exit 2 ;;
esac

cargo build -q --manifest-path "$ROOT/Cargo.toml" -p rexi-schema-gen
GEN="$ROOT/target/debug/rexi-schema-gen"

gen() { # <module> <xsd...>
    local module="$1"; shift
    "$GEN" "$@" --out "$OUT/$module.rs" "${CHECK[@]}"
}

status=0
while IFS=$'\t' read -r name _xml main imports; do
    [[ "$name" == name || -z "$name" ]] && continue
    xsds=("$FIXTURES/$main")
    IFS=',' read -ra extra <<< "${imports:-}"
    for x in "${extra[@]}"; do [[ -n "$x" ]] && xsds+=("$FIXTURES/$x"); done
    gen "${name//-/_}" "${xsds[@]}" || status=1
done < "$FIXTURES/cases.tsv"
gen exi_options "$TESTS/fixtures/spec/exi-options.xsd" --name EXI_OPTIONS || status=1
exit $status
