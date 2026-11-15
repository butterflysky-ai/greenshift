#!/usr/bin/env bash
# What stops a pull request writing the test history main's runs read is
# GitHub, which scopes caches by ref: a cache saved by a pull request is
# restorable only from that pull request. This script is a regression
# lint against honest edits: it checks, from the workflow file, that the
# workflow still asks for only pushes to main to save the history:
#
# - no job calls a reusable workflow, whose steps this cannot see;
# - no step uses actions/cache itself, which saves in a post step on
#   every event, pull requests included;
# - every actions/cache/save step runs only on push events, under
#   exactly the condition below;
# - push events are main's only.
#
# A static check of this one file: it reads what the workflow asks for,
# and cannot show what GitHub does with it, nor see into other workflow
# files or into actions other than actions/cache.
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

# A job that calls a reusable workflow has no steps of its own here, so
# the checks below would pass it unseen. Fail on one instead.
callers="$(yq -o=json -I=0 '[.jobs | to_entries | .[] | select(.value | has("uses")) | .key]' "$workflow")"
if [ "$callers" != '[]' ]; then
    fail "jobs $callers call a reusable workflow, whose steps this check cannot see"
fi

steps='.jobs[].steps[]'

# GitHub reads an action's owner and repository without regard to case,
# so these patterns do too.
combined="$(yq "[$steps | select(.uses // \"\" | test(\"(?i)^actions/cache@\"))] | length" "$workflow")"
if [ "$combined" -ne 0 ]; then
    fail "$combined step(s) use actions/cache, which saves on pull requests too; use actions/cache/restore and actions/cache/save"
fi

saves="$(yq "[$steps | select(.uses // \"\" | test(\"(?i)^actions/cache/save@\"))] | length" "$workflow")"
if [ "$saves" -eq 0 ]; then
    fail "no actions/cache/save step to check"
else
    # Read the conditions first, so a yq failure stops the script (set -e)
    # rather than leaving the loop nothing to check.
    conds="$(yq "$steps | select(.uses // \"\" | test(\"(?i)^actions/cache/save@\")) | .if // \"(always)\"" "$workflow")"
    while IFS= read -r cond; do
        if [ "$cond" != "$save_if" ]; then
            fail "an actions/cache/save step runs if '$cond', not '$save_if'"
        fi
    done <<< "$conds"
fi

branches="$(yq -o=json -I=0 '.on.push.branches' "$workflow")"
if [ "$branches" != '["main"]' ]; then
    fail "push events run on branches $branches, not only main"
fi

if [ "$status" -eq 0 ]; then
    echo "$workflow: no reusable workflow jobs, no actions/cache steps, $saves cache save step(s), each only on push to main"
fi
exit "$status"
