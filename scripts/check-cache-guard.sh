#!/usr/bin/env bash
# Check, from the workflow file, that a pull request cannot write the test
# history main's runs read:
#
# - no step uses actions/cache itself, which saves in a post step on
#   every event, pull requests included;
# - every actions/cache/save step runs only on push events, under
#   exactly the condition below;
# - push events are main's only.
#
# A static check: it reads what the workflow asks for, and cannot show
# what GitHub does with it. (GitHub also scopes caches by ref: one saved
# by a pull request is restorable only from that pull request.)
#
#   scripts/check-cache-guard.sh [WORKFLOW]   (default .github/workflows/ci.yml)
#
# Needs mikefarah's yq v4. Exits 1, saying why, if a check fails.
set -euo pipefail

workflow="${1:-.github/workflows/ci.yml}"
save_if="always() && github.event_name == 'push'"
status=0

fail() {
    echo "error: $workflow: $*" >&2
    status=1
}

steps='.jobs[].steps[]'

combined="$(yq "[$steps | select(.uses // \"\" | test(\"^actions/cache@\"))] | length" "$workflow")"
if [ "$combined" -ne 0 ]; then
    fail "$combined step(s) use actions/cache, which saves on pull requests too; use actions/cache/restore and actions/cache/save"
fi

saves="$(yq "[$steps | select(.uses // \"\" | test(\"^actions/cache/save@\"))] | length" "$workflow")"
if [ "$saves" -eq 0 ]; then
    fail "no actions/cache/save step to check"
fi
# Read the conditions first, so a yq failure stops the script (set -e)
# rather than leaving the loop nothing to check.
conds="$(yq "$steps | select(.uses // \"\" | test(\"^actions/cache/save@\")) | .if // \"(always)\"" "$workflow")"
while IFS= read -r cond; do
    if [ "$cond" != "$save_if" ]; then
        fail "an actions/cache/save step runs if '$cond', not '$save_if'"
    fi
done <<< "$conds"

branches="$(yq -o=json -I=0 '.on.push.branches' "$workflow")"
if [ "$branches" != '["main"]' ]; then
    fail "push events run on branches $branches, not only main"
fi

if [ "$status" -eq 0 ]; then
    echo "$workflow: $saves cache save step(s), each only on push to main"
fi
exit "$status"
