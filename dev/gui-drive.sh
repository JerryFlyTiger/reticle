#!/bin/sh
# Drive the GUI's *runtime* behaviour from elisp and capture the screen as it
# changes, without sending a single keystroke.
#
#     dev/gui-drive.sh DRIVER.el FILE-TO-OPEN OUT-DIR [SECONDS...]
#
# Example (the one this tool was written for -- proving that switching fonts
# while running produces the same screen as choosing that font at startup):
#
#     dev/gui-drive.sh dev/drivers/switch-font.el demo/rtl/core/alu.sv /tmp/out 1 4 8 12
#
# Why this exists
# ---------------
# `dev/gui-shot.sh` answers "what does the GUI look like", one frame, at
# startup. It cannot answer "what happens when something changes while it is
# running" -- and that is where the interesting defects live: a font switch
# that leaves a stale glyph cache draws the OLD font's glyphs, a theme change
# that misses a repaint shows half the old colours.
#
# The obvious way to drive a running GUI is to send it keystrokes. On macOS
# that means System Events, which needs the window to come to the front, and
# an automated session generally cannot make that happen (measured 2026-09-04:
# three attempts, focus never left the controlling app). Sending keys blind is
# not an option -- they land in whatever app *is* frontmost.
#
# The way around it: **the GUI re-reads its elisp configuration every frame,
# and calls a handful of elisp functions every idle tick** (`idle_tick`,
# crates/core/src/lib.rs). Redefine one of those functions in the init file and
# you have a timer -- entirely inside the editor's own public elisp surface, no
# product code changed, no keyboard involved. See dev/drivers/switch-font.el,
# and note its warning about firing on elapsed time rather than a tick count:
# the tick's own interval varies from 33ms to 500ms depending on focus and
# outstanding async work.
#
# What is sandboxed, and what is not
# ----------------------------------
# The editor loads `$HOME/.reticle/init.el` (crates/core/src/lib.rs's
# `load_user_init`), so this script runs it under a throwaway `HOME` and copies
# DRIVER.el in as that home's init file. Your real configuration is never
# touched. An earlier version of this experiment moved the real
# ~/.reticle/init.el aside and moved it back, which works exactly until
# something interrupts the script.
#
# **FILE-TO-OPEN is not sandboxed** -- it is opened at its real path. No driver
# needs to modify a buffer today, but one that did and then saved would write
# to the real file, and the throwaway home would not protect it. Point drivers
# that edit anything at a copy.
#
# One more consequence of the synthetic home: the editor sees an empty
# `~/Library/Fonts`, so a font installed only for your user is invisible to the
# driven instance. Fonts under /System and /Library, and the ones compiled into
# the binary, are unaffected.
#
# macOS only: it uses `screencapture`, same as dev/gui-shot.sh, and needs the
# same Screen Recording permission (System Settings -> Privacy & Security).

set -eu

if [ "$(uname)" != "Darwin" ]; then
    echo "error: this script is macOS-only (it uses screencapture)" >&2
    exit 1
fi

