#!/usr/bin/env bash
# Runs the MQTT todo example end to end and checks that every client sees the
# others' changes.
#
#   ./demo.sh [--no-build]
#
# Starts todo-server on a free port, then, with every client that build.sh
# built (build/clients.txt):
#   1. each client adds "<client> item" (ids 1, 2, ... in client order);
#   2. each client marks the next client's item done;
#   3. the last client removes item 1;
#   4. each client lists, and all lists must equal the expected one.
# Exits non-zero on any mismatch.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
[[ "${1:-}" == "--no-build" ]] || "$HERE/build.sh"

DOTNET="$(command -v dotnet || true)"
for d in "${DOTNET_ROOT:-}" "$HOME/.dotnet"; do
    [[ -z "$DOTNET" && -n "$d" && -x "$d/dotnet" ]] && DOTNET="$d/dotnet"
done
# The C# client's launcher finds the .NET runtime through DOTNET_ROOT.
[[ -n "$DOTNET" ]] && export DOTNET_ROOT="${DOTNET_ROOT:-$(dirname "$(readlink -f "$DOTNET")")}"

mapfile -t CLIENTS <"$HERE/build/clients.txt"
PORT="$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')"

client() { # name args...
    local name="$1"
    shift
    case "$name" in
        rust) "$HERE/target/release/todo-rust-client" --port "$PORT" "$@" ;;
        java) java -jar "$HERE/java-client/target/todo-java-client.jar" --port "$PORT" "$@" ;;
        c) "$HERE/build/c-client/todo-c-client" --port "$PORT" "$@" ;;
        csharp) "$HERE/build/csharp-client/todo-csharp-client" --port "$PORT" "$@" ;;
        *) echo "unknown client $name" >&2; return 1 ;;
    esac
}

LOG="$(mktemp)"
"$HERE/target/release/todo-server" --port "$PORT" >"$LOG" 2>&1 &
SERVER=$!
trap 'kill "$SERVER" 2>/dev/null || true; rm -f "$LOG"' EXIT
for _ in $(seq 50); do
    grep -q 'listening' "$LOG" && break
    kill -0 "$SERVER" 2>/dev/null || { cat "$LOG" >&2; exit 1; }
    sleep 0.2
done
grep -q 'listening' "$LOG" || { echo "server did not start" >&2; cat "$LOG" >&2; exit 1; }
echo "server on port $PORT; clients: ${CLIENTS[*]}"

fail=0
expect_in() { # output needle what
    if ! grep -qF -- "$2" <<<"$1"; then
        echo "MISMATCH: $3: expected a line with '$2' in:" >&2
        printf '%s\n' "$1" >&2
        fail=1
    fi
}

n=${#CLIENTS[@]}
for ((i = 0; i < n; i++)); do
    c="${CLIENTS[i]}"
    out="$(client "$c" add "$c item")"
    expect_in "$out" "[ ] $((i + 1)) $c item" "$c add"
    echo "$c add: ok"
done
for ((i = 0; i < n; i++)); do
    c="${CLIENTS[i]}"
    next=$(((i + 1) % n))
    out="$(client "$c" "done" $((next + 1)))"
    expect_in "$out" "[x] $((next + 1)) ${CLIENTS[next]} item" "$c done"
    echo "$c done $((next + 1)): ok"
done
last="${CLIENTS[n - 1]}"
out="$(client "$last" remove 1)"
if grep -qE '^\[.\] 1 ' <<<"$out"; then
    echo "MISMATCH: $last remove 1: item 1 is still listed" >&2
    fail=1
fi
echo "$last remove 1: ok"

expected="revision $((2 * n + 1))"
for ((i = 1; i < n; i++)); do
    expected+=$'\n'"[x] $((i + 1)) ${CLIENTS[i]} item"
done
for c in "${CLIENTS[@]}"; do
    out="$(client "$c" list)"
    if [[ "$out" != "$expected" ]]; then
        echo "MISMATCH: $c list:" >&2
        diff <(printf '%s\n' "$expected") <(printf '%s\n' "$out") >&2 || true
        fail=1
    else
        echo "$c list: ok"
    fi
done
echo
echo "final list:"
printf '%s\n' "$expected"
if [[ $fail -ne 0 ]]; then
    echo "demo FAILED" >&2
    exit 1
fi
echo "demo passed with ${CLIENTS[*]}"
