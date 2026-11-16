#!/usr/bin/env bash
# Check whether a release tag names a semver pre-release.
#
#   scripts/is-prerelease-tag.sh TAG
#
# Exits 0 (and prints "yes") if the version has a "-" before any "+", as
# in v0.2.0-rc.1; exits 1 (and prints "no") otherwise. Build metadata
# (after a "+") may hold a "-" too, but does not make a pre-release:
# v0.2.0+build-1 is a release.
set -euo pipefail

if [ "$#" -ne 1 ]; then
    echo "usage: $0 TAG" >&2
    exit 2
fi

tag="$1"

case "${tag%%+*}" in
    *-*)
        echo "yes"
        exit 0
        ;;
    *)
        echo "no"
        exit 1
        ;;
esac
