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

version="$(awk '
    /^\[/ { in_package = ($0 == "[package]") }
    in_package && /^version[[:space:]]*=/ {
        sub(/^version[[:space:]]*=[[:space:]]*"/, "")
        sub(/".*$/, "")
        print
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