if [ $# -lt 3 ]; then
    echo "usage: dev/gui-drive.sh DRIVER.el FILE-TO-OPEN OUT-DIR [SECONDS...]" >&2
    exit 2
fi

DRIVER=$1
FILE=$2
OUTDIR=$3
shift 3
# Default schedule: often enough to bracket a change, cheap enough to rerun.
[ $# -gt 0 ] || set -- 1 4 8 12

ROOT=$(cd "$(dirname "$0")/.." && pwd)
HELPER="${TMPDIR:-/tmp}/reticle-gui-shot"

[ -f "$DRIVER" ] || { echo "error: no driver file at $DRIVER" >&2; exit 1; }
[ -f "$FILE" ] || { echo "error: no file to open at $FILE" >&2; exit 1; }

# A schedule that is not strictly increasing would still produce files, named
# for times they were not taken at -- and reading the moment of a change off
# those names is this tool's entire purpose. Refuse it rather than produce it.
PREV=-1
for T in "$@"; do
    case $T in
        '' | *[!0-9]*)
            echo "error: capture times must be whole seconds, got '$T'" >&2
            exit 2
            ;;
    esac
    if [ "$T" -le "$PREV" ]; then
        echo "error: capture times must strictly increase; '$T' follows '$PREV'" >&2
        exit 2
    fi
    PREV=$T
done

mkdir -p "$OUTDIR"

# Pick the newer of the two binaries, then refuse to run against a stale one.
# Identical reasoning to dev/gui-shot.sh's gate, and for the same reason: on
# 2026-09-04 a screenshot taken against a binary linked 73 minutes before the
# fix under test was read as "the fix does nothing", which cost five wrong
# hypotheses. A driven run is even easier to misread, because the whole point
# is that the screen changes on its own.
BIN="$ROOT/target/debug/reticle"
if [ ! -x "$BIN" ] || { [ -x "$ROOT/target/release/reticle" ] &&
    [ "$ROOT/target/release/reticle" -nt "$BIN" ]; }; then
    BIN="$ROOT/target/release/reticle"
fi
if [ ! -x "$BIN" ]; then
    echo "error: no reticle binary; run 'cargo build --workspace' first" >&2
    exit 1
fi
# Only sources that can actually change the binary: a test or bench file is
# newer constantly (they are edited far more often than product code) and
# blocking on those would train everyone to ignore this gate.
STALE=$(find "$ROOT/crates" "$ROOT/src" -name '*.rs' \
    -not -path '*/tests/*' -not -path '*/benches/*' \
    -newer "$BIN" -print -quit)
if [ -n "$STALE" ]; then
    echo "error: $BIN is older than $STALE -- run 'cargo build --workspace' first" >&2
    exit 1
fi

# Rebuild the window locator only when it is missing or older than its source.
if [ ! -x "$HELPER" ] || [ "$ROOT/dev/gui-shot.swift" -nt "$HELPER" ]; then
    swiftc -O -o "$HELPER" "$ROOT/dev/gui-shot.swift"
fi

FAKEHOME=$(mktemp -d "${TMPDIR:-/tmp}/reticle-gui-drive.XXXXXX")
APP=""
# Registered here, before anything else can fail: the `mkdir`/`cp` below can
# fail on a full disk, and with `set -e` that would end the script while the
# temporary home is already on disk with nothing listening to remove it.
#
# `|| true` on the kill is load-bearing, not decoration. Under `set -e` the
# last command actually executed in an `&&` list still triggers errexit when it
# fails -- so once the editor has exited on its own (the "no window appeared"
# branch below reaches `exit 1` precisely *because* it is dead), `kill` fails,
# the function aborts on that line, and `rm -rf` never runs. dev/gui-shot.sh
# has always had the `|| true`; this script had reinvented the line without it.
cleanup() {
    [ -n "$APP" ] && kill "$APP" 2>/dev/null || true
    rm -rf "$FAKEHOME"
    return 0
}
trap cleanup EXIT INT TERM

mkdir -p "$FAKEHOME/.reticle"
cp "$DRIVER" "$FAKEHOME/.reticle/init.el"

HOME="$FAKEHOME" "$BIN" "$FILE" >"$FAKEHOME/stdout.txt" 2>"$FAKEHOME/stderr.txt" &
APP=$!

INFO=""
i=0
while [ $i -lt 15 ]; do
    if ! kill -0 "$APP" 2>/dev/null; then
        echo "error: the editor exited before a window appeared; stderr:" >&2
        tail -5 "$FAKEHOME/stderr.txt" >&2
        exit 1
    fi
    INFO=$("$HELPER" reticle 2>/dev/null || true)
    [ -n "$INFO" ] && break
    sleep 1
    i=$((i + 1))
done
[ -n "$INFO" ] || { echo "error: no window found after 15s" >&2; exit 1; }

# The locator prints "ID x y w h". Taking field 1 inside a function keeps
# `set --` away from this script's own positional parameters, which still hold
# the capture schedule.
set_window_id() { WINDOW_ID=$1; }
# shellcheck disable=SC2086
set_window_id $INFO

# One path per line rather than a space-joined string: OUT-DIR is user-supplied
# and a space in it would split every path into fragments, which the summary
# below would then report as missing files.
SHOTS_FILE="$FAKEHOME/shots.txt"
: >"$SHOTS_FILE"

ELAPSED=0
for T in "$@"; do
    WAIT=$((T - ELAPSED))
    [ "$WAIT" -gt 0 ] && sleep "$WAIT"
    ELAPSED=$T
    OUT="$OUTDIR/shot-${T}s.png"
    if screencapture -x -o -l"$WINDOW_ID" "$OUT" 2>/dev/null && [ -s "$OUT" ]; then
        echo "wrote $OUT"
        printf '%s\n' "$OUT" >>"$SHOTS_FILE"
    else
        rm -f "$OUT"
        echo "error: screencapture failed -- grant Screen Recording permission to" >&2
        echo "       this terminal (System Settings -> Privacy & Security), then" >&2
        echo "       restart it. Locating the window needs no permission; the" >&2
        echo "       pixels do." >&2
        exit 1
    fi
done

# The summary is the point: a driven run is only useful if you can see WHEN the
# screen changed. Consecutive diffs make the moment obvious -- a few hundred
# pixels is the cursor blinking, tens of thousands is the thing you drove.
#
# The summary logic itself lives in dev/pixdiff.py, not inline here, because
# an inline heredoc cannot be tested -- see dev/test_pixdiff.py for what that
# cost: two real bugs (an RGB-only comparison that made a pure alpha/opacity
# change invisible, then a luma conversion that dropped alpha again after the
# first fix) both shipped and both survived a self-check, and only a cold
# reviewer caught them. dev/test_pixdiff.py pins both down by name so they
# cannot come back unnoticed.
if command -v python3 >/dev/null 2>&1; then
    echo
    python3 "$ROOT/dev/pixdiff.py" "$SHOTS_FILE"
fi
