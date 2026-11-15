#!/usr/bin/env bash
# Check that two nextest filtersets split the test suite between them:
# every test `cargo nextest list` lists is selected by exactly one of them.
#
#   scripts/check-partition.sh POOR_EXPR_FILE RICH_EXPR_FILE
#
# Each file holds one filterset, as `greenshift nextest-filter` prints it.
# Exits 1, naming the tests, if a test is in both or in neither. (A
# filterset selects from the full list, so a test cannot be in a pass and
# not in it.)
set -euo pipefail

if [ "$#" -ne 2 ]; then
    echo "usage: $0 POOR_EXPR_FILE RICH_EXPR_FILE" >&2
    exit 2
fi

dir="$(mktemp -d)"
trap 'rm -rf "$dir"' EXIT
export LC_ALL=C

list() {
    cargo nextest list --locked --message-format oneline "$@" | sort
}

list > "$dir/all"
list -E "$(cat "$1")" > "$dir/poor"
list -E "$(cat "$2")" > "$dir/rich"

echo "tests: $(wc -l < "$dir/all") in all, $(wc -l < "$dir/poor") poor, $(wc -l < "$dir/rich") rich"

status=0
both="$(comm -12 "$dir/poor" "$dir/rich")"
if [ -n "$both" ]; then
    echo "error: tests in both passes:" >&2
    printf '%s\n' "$both" >&2
    status=1
fi
neither="$(sort "$dir/poor" "$dir/rich" | comm -23 "$dir/all" -)"
if [ -n "$neither" ]; then
    echo "error: tests in neither pass:" >&2
    printf '%s\n' "$neither" >&2
    status=1
fi
exit "$status"
