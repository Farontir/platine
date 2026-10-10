#!/usr/bin/env bash
# Legal guard (see CLAUDE.md, "Interdits"): two vendor names must never appear in the
# product, the code or the user documentation, and proprietary file formats must never
# be committed. Internal planning notes (CLAUDE.md, docs/PLAN.md) are exempt.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

status=0
words='subst''ance|ado''be'
if git grep -n -i -I -E "$words" -- . ':!CLAUDE.md' ':!docs/PLAN.md' ':!scripts/check-forbidden.sh'; then
    echo "error: forbidden vendor name found (see CLAUDE.md)" >&2
    status=1
fi
if git ls-files | grep -i -E '\.(spp|sbsar|sbs|sppr)$'; then
    echo "error: proprietary file format committed (see CLAUDE.md)" >&2
    status=1
fi
[ "$status" -eq 0 ] && echo "forbidden-content check passed"
exit "$status"
