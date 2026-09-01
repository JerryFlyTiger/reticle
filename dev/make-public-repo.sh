#!/bin/sh
# Build the publishable copy of Reticle as a fresh repository with a single
# initial commit.
#
#     dev/make-public-repo.sh [DESTINATION]
#
# Default destination is ../reticle-public next to this repo.
#
# Why this exists rather than just pushing this repository:
#
#   1. The history is not publishable. It is 226+ commits whose messages are
#      largely in Chinese, and every one of them contains the Chinese PLAN.md.
#      Deleting PLAN.md today would not help -- `git log -p` still has every
#      version of it.
#
#   2. PLAN.md itself is the internal development log: every wrong turn, every
#      test that was green without meaning anything, cost figures, and the
#      agent-driven process. That reads as admirable rigour on a hobby project
#      and as a curated list of weaknesses to a company evaluating a product.
#      README.md's "Known limitations" section already buys the credibility.
#
# Nothing here pushes anything. Inspect the result, then add a remote yourself.

set -eu

SRC=$(git rev-parse --show-toplevel)
DEST=${1:-"$(dirname "$SRC")/reticle-public"}

# Paths kept out of the public repository, relative to the repo root.
#
# CLAUDE.md is excluded for the same reason as PLAN.md: it is an internal
# working-practice document that catalogues past incidents. Drop it from this
# list if you would rather ship it.
#
# dev/mutations/ is a judgement call and is currently INCLUDED. It demonstrates
# real mutation-testing discipline, which is good evidence of engineering
# quality -- but each list also names the exact defects a milestone fixed. Add
# it here if you would rather not hand that over.
EXCLUDE="PLAN.md CLAUDE.md"

if [ -e "$DEST" ]; then
    echo "error: $DEST already exists; remove it or pass another path" >&2
    exit 1
fi

if ! git -C "$SRC" diff --quiet HEAD; then
    echo "error: working tree has uncommitted changes; commit them first" >&2
    exit 1
fi

mkdir -p "$DEST"

# git archive exports tracked files only at the current commit: no history, no
# target/, no stray untracked scratch files.
git -C "$SRC" archive HEAD | tar -x -C "$DEST"

for path in $EXCLUDE; do
    if [ -e "$DEST/$path" ]; then
        rm -rf "$DEST/$path"
        echo "excluded: $path"
    fi
done

# Warn if a *user-facing document* still points at an excluded file. Source
# comments citing the internal design log (e.g. "see PLAN.md, M56") are
# expected and harmless -- they only tell a reader that a design record exists.
for path in $EXCLUDE; do
    hits=$(cd "$DEST" && grep -rln --include='*.md' -- "$path" . 2>/dev/null || true)
    if [ -n "$hits" ]; then
        echo "warning: a published document still references the excluded '$path':" >&2
        echo "$hits" >&2
    fi
done

cd "$DEST"
git init -q
git add -A
git commit -q -m "Reticle: an Emacs-class editor for Verilog and SystemVerilog RTL

Source-available under the Functional Source License 1.1 with an
Apache-2.0 future grant. See LICENSE.md for the terms and
CONTRIBUTING.md before opening a pull request."

echo
echo "Public repository built at: $DEST"
echo "  files:   $(git ls-files | wc -l | tr -d ' ')"
echo "  commits: $(git rev-list --count HEAD)"
echo
echo "Next: inspect it, then"
echo "  cd $DEST"
echo "  git remote add origin git@github.com:<you>/reticle.git"
echo "  git push -u origin main"
