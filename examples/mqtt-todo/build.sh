#!/usr/bin/env bash
# Builds the MQTT todo example: the server and every client whose toolchain is
# here. Nothing in it is part of the normal rexi build.
#
#   ./build.sh
#
# Writes into build/ (C and C# clients) and target/ (Rust), java-client/target/
# (Java), and lists the clients it built in build/clients.txt for demo.sh.
#
# Needs: cargo (always). Optional: cmake + a C compiler + git (C client),
# java 25 + mvn (Java client), the .NET 10 SDK + svn (C# client). The .NET SDK
# is found on PATH, in $DOTNET_ROOT or in ~/.dotnet.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REXI="$(cd "$HERE/../.." && pwd)"
BUILD="$HERE/build"
mkdir -p "$BUILD"
built=()
skipped=()
have() { command -v "$1" >/dev/null; }

echo "== Rust: server and client"
cargo build -q --release --manifest-path "$HERE/Cargo.toml"
built+=(rust)

TP=""
if have git || have svn; then
    "$HERE/scripts/fetch-third-party.sh" || echo "warning: some third parties could not be fetched" >&2
    TP="$("$HERE/scripts/fetch-third-party.sh" --print-dir)"
fi

echo "== C: Paho MQTT C + rexi C API"
if have cmake && have cc && [[ -n "$TP" && -f "$TP/paho.mqtt.c/src/MQTTClient.h" ]]; then
    cargo build -q --release --manifest-path "$REXI/Cargo.toml" -p rexi-capi --lib
    cmake -S "$HERE/c-client" -B "$BUILD/c-client" -DPAHO_DIR="$TP/paho.mqtt.c" >/dev/null
    cmake --build "$BUILD/c-client" -j >/dev/null
    built+=(c)
else
    skipped+=("c (needs cmake, cc and the Paho C checkout)")
fi

echo "== Java: Paho + EXIficient"
if have java && have mvn; then
    mvn -q -f "$HERE/java-client/pom.xml" package 2> >(grep -v '^WARNING' >&2)
    built+=(java)
else
    skipped+=("java (needs java and mvn)")
fi

echo "== C#: MQTTnet + Nagasena for .NET"
DOTNET="$(command -v dotnet || true)"
for d in "${DOTNET_ROOT:-}" "$HOME/.dotnet"; do
    [[ -z "$DOTNET" && -n "$d" && -x "$d/dotnet" ]] && DOTNET="$d/dotnet"
done
if [[ -n "$DOTNET" && -n "$TP" && -d "$TP/nagasenacs/src/Nagasena" ]]; then
    DOTNET_CLI_TELEMETRY_OPTOUT=1 DOTNET_NOLOGO=1 "$DOTNET" build -c Release --nologo -v q \
        "$HERE/csharp-client/todo-csharp-client.csproj" \
        -p:NagasenaCsDir="$TP/nagasenacs" -o "$BUILD/csharp-client" >/dev/null
    built+=(csharp)
else
    skipped+=("csharp (needs the .NET SDK and the nagasenacs checkout)")
fi

printf '%s\n' "${built[@]}" >"$BUILD/clients.txt"
echo "built: ${built[*]}"
for s in "${skipped[@]}"; do
    echo "skipped: $s"
done
