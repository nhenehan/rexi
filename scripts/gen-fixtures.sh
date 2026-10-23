#!/usr/bin/env bash
# Generate (or verify) the EXIficient oracle fixtures under
# crates/rexi-tests/fixtures/interop with the EXIficient oracle of the pinned exip
# (utils/xsd2exip, EXIficient 1.0.7 from Maven Central; scripts/fetch-third-party.sh
# checks it out).
#
#   scripts/gen-fixtures.sh            regenerate fixtures in place
#   scripts/gen-fixtures.sh --force    ... also overwrite changed *.exi files
#   scripts/gen-fixtures.sh --check    regenerate into a temp dir and diff
#                                      against the committed files (exit 1 on
#                                      any difference)
#
# Inputs:  fixtures/interop/cases.tsv  (name, xml, main_xsd, import_xsds)
# Outputs: <name>.<profile>.exi, <name>.<profile>.events, <name>.grammars.txt,
#          <xsd>.exi (schema documents encoded for exipd -schema=), manifest.tsv
#
# Env:     REXI_ORACLE_JAR  use this oracle jar instead of building
#          <pinned exip>/utils/xsd2exip/target/xsd2exip-cli.jar (rebuilt with Maven when its
#          sources are newer than the jar)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FIXTURES="$ROOT/crates/rexi-tests/fixtures/interop"
CASES="$FIXTURES/cases.tsv"
ORACLE_DIR="$("$ROOT/scripts/fetch-third-party.sh" --print-dir)/exip/utils/xsd2exip"
JAR="${REXI_ORACLE_JAR:-$ORACLE_DIR/target/xsd2exip-cli.jar}"

MODE=generate
FORCE=0
for arg in "$@"; do
    case "$arg" in
        --check) MODE=check ;;
        --force) FORCE=1 ;;
        -h|--help)
            sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            echo "unknown argument: $arg" >&2
            exit 2
            ;;
    esac
done

# Profiles: name|flags.  "@NAME@" is replaced by the case name (schemaId).
PROFILES=(
    "strict-bp|--strict --alignment bit-packed --no-options --no-cookie"
    "strict-bp-opts|--strict --alignment bit-packed --include-options --no-cookie --schema-id @NAME@"
    "strict-ba|--strict --alignment byte-aligned --no-options --no-cookie"
    "default-bp|--no-strict --alignment bit-packed --no-options --no-cookie"
)

require_command() {
    command -v "$1" >/dev/null 2>&1 || { echo "missing required command: $1" >&2; exit 1; }
}
require_command java
require_command sha256sum
require_command cmp

if [[ -z "${REXI_ORACLE_JAR:-}" ]] &&
    { [[ ! -f "$JAR" ]] || [[ -n "$(find "$ORACLE_DIR/src" "$ORACLE_DIR/pom.xml" -newer "$JAR" -print -quit)" ]]; }; then
    echo "building the oracle jar with maven ($ORACLE_DIR)" >&2
    require_command mvn
    (cd "$ORACLE_DIR" && mvn -q -DskipTests clean package)
fi
[[ -f "$JAR" ]] || { echo "oracle jar not found at $JAR" >&2; exit 1; }

[[ -f "$CASES" ]] || { echo "missing $CASES" >&2; exit 1; }

# Run the oracle tool; SLF4J "no provider" chatter is dropped, real errors kept.
tool() {
    local err rc=0
    err="$(mktemp)"
    java -jar "$JAR" "$@" 2>"$err" || rc=$?
    if [[ $rc -ne 0 ]]; then
        echo "oracle failed (exit $rc): $*" >&2
        grep -v '^SLF4J' "$err" >&2 || true
    fi
    rm -f "$err"
    return $rc
}

# EXIficient artifact version actually bundled in the jar (fallback 1.0.7).
EXIFICIENT_VERSION="$(unzip -p "$JAR" META-INF/maven/com.siemens.ct.exi/exificient-core/pom.properties 2>/dev/null \
    | awk -F= '$1=="version"{print $2}' | tr -d '\r' || true)"
EXIFICIENT_VERSION="${EXIFICIENT_VERSION:-1.0.7}"

# Hash of the oracle's sources and pom: a change to the oracle shows up in
# --check as a manifest difference even when the bytes do not change.
ORACLE_SRC_SHA="$(cd "$ORACLE_DIR" && find src pom.xml -type f | LC_ALL=C sort |
    xargs sha256sum | sha256sum | cut -c1-64)"

GENERATED_UTC="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
OUT="$WORK/out"
mkdir -p "$OUT"

# manifest rows: name profile file sha256 bytes (timestamp etc. appended later)
ROWS="$WORK/rows.tsv"
: >"$ROWS"

add_row() {
    local name="$1" profile="$2" file="$3"
    local sha bytes
    sha="$(sha256sum "$OUT/$file" | awk '{print $1}')"
    bytes="$(stat -c %s "$OUT/$file")"
    printf '%s\t%s\t%s\t%s\t%s\n' "$name" "$profile" "$file" "$sha" "$bytes" >>"$ROWS"
}

declare -A SCHEMA_EXI_DONE=()

