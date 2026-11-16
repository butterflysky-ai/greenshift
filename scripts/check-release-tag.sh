#!/usr/bin/env bash
# Check that a release tag names the version in Cargo.toml.
#
#   scripts/check-release-tag.sh TAG [CARGO_TOML]
#
# TAG must be "v" followed by the [package] version, e.g. v0.1.0.
# CARGO_TOML defaults to Cargo.toml. Exits 1 if they differ.
set -euo pipefail

if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then
    echo "usage: $0 TAG [CARGO_TOML]" >&2
    exit 2
fi

tag="$1"
manifest="${2:-Cargo.toml}"

# A plain `version = "..."` (or '...') under [package]; anything else,
# such as version.workspace = true, is reported as no version.
version="$(awk '
    { sub(/\r$/, "") }
    /^[[:space:]]*\[/ {
        in_package = ($0 ~ /^[[:space:]]*\[[[:space:]]*package[[:space:]]*\][[:space:]]*(#.*)?$/)
    }
    in_package && /^[[:space:]]*version[[:space:]]*=[[:space:]]*["\047]/ {
        sub(/^[[:space:]]*version[[:space:]]*=[[:space:]]*/, "")
        q = substr($0, 1, 1)
        $0 = substr($0, 2)
        end = index($0, q)
        if (end) print substr($0, 1, end - 1)
        exit
    }
' "$manifest")"

if [ -z "$version" ]; then
    echo "::error::no [package] version in $manifest"
    exit 1
fi

if [ "$tag" != "v$version" ]; then
    echo "::error::tag $tag does not match Cargo.toml version $version (expected v$version)"
    exit 1
fi

echo "tag $tag matches Cargo.toml version $version"
