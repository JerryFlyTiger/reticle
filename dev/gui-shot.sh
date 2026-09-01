#!/bin/sh
# Screenshot the GUI front end, so a claim about how it renders can be checked
# by looking rather than by trusting a comment.
#
#     dev/gui-shot.sh [FILE-TO-OPEN] [OUTPUT.png]
#
# Defaults to opening demo/rtl/top/soc_top.sv and writing to gui-shot.png in
# the current directory. Launches the editor, waits for its window to appear,
# captures only that window, and kills the process.
#
# Why this exists: every GUI claim in this project has been verified by a human
# looking at the screen, because egui has no headless rendering path. That is
# recorded as a known gap in PLAN.md and it has never been closed. This does
# not make the GUI testable in CI, but it does mean a change to the rendering
# can be reviewed from an artifact instead of a description.
#
# macOS only, and it needs one permission that cannot be granted from a script:
#
#     System Settings -> Privacy & Security -> Screen Recording
#
# Grant it to the terminal (or whichever app runs this), then restart that app.
# Without it `screencapture` exits with "could not create image from rect" and
# this script tells you so rather than writing a misleading blank file. Note
# that locating the window needs no permission at all -- only the pixels do.

set -eu

FILE=${1:-demo/rtl/top/soc_top.sv}
OUT=${2:-gui-shot.png}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
HELPER="${TMPDIR:-/tmp}/reticle-gui-shot"

if [ "$(uname)" != "Darwin" ]; then
    echo "error: this script is macOS-only (it uses screencapture)" >&2
    exit 1
fi

BIN="$ROOT/target/debug/reticle"
[ -x "$BIN" ] || BIN="$ROOT/target/release/reticle"
if [ ! -x "$BIN" ]; then
    echo "error: no reticle binary; run 'cargo build --workspace' first" >&2
    exit 1
fi

# Rebuild the locator only when it is missing or older than its source.
if [ ! -x "$HELPER" ] || [ "$ROOT/dev/gui-shot.swift" -nt "$HELPER" ]; then
    swiftc -O -o "$HELPER" "$ROOT/dev/gui-shot.swift"
fi

"$BIN" "$FILE" >/dev/null 2>&1 &
APP=$!
# Kill the editor however we leave this script, including on error.
trap 'kill $APP 2>/dev/null || true' EXIT INT TERM

INFO=""
i=0
while [ $i -lt 15 ]; do
    if ! kill -0 $APP 2>/dev/null; then
        echo "error: the editor exited before a window appeared" >&2
        exit 1
    fi
    INFO=$("$HELPER" reticle 2>/dev/null || true)
    [ -n "$INFO" ] && break
    sleep 1
    i=$((i + 1))
done

if [ -z "$INFO" ]; then
    echo "error: no window found after 15s" >&2
    exit 1
fi

# shellcheck disable=SC2086
set -- $INFO
WINDOW_ID=$1

if screencapture -x -o -l"$WINDOW_ID" "$OUT" 2>/dev/null && [ -s "$OUT" ]; then
    echo "wrote $OUT ($(($(wc -c <"$OUT") / 1024)) KB, window ${4}x${5})"
else
    rm -f "$OUT"
    cat >&2 <<'MSG'
error: screencapture could not read the window.

This is the Screen Recording permission, not a bug here -- the window was
found, only its pixels are gated. Grant it in

    System Settings -> Privacy & Security -> Screen Recording

to the app running this script, restart that app, and run this again.
MSG
    exit 1
fi
