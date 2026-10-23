#!/usr/bin/env bash
# Regenerates csharp-client/todo.xsc from todo.xsd with Java Nagasena, for
# Nagasena for .NET (which cannot compile schemas itself).
#
#   scripts/gen-xsc.sh           regenerate
#   scripts/gen-xsc.sh --check   exit 1 if the committed todo.xsc is stale
#
# Needs Java 25 and the Java Nagasena jars (nagasena.jar, nagasena-rta.jar)
# with Xerces 2.11, which Nagasena's schema compiler uses. By default they come
# from the W3C exi-testsuite entry in ../third-party.lock, which
# scripts/fetch-third-party.sh checks out (only candidates/openexi/lib).
# $NAGASENA_LIB (a folder holding nagasena.jar and nagasena-rta.jar) and
# $XERCES_JAR override them.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TS_LIB="$("$ROOT/scripts/fetch-third-party.sh" --print-dir)/exi-testsuite/candidates/openexi/lib"
LIB="${NAGASENA_LIB:-$TS_LIB}"
JARS="$LIB/nagasena.jar:$LIB/nagasena-rta.jar"
JARS="$JARS:${XERCES_JAR:-$TS_LIB/apache/xerces/2.11.0/xercesImpl.jar}"
for j in ${JARS//:/ }; do
    [[ -f "$j" ]] || {
        echo "missing $j" >&2
        echo "run scripts/fetch-third-party.sh first, or see the comment at the top of $0" >&2
        exit 1
    }
done

OUT="$ROOT/csharp-client/todo.xsc"
TMP="$(mktemp)"
trap 'rm -f "$TMP"' EXIT
java -cp "$JARS" "$ROOT/scripts/CompileXsc.java" "$ROOT/todo.xsd" "$TMP"
if [[ "${1:-}" == "--check" ]]; then
    cmp -s "$TMP" "$OUT" || { echo "csharp-client/todo.xsc is stale: run scripts/gen-xsc.sh" >&2; exit 1; }
    echo "todo.xsc is up to date"
else
    mkdir -p "$(dirname "$OUT")"
    cp "$TMP" "$OUT"
    echo "wrote $OUT"
fi
