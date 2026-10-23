#!/usr/bin/env bash
# Fetch the libraries pinned in ../third-party.lock into a cache outside the
# working tree.
#
#   scripts/fetch-third-party.sh               check out every pin
#   scripts/fetch-third-party.sh --check       exit 1 unless every checkout is
#                                              present, clean and at its pin
#   scripts/fetch-third-party.sh --print-dir   print the cache directory
#
# Env: REXI_EXAMPLES_THIRD_PARTY  cache directory
#      (default ${XDG_CACHE_HOME:-$HOME/.cache}/rexi/examples/mqtt-todo)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LOCK="$ROOT/third-party.lock"
DIR="${REXI_EXAMPLES_THIRD_PARTY:-${XDG_CACHE_HOME:-$HOME/.cache}/rexi/examples/mqtt-todo}"

MODE=fetch
case "${1:-}" in
    "") ;;
    --check) MODE=check ;;
    --print-dir) echo "$DIR"; exit 0 ;;
    -h|--help) sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
esac

entries() { awk '!/^[[:space:]]*(#|$)/ { print $1, $2, $3, $4 }' "$LOCK"; }

git_fetch() { # name url sha [sparse path]
    local name="$1" url="$2" sha="$3" sparse="${4:-}" path="$DIR/$1"
    command -v git >/dev/null || { echo "missing required command: git" >&2; return 1; }
    if [[ ! -d "$path/.git" ]]; then
        [[ -e "$path" ]] && { echo "$path exists but is not a git checkout" >&2; return 1; }
        mkdir -p "$path"
        git -C "$path" init -q
        git -C "$path" remote add origin "$url"
        # Only this folder is checked out, and only its files are downloaded.
        if [[ -n "$sparse" ]]; then
            git -C "$path" sparse-checkout set "$sparse"
        fi
    fi
    # The lock may have moved this entry to another url.
    git -C "$path" remote set-url origin "$url"
    if [[ -n "$(git -C "$path" status --porcelain --untracked-files=no)" ]]; then
        echo "$name: $path has local changes; refusing to touch it" >&2
        return 1
    fi
    if [[ "$(git -C "$path" rev-parse -q --verify HEAD || true)" == "$sha" ]]; then
        echo "$name: at $sha"
        return 0
    fi
    echo "$name: fetching $sha"
    local filter=()
    [[ -n "$sparse" ]] && filter=(--filter=blob:none)
    git -C "$path" fetch -q --depth 1 "${filter[@]}" origin "$sha"
    git -C "$path" checkout -q --detach FETCH_HEAD
}

git_check() { # name sha
    local name="$1" sha="$2" path="$DIR/$1" head
    [[ -d "$path/.git" ]] || { echo "MISSING  $name ($path)" >&2; return 1; }
    head="$(git -C "$path" rev-parse -q --verify HEAD || true)"
    [[ "$head" == "$sha" ]] || { echo "MOVED    $name: at ${head:-nothing}, pinned $sha" >&2; return 1; }
    [[ -z "$(git -C "$path" status --porcelain --untracked-files=no)" ]] || { echo "DIRTY    $name" >&2; return 1; }
    echo "ok       $name $sha"
}

svn_fetch() { # name url rev path
    local name="$1" url="$2" rev="$3" sub="$4" path="$DIR/$1"
    command -v svn >/dev/null || { echo "missing required command: svn" >&2; return 1; }
    if [[ -d "$path/.svn" ]]; then
        if [[ -n "$(svn status -q "$path")" ]]; then
            echo "$name: $path has local changes; refusing to touch it" >&2
            return 1
        fi
        if [[ "$(svn info --show-item revision "$path")" == "$rev" ]]; then
            echo "$name: at r$rev"
            return 0
        fi
        echo "$name: updating to r$rev"
        svn update -q -r "$rev" "$path"
    else
        [[ -e "$path" ]] && { echo "$path exists but is not a Subversion checkout" >&2; return 1; }
        echo "$name: checking out r$rev"
        mkdir -p "$DIR"
        svn checkout -q -r "$rev" "$url/$sub" "$path"
    fi
}

svn_check() { # name rev
    local name="$1" rev="$2" path="$DIR/$1" at
    [[ -d "$path/.svn" ]] || { echo "MISSING  $name ($path)" >&2; return 1; }
    at="$(svn info --show-item revision "$path")"
    [[ "$at" == "$rev" ]] || { echo "MOVED    $name: at r$at, pinned r$rev" >&2; return 1; }
    [[ -z "$(svn status -q "$path")" ]] || { echo "DIRTY    $name" >&2; return 1; }
    echo "ok       $name r$rev"
}

status=0
mkdir -p "$DIR"
while read -r name url pin sub; do
    case "$url:$MODE" in
        svn:*:fetch) svn_fetch "$name" "${url#svn:}" "$pin" "$sub" || status=1 ;;
        svn:*:check) svn_check "$name" "$pin" || status=1 ;;
        *:fetch) git_fetch "$name" "$url" "$pin" "$sub" || status=1 ;;
        *:check) git_check "$name" "$pin" || status=1 ;;
    esac
done < <(entries)
[[ $MODE == fetch ]] && echo "third-party cache: $DIR"
exit $status
