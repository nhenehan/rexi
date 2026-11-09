#!/usr/bin/env bash
# Fetch the standards listed in standards.lock (RFCs, W3C Recommendations) from
# their publishers into a cache outside the working tree, and extract the parts
# the tests and generators read (such as a schema printed in an RFC). Standards
# are linked, never committed.
#
#   scripts/fetch-standards.sh              fetch what is missing, check every sha256
#   scripts/fetch-standards.sh --check      exit 1 unless every file is present
#                                           and has its sha256 (no network)
#   scripts/fetch-standards.sh --print-dir  print the cache directory
#
# Env: REXI_STANDARDS  cache directory
#      (default ${XDG_CACHE_HOME:-$HOME/.cache}/rexi/standards)
#
# standards.lock lines: <file> <sha256> <source>. The source is a URL, or a part
# of another file in the lock:
#   rfc-xsd:<file>#<section>   the XML schema printed in that RFC section (from
#                              "<?xml" to "</xs:schema>", page breaks and the
#                              3-space indent removed)
#   html-pre:<file>#<id>       the first <pre> after the element with that id,
#                              tags removed and entities decoded
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LOCK="$ROOT/standards.lock"
DIR="${REXI_STANDARDS:-${XDG_CACHE_HOME:-$HOME/.cache}/rexi/standards}"

MODE=fetch
case "${1:-}" in
    --check) MODE=check ;;
    --print-dir) echo "$DIR"; exit 0 ;;
    -h|--help) sed -n '2,21p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    "") ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
esac

[[ -f "$LOCK" ]] || { echo "missing $LOCK" >&2; exit 1; }
entries() { awk '!/^[[:space:]]*(#|$)/ { print $1, $2, $3 }' "$LOCK"; }

# The schema in an RFC section. Page breaks are the footer ("... [Page N]"),
# the form feed and the header ("RFC NNNN   title   date").
rfc_xsd() { # file section
    awk -v want="$2" '
        /\[Page [0-9]+\][[:space:]]*$/ || /^\f/ || /^RFC [0-9]+  / { next }
        {
            n = ""
            if (match($0, /^[0-9.]+  /)) { n = substr($0, 1, RLENGTH - 2); sub(/\.+$/, "", n) }
            if (n != "") {
                if (inside && index(n, want ".") != 1) exit
                if (n == want) inside = 1
            }
            if (!inside) next
            t = $0; sub(/^[[:space:]]+/, "", t)
            if (index(t, "<?xml") == 1) xsd = 1
            if (!xsd) next
            line = $0; sub(/^   /, "", line); print line
            if (t ~ /^<\/xs:schema>[[:space:]]*$/) exit
        }' "$1"
}

# The first <pre> after an id in an HTML page, as text.
html_pre() { # file id
    awk -v id="$2" '
        index($0, "id=\"" id "\"") { found = 1 }
        found && !pre && /<pre>/ { pre = 1; sub(/.*<pre>/, "") }
        pre {
            if (/<\/pre>/) { sub(/<\/pre>.*/, ""); print; exit }
            print
        }' "$1" |
        sed -e 's/<[^>]*>//g' -e 's/&lt;/</g' -e 's/&gt;/>/g' -e 's/&quot;/"/g' \
            -e "s/&apos;/'/g" -e 's/&#160;/ /g' -e 's/&nbsp;/ /g' -e 's/&amp;/\&/g'
}

make_file() { # file source
    local out="$DIR/$1" src="$2" tmp="$DIR/.$1.tmp"
    case "$src" in
        http://*|https://*)
            command -v curl >/dev/null || { echo "missing required command: curl" >&2; exit 1; }
            curl -fsSL --retry 3 -o "$tmp" "$src" ;;
        rfc-xsd:*)
            src="${src#rfc-xsd:}"; rfc_xsd "$DIR/${src%%#*}" "${src#*#}" > "$tmp" ;;
        html-pre:*)
            src="${src#html-pre:}"; html_pre "$DIR/${src%%#*}" "${src#*#}" > "$tmp" ;;
        *) echo "$1: unknown source $src" >&2; exit 1 ;;
    esac
    mv "$tmp" "$out"
}

sha() { sha256sum "$1" | cut -d' ' -f1; }

[[ "$MODE" == fetch ]] && mkdir -p "$DIR"
status=0
# Lock order matters: a part comes after the file it is taken from.
while read -r file want src; do
    if [[ "$MODE" == fetch && ( ! -f "$DIR/$file" || "$(sha "$DIR/$file")" != "$want" ) ]]; then
        make_file "$file" "$src"
    fi
    if [[ ! -f "$DIR/$file" ]]; then
        echo "MISSING  $file (run scripts/fetch-standards.sh)" >&2; status=1
    elif [[ "$(sha "$DIR/$file")" != "$want" ]]; then
        echo "SHA256   $file is $(sha "$DIR/$file"), lock says $want ($src)" >&2; status=1
    else
        echo "ok       $file"
    fi
done < <(entries)
exit $status
