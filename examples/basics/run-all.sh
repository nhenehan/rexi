#!/usr/bin/env bash
# Builds and runs every example, in the order the guide uses.
#   ./run-all.sh        run them all
#   ./run-all.sh --c    also build and run the C API example (needs cmake)
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
EXAMPLES=(roundtrip options read_events write_events structs prebuilt_tables fixed_buffer)
cargo build --quiet --bins
for name in "${EXAMPLES[@]}"; do
    echo "=== $name"
    "target/debug/$name"
done
if [[ "${1:-}" == "--c" ]]; then
    echo "=== c/order"
    (cd ../.. && cargo build --quiet --release -p rexi-capi)
    cmake -S c -B c/build >/dev/null
    cmake --build c/build >/dev/null
    c/build/order data/order.xsd
fi
