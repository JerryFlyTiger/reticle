#!/bin/sh
# Assemble target/Reticle.app, a minimal macOS app bundle around the release
# binary, so the Dock shows the real icon instead of the generic terminal
# icon a bare executable gets. This is the only path that changes the Dock
# icon: with_icon() in crates/frontend-gui/src/lib.rs drives the window and
# taskbar icon on Linux and Windows, but on macOS the Dock reads the icon
# from the app bundle's Contents/Resources/*.icns, not from the running
# process, so that call is a no-op there.
#
#     dev/make-app-bundle.sh
#
# macOS only. Rebuilds the release binary, then assembles the bundle fresh
# each run (any previous target/Reticle.app is removed first), so a rerun
# after a code or icon change always reflects the current tree.

set -eu

if [ "$(uname)" != "Darwin" ]; then
    echo "error: this script is macOS-only (it builds a .app bundle)" >&2
    exit 1
fi

ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"

cargo build --release --workspace

BIN="$ROOT/target/release/reticle"
if [ ! -x "$BIN" ]; then
    echo "error: $BIN was not produced by the release build" >&2
    exit 1
fi

ICNS="$ROOT/assets/icon/reticle.icns"
if [ ! -f "$ICNS" ]; then
    echo "error: $ICNS not found" >&2
    exit 1
fi

# Version comes from the workspace's Cargo.toml, not hard-coded here, so the
# bundle can't silently drift from what was actually built. Anchored to the
# [workspace.package] table specifically -- a bare '^version = "..."' match
# would also hit a table-style dependency entry (e.g. a
# [dependencies.foo] block with its own "version" key) added anywhere
# earlier in the file, silently poisoning the bundle version with no error.
VERSION=$(awk '
    /^\[workspace\.package\]/ { in_section = 1; next }
    # A [workspace.package.metadata] sub-table also begins with a bracket
    # but does not end the section, so exclude that prefix -- otherwise a
    # sub-table placed above the version line makes this find nothing and
    # the guard below refuses to build a bundle it could have built.
    # (No apostrophes in here: this awk program is a single-quoted shell
    # string, and one would end it.)
    /^\[/ && !/^\[workspace\.package\./ { in_section = 0 }
    in_section && /^version *= *"/ {
        sub(/^version *= *"/, "")
        sub(/".*/, "")
        print
        exit
    }
' "$ROOT/Cargo.toml")
if [ -z "$VERSION" ]; then
    echo "error: could not read version from $ROOT/Cargo.toml" >&2
    exit 1
fi

APP="$ROOT/target/Reticle.app"
rm -rf "$APP"

MACOS_DIR="$APP/Contents/MacOS"
RES_DIR="$APP/Contents/Resources"
mkdir -p "$MACOS_DIR" "$RES_DIR"

cp "$BIN" "$MACOS_DIR/reticle"
cp "$ICNS" "$RES_DIR/reticle.icns"

# CFBundleIdentifier: reverse-DNS under the project's own domain-shaped name
# (no company registered one exists yet); "app.reticle" is short, stable,
# and won't collide with anything else on a developer's machine.
cat >"$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>Reticle</string>
    <key>CFBundleExecutable</key>
    <string>reticle</string>
    <key>CFBundleIconFile</key>
    <string>reticle</string>
    <key>CFBundleIdentifier</key>
    <string>app.reticle</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>$VERSION</string>
    <key>CFBundleVersion</key>
    <string>$VERSION</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
PLIST

# macOS 26's Dock and Finder prefer the new .icon package format
# (CFBundleIconName + Assets.car) over a legacy .icns, which macOS 26 places
# inside a system container instead of rendering directly. Building it
# requires Xcode's actool, which is not guaranteed to be on the machine
# doing the build (e.g. a CI runner with only the command-line tools), so
# this is additive: on a machine without it, the bundle above is already
# complete and correct -- just older-style -- and this block only upgrades
# it.
ICONPKG="$ROOT/assets/icon/Reticle.icon"
if ACTOOL=$(xcrun --find actool 2>/dev/null); then
    ACTOOL_TMP=$(mktemp -d)
    # Always clean up the compile scratch dir, whichever way actool exits.
    trap 'rm -rf "$ACTOOL_TMP"' EXIT

    # actool also emits its own Reticle.icns, generated from the flat glyph
    # and the gradient -- NOT the hand-lit artwork in reticle.icns. Copying
    # it in would shadow ours for every macOS version older than 26, which
    # is a strict downgrade. Only Assets.car is wanted here; CFBundleIconFile
    # stays pointed at our own "reticle" icns as the pre-26 fallback.
    #
    # The whole step is best effort. The legacy bundle is already complete
    # by this point, so a failure here must leave it that way rather than
    # abort with a half-written Resources directory. Hence the guarded
    # chain: the copy lands under a temporary name and is renamed into
    # place, and CFBundleIconName -- the key that makes macOS 26 prefer
    # Assets.car over the icns -- is written only once the file is there
    # under its final name. Without that key the bundle simply falls back
    # to the icns, which is exactly the pre-26 behaviour.
    if "$ACTOOL" --output-format human-readable-text --notices --warnings \
            --platform macosx --target-device mac --minimum-deployment-target 26.0 \
            --app-icon Reticle --output-partial-info-plist "$ACTOOL_TMP/partial.plist" \
            --compile "$ACTOOL_TMP" "$ICONPKG" > "$ACTOOL_TMP/actool.log" 2>&1 \
        && cp "$ACTOOL_TMP/Assets.car" "$RES_DIR/Assets.car.tmp" \
        && mv "$RES_DIR/Assets.car.tmp" "$RES_DIR/Assets.car" \
        && plutil -insert CFBundleIconName -string Reticle "$APP/Contents/Info.plist"
    then
        echo "compiled assets/icon/Reticle.icon -> $RES_DIR/Assets.car (actool found at $ACTOOL)"
    else
        rm -f "$RES_DIR/Assets.car.tmp" "$RES_DIR/Assets.car"
        echo "actool step failed: bundle carries the legacy .icns only. Last output:"
        tail -5 "$ACTOOL_TMP/actool.log" 2>/dev/null || true
    fi

    rm -rf "$ACTOOL_TMP"
    trap - EXIT
else
    echo "actool not found (no Xcode): bundle carries the legacy .icns only -- macOS 26 will draw it inside a system container instead of natively"
fi

echo "wrote $APP"
