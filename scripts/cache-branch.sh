#!/usr/bin/env bash
# Print a branch name in a form that can go into an actions/cache key.
#
#   scripts/cache-branch.sh BRANCH
#
# actions/cache refuses a key that holds a comma or is longer than 512
# characters (checkKey in actions/toolkit, packages/cache/src/cache.ts),
# and reads restore-keys one per line, trimmed (getInputAsArray in
# actions/cache, src/utils/actionUtils.ts). git check-ref-format bars
# newlines and other control characters from branch names, but not commas,
# great length, or non-ASCII text, whose length JavaScript counts
# differently from bash.
#
# A branch of at most MAX characters from [A-Za-z0-9._/+@-] is printed as
# it is. Any other has every other byte replaced by "_", is cut to
# MAX - 17 characters, and gets "-" and the first 16 hex digits of the
# SHA-256 of the name as given appended, so two branches that clean up
# alike still get keys of their own. The result is at most MAX (200)
# characters, which leaves the key greenshift-db-BRANCH-RUN_ID-ATTEMPT well
# under 512.
set -euo pipefail
export LC_ALL=C

max=200

if [ "$#" -ne 1 ] || [ -z "$1" ]; then
    echo "usage: $0 BRANCH" >&2
    exit 2
fi
branch="$1"

safe="${branch//[!A-Za-z0-9._\/+@-]/_}"
if [ "$safe" = "$branch" ] && [ "${#branch}" -le "$max" ]; then
    printf '%s\n' "$branch"
    exit 0
fi
hash="$(printf '%s' "$branch" | sha256sum | cut -c1-16)"
printf '%s-%s\n' "${safe:0:$((max - 17))}" "$hash"