encode_schema_exi() {
    local xsd="$1"
    [[ -n "${SCHEMA_EXI_DONE[$xsd]:-}" ]] && return 0
    [[ -f "$FIXTURES/$xsd" ]] || { echo "missing schema $FIXTURES/$xsd" >&2; exit 1; }
    tool encode-xsd --xsd "$FIXTURES/$xsd" --xsd-exi-out "$OUT/$xsd.exi"
    add_row "$xsd" "schema-exi" "$xsd.exi"
    SCHEMA_EXI_DONE[$xsd]=1
}

echo "generating fixtures with $JAR (exificient $EXIFICIENT_VERSION)" >&2

# cases.tsv: name xml main_xsd import_xsds (import_xsds comma-separated, may be empty)
while IFS=$'\t' read -r name xml main_xsd import_xsds || [[ -n "${name:-}" ]]; do
    [[ -z "${name:-}" || "$name" == "name" || "$name" == \#* ]] && continue
    import_xsds="${import_xsds:-}"
    [[ -f "$FIXTURES/$xml" ]] || { echo "missing $FIXTURES/$xml" >&2; exit 1; }
    [[ -f "$FIXTURES/$main_xsd" ]] || { echo "missing $FIXTURES/$main_xsd" >&2; exit 1; }

    echo "  case $name" >&2
    for entry in "${PROFILES[@]}"; do
        profile="${entry%%|*}"
        flags="${entry#*|}"
        flags="${flags//@NAME@/$name}"
        # shellcheck disable=SC2086
        tool encode-xml --xsd "$FIXTURES/$main_xsd" --xml "$FIXTURES/$xml" \
            --out "$OUT/$name.$profile.exi" $flags
        add_row "$name" "$profile" "$name.$profile.exi"
        # shellcheck disable=SC2086
        tool dump-events --xsd "$FIXTURES/$main_xsd" --exi "$OUT/$name.$profile.exi" $flags \
            >"$OUT/$name.$profile.events"
        add_row "$name" "$profile.events" "$name.$profile.events"
    done

    tool dump-grammars --xsd "$FIXTURES/$main_xsd" --out "$OUT/$name.grammars.txt"
    add_row "$name" "grammars" "$name.grammars.txt"

    encode_schema_exi "$main_xsd"
    IFS=',' read -r -a imports <<<"$import_xsds"
    for imp in "${imports[@]}"; do
        imp="${imp// /}"
        [[ -n "$imp" ]] && encode_schema_exi "$imp"
    done
done <"$CASES"

# Deterministic manifest (sorted by name, profile, file).
MANIFEST="$OUT/manifest.tsv"
{
    printf 'name\tprofile\tfile\tsha256\tbytes\texificient_version\toracle_src_sha256\tgenerated_utc\n'
    LC_ALL=C sort -t $'\t' -k1,1 -k2,2 -k3,3 "$ROWS" | while IFS=$'\t' read -r n p f s b; do
        printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$n" "$p" "$f" "$s" "$b" \
            "$EXIFICIENT_VERSION" "$ORACLE_SRC_SHA" "$GENERATED_UTC"
    done
} >"$MANIFEST"

# Column 8 (generated_utc) is excluded when comparing manifests.
manifest_stable() { cut -f1-7 "$1"; }

if [[ "$MODE" == "check" ]]; then
    status=0
    while IFS= read -r f; do
        rel="${f#"$OUT/"}"
        if [[ ! -f "$FIXTURES/$rel" ]]; then
            echo "MISSING  $rel" >&2
            status=1
            continue
        fi
        if [[ "$rel" == "manifest.tsv" ]]; then
            if ! diff -u <(manifest_stable "$FIXTURES/$rel") <(manifest_stable "$f") >&2; then
                echo "DIFFERS  $rel" >&2
                status=1
            fi
        elif ! cmp -s "$FIXTURES/$rel" "$f"; then
            echo "DIFFERS  $rel" >&2
            if [[ "$rel" == *.events || "$rel" == *.grammars.txt ]]; then
                diff -u "$FIXTURES/$rel" "$f" >&2 || true
            else
                echo "  committed: $(sha256sum "$FIXTURES/$rel" | awk '{print $1}')" >&2
                echo "  generated: $(sha256sum "$f" | awk '{print $1}')" >&2
            fi
            status=1
        fi
    done < <(find "$OUT" -type f | LC_ALL=C sort)
    if [[ $status -eq 0 ]]; then
        echo "fixtures up to date ($(find "$OUT" -type f | wc -l) files)" >&2
    else
        echo "fixture check FAILED" >&2
    fi
    exit $status
fi

# generate mode: refuse to overwrite a changed .exi unless --force
changed=()
while IFS= read -r f; do
    rel="${f#"$OUT/"}"
    if [[ "$rel" == *.exi && -f "$FIXTURES/$rel" ]] && ! cmp -s "$FIXTURES/$rel" "$f"; then
        changed+=("$rel")
    fi
done < <(find "$OUT" -type f | LC_ALL=C sort)

if [[ ${#changed[@]} -gt 0 && $FORCE -eq 0 ]]; then
    echo "refusing to overwrite changed .exi fixtures (use --force):" >&2
    printf '  %s\n' "${changed[@]}" >&2
    exit 1
fi

while IFS= read -r f; do
    rel="${f#"$OUT/"}"
    cp "$f" "$FIXTURES/$rel"
done < <(find "$OUT" -type f | LC_ALL=C sort)

echo "wrote $(find "$OUT" -type f | wc -l) files to $FIXTURES" >&2
