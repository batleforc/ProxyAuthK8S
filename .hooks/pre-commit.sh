#!/usr/bin/env bash
# Pre-commit hook: regenerate the generated artifacts, lint, and scan for secrets.
#
# `task recu` rewrites generated files (swagger, CRDs, API clients) and runs
# `cargo fmt --all`, so the index must be refreshed afterwards. Instead of a
# blanket `git add .` (which would sweep in untracked/unrelated work), only:
#   - the generated output paths are re-staged (tracked changes + new files);
#   - files that were *fully* staged before the hook are re-staged, so a
#     formatter fix lands in the commit while partially staged files keep
#     their unstaged hunks out of it.
set -e

if [ ! -f .hooks/hook-gen/check_staged.py ]; then
    echo "pre-commit: watermarks-remover submodule missing, run 'git submodule update --init' (or 'task init')" >&2
    exit 1
fi

# Outputs of `task recu` (see Taskfile.yaml / .taskfile/gen.yaml).
GENERATED_PATHS="
swagger.json
.docs/swagger.json
deploy/crds.yaml
deploy/chart-crd/templates/crds.yaml
libs/cli/client_api
libs/front/front-api/src/lib
"

# Staged files that have no unstaged changes yet (NUL-separated, path-safe).
TMP_FULLY_STAGED=$(mktemp)
trap 'rm -f "$TMP_FULLY_STAGED"' EXIT
git diff --cached --name-only -z --diff-filter=ACMR | while IFS= read -r -d '' f; do
    if git diff --quiet -- "$f"; then
        printf '%s\0' "$f"
    fi
done >"$TMP_FULLY_STAGED"

restage() {
    for p in $GENERATED_PATHS; do
        # Skip paths that neither exist nor are tracked (git add would fail).
        if [ -e "$p" ] || [ -n "$(git ls-files -- "$p")" ]; then
            git add -A -- "$p"
        fi
    done
    if [ -s "$TMP_FULLY_STAGED" ]; then
        xargs -0 git add -- <"$TMP_FULLY_STAGED"
    fi
}

task recu
restage # generated files must be staged before linting
task lint
restage # in case linting fixed anything
gitleaks git --pre-commit --redact --staged --verbose
.hooks/end-of-line.sh
.hooks/whitespace-fixer.sh
# Fail on AI/C2PA provenance marks in staged files (scripts from the
# watermarks-remover submodule, see .hooks/hook-gen).
# Only regular files: the script rejects directories, so the submodule gitlink
# and symlinks to directories (.hooks/hook-gen) must be skipped.
git diff --cached --name-only -z --diff-filter=ACMR |
    while IFS= read -r -d '' f; do
        if [ -f "$f" ]; then
            printf '%s\0' "$f"
        fi
    done |
    xargs -0 -r uv run .hooks/hook-gen/check_staged.py
