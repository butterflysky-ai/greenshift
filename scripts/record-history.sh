#!/usr/bin/env bash
# Record a CI job's test passes in the greenshift history as one run.
#
#   scripts/record-history.sh GREENSHIFT DB COMMIT SOURCE JUNIT_XML...
#
# GREENSHIFT is the greenshift binary; DB, COMMIT and SOURCE are passed to
# `greenshift ingest` as --db, --commit and --source. Each JUNIT_XML is the
# file one pass writes; a pass that did not run, or stopped before writing
# it, leaves none. With every file present, the run is recorded as whole;
# with only some, as partial (`ingest --partial`), so that it never counts
# as a full run; with none, nothing is recorded and the store is left
# alone.
set -euo pipefail

if [ "$#" -lt 5 ]; then
    echo "usage: $0 GREENSHIFT DB COMMIT SOURCE JUNIT_XML..." >&2
    exit 2
fi

greenshift="$1"
db="$2"
commit="$3"
source="$4"
shift 4

present=()
for junit in "$@"; do
    if [ -f "$junit" ]; then
        present+=("$junit")
    else
        echo "no JUnit at $junit"
    fi
done

if [ "${#present[@]}" -eq 0 ]; then
    echo "no JUnit to record"
    exit 0
fi

partial=()
if [ "${#present[@]}" -lt "$#" ]; then
    partial=(--partial)
    echo "recording ${#present[@]} of $# passes, as a partial run"
fi

exec "$greenshift" ingest --db "$db" --commit "$commit" --source "$source" \
    "${partial[@]}" "${present[@]}"
