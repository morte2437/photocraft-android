#!/usr/bin/env bash
# Smoke test for packaging/linux/AppRun: the AppImage's desktop integration writes
# and refreshes the host .desktop entry and icons, is idempotent, honours the opt-out
# and an unwritable HOME, and always launches the app (#593).
#
# Runs in seconds with no build (CI: .github/workflows/packaging-lint.yml).
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
APP_ID=ai.storyteller.photocraft

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# Fake AppDir with the real AppRun, .desktop template, icons, and a stub binary.
APPDIR="$TMP/PhotoCraft.AppDir"
mkdir -p "$APPDIR/usr/bin"
install -Dm755 "$HERE/AppRun" "$APPDIR/AppRun"
install -Dm644 "$HERE/$APP_ID.desktop" "$APPDIR/usr/share/applications/$APP_ID.desktop"
mkdir -p "$APPDIR/usr/share/icons"
cp -R "$ROOT/assets/app-icon/hicolor" "$APPDIR/usr/share/icons/"
printf '#!/bin/sh\necho "stub: $*"\n' >"$APPDIR/usr/bin/photocraft"
chmod +x "$APPDIR/usr/bin/photocraft"
printf '#!/bin/sh\necho "cli stub: $*"\n' >"$APPDIR/usr/bin/photocraft-cli"
chmod +x "$APPDIR/usr/bin/photocraft-cli"

APPIMAGE="$TMP/photocraft test.AppImage" # spaces on purpose: Exec must stay quoted
: >"$APPIMAGE"
export APPIMAGE
export HOME="$TMP/home"
export XDG_DATA_HOME="$HOME/.local/share"
unset DESKTOPINTEGRATION PHOTOCRAFT_NO_DESKTOP_INTEGRATION
mkdir -p "$HOME"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

# 1. Launch works and passes arguments through.
out="$("$APPDIR/AppRun" open file.png)" || fail "AppRun exited non-zero"
[[ "$out" = "stub: open file.png" ]] || fail "arguments not forwarded: $out"

DESKTOP="$XDG_DATA_HOME/applications/$APP_ID.desktop"
[[ -f "$DESKTOP" ]] || fail "desktop entry not installed"

# 2. Exec rewritten to the AppImage (quoted, spaces intact), TryExec dropped,
#    everything else preserved from the packaged entry.
expected_exec="Exec=\"$(readlink -f "$APPIMAGE")\" %F"
grep -Fxq "$expected_exec" "$DESKTOP" || fail "Exec not rewritten: $(grep '^Exec=' "$DESKTOP")"
grep -q '^TryExec=' "$DESKTOP" && fail "TryExec kept, the entry would be hidden"
grep -Fxq "Icon=$APP_ID" "$DESKTOP" || fail "Icon line lost"
grep -Fxq "X-AppImage-Integrate=false" "$DESKTOP" || fail "X-AppImage-Integrate=false missing (#3623)"
grep -Fxq 'Categories=Graphics;2DGraphics;RasterGraphics;Photography;' "$DESKTOP" || fail "metadata lines lost"
if command -v desktop-file-validate >/dev/null; then
  desktop-file-validate "$DESKTOP" || fail "generated desktop entry is invalid"
fi

# 3. Icons landed under the name the .desktop entry references.
for f in "16x16/apps/$APP_ID.png" "256x256/apps/$APP_ID.png" "scalable/apps/$APP_ID.svg"; do
  [[ -f "$XDG_DATA_HOME/icons/hicolor/$f" ]] || fail "icon missing: $f"
done

# 4. Idempotent: a second launch must not rewrite anything (inode unchanged).
before="$(stat -c %i "$DESKTOP")"
out="$("$APPDIR/AppRun")" || fail "second launch exited non-zero"
[[ "$out" = "stub: " ]] || fail "second launch output: $out"
after="$(stat -c %i "$DESKTOP")"
[[ "$before" = "$after" ]] || fail "desktop entry rewritten although unchanged"

# 5. Moving the AppImage refreshes Exec.
mv "$APPIMAGE" "$TMP/moved.AppImage"
export APPIMAGE="$TMP/moved.AppImage"
"$APPDIR/AppRun" >/dev/null || fail "launch after move exited non-zero"
grep -Fxq "Exec=\"$(readlink -f "$APPIMAGE")\" %F" "$DESKTOP" || fail "Exec not refreshed after move"

# Customized launch options and metadata survive launches and relocation (#2905).
old_path="$(readlink -f "$APPIMAGE")"
sed -i "s|^Exec=.*|Exec=env WAYLAND_DISPLAY= \"$old_path\" --custom %F|; s|^Name=.*|Name=My PhotoCraft|" "$DESKTOP"
printf '\nX-User-Setting=keep me\n' >>"$DESKTOP"
cp "$DESKTOP" "$TMP/custom.desktop"
before="$(stat -c %i "$DESKTOP")"
"$APPDIR/AppRun" >/dev/null
cmp -s "$DESKTOP" "$TMP/custom.desktop" || fail "customized launcher overwritten"
[ "$before" = "$(stat -c %i "$DESKTOP")" ] || fail "custom launcher rewritten although unchanged"
mv "$APPIMAGE" "$TMP/new version.AppImage"
export APPIMAGE="$TMP/new version.AppImage"
"$APPDIR/AppRun" >/dev/null
grep -Fxq "Exec=env WAYLAND_DISPLAY= \"$APPIMAGE\" --custom %F" "$DESKTOP" || fail "custom Exec not relocated"
grep -Fxq 'Name=My PhotoCraft' "$DESKTOP" || fail "custom Name lost"
grep -Fxq 'X-User-Setting=keep me' "$DESKTOP" || fail "custom key lost"

# Packaged updates apply only to untouched keys; user deletions stay deleted.
sed -i '/^Comment=/d' "$DESKTOP"
sed -i 's/^Name=.*/Name=Packaged update/; s/^Categories=.*/Categories=Graphics;/; s/^Comment=.*/Comment=Updated description/' "$APPDIR/usr/share/applications/$APP_ID.desktop"
printf '\nX-New-Packaged-Key=added\n' >>"$APPDIR/usr/share/applications/$APP_ID.desktop"
"$APPDIR/AppRun" >/dev/null
grep -Fxq 'Name=My PhotoCraft' "$DESKTOP" || fail "packaged update erased custom Name"
grep -Fxq 'Categories=Graphics;' "$DESKTOP" || fail "untouched metadata not updated"
grep -q '^Comment=' "$DESKTOP" && fail "user deletion undone"
grep -Fxq 'X-New-Packaged-Key=added' "$DESKTOP" || fail "new packaged key missing"
if command -v desktop-file-validate >/dev/null; then
  desktop-file-validate "$DESKTOP" || fail "customized entry is invalid"
fi

# A replaced executable may mention the old AppImage as an argument: leave it.
sed -i "s|^Exec=.*|Exec=\"/custom/wrapper\" \"$APPIMAGE\" %F|" "$DESKTOP"
expected_wrapper="$(grep '^Exec=' "$DESKTOP")"
mv "$APPIMAGE" "$TMP/another version.AppImage"
export APPIMAGE="$TMP/another version.AppImage"
"$APPDIR/AppRun" >/dev/null
grep -Fxq "$expected_wrapper" "$DESKTOP" || fail "wrapper argument mistaken for managed executable"

# Existing launchers from before snapshots: bootstrap without losing edits.
rm -f "$XDG_DATA_HOME/applications/.$APP_ID.generated"
cp "$DESKTOP" "$TMP/legacy.desktop"
"$APPDIR/AppRun" >/dev/null
cmp -s "$DESKTOP" "$TMP/legacy.desktop" || fail "legacy custom launcher overwritten"
rm -f "$XDG_DATA_HOME/applications/.$APP_ID.generated"
sed -i 's|^Exec=.*|Exec="/custom/wrapper" %F|' "$DESKTOP"
"$APPDIR/AppRun" >/dev/null
grep -Fxq 'Exec="/custom/wrapper" %F' "$DESKTOP" || fail "legacy executable customization overwritten"


# Empty or malformed snapshots must never resurrect user-deleted keys.
for invalid in empty garbage missing-exec; do
  baseline="$XDG_DATA_HOME/applications/.$APP_ID.generated"
  case "$invalid" in
    empty) : >"$baseline" ;;
    garbage) printf 'not a desktop entry\n' >"$baseline" ;;
    missing-exec) printf '[Desktop Entry]\nType=Application\nName=PhotoCraft\n' >"$baseline" ;;
  esac
  sed -i '/^Comment=/d; /^StartupNotify=/d' "$DESKTOP"
  cp "$DESKTOP" "$TMP/before-invalid.desktop"
  "$APPDIR/AppRun" >/dev/null || fail "invalid baseline blocked launch"
  cmp -s "$DESKTOP" "$TMP/before-invalid.desktop" || fail "$invalid baseline changed user launcher"
done

# External integration must preserve customized and unknown legacy entries.
for baseline_kind in valid missing empty; do
  baseline="$XDG_DATA_HOME/applications/.$APP_ID.generated"
  case "$baseline_kind" in
    missing) rm -f "$baseline" ;;
    empty) : >"$baseline" ;;
  esac
  cp "$DESKTOP" "$TMP/before-external.desktop"
  DESKTOPINTEGRATION=AppImageLauncher "$APPDIR/AppRun" >/dev/null
  cmp -s "$DESKTOP" "$TMP/before-external.desktop" || fail "DESKTOPINTEGRATION changed $baseline_kind custom entry"
  ext="$XDG_DATA_HOME/applications/external.desktop"
  printf '[Desktop Entry]\nType=Application\nName=External\nExec="%s" %%F\n' "$APPIMAGE" >"$ext"
  "$APPDIR/AppRun" >/dev/null
  cmp -s "$DESKTOP" "$TMP/before-external.desktop" || fail "external desktop removed $baseline_kind custom entry"
  rm -f "$ext"
done

# Linked launchers and snapshots are externally owned, even when their bytes
# match a generated entry. Check both normal refresh and external deduplication.
for integration in normal external; do
  for linked in launcher dangling-launcher baseline dangling-baseline; do
    rm -rf "$XDG_DATA_HOME"
    "$APPDIR/AppRun" >/dev/null
    baseline="$XDG_DATA_HOME/applications/.$APP_ID.generated"
    target="$TMP/linked-target"
    cp "$DESKTOP" "$target"
    case "$linked" in
      launcher|dangling-launcher) link="$DESKTOP" ;;
      baseline|dangling-baseline) link="$baseline" ;;
    esac
    rm -f "$link"
    case "$linked" in dangling-*) rm -f "$target" ;; esac
    ln -s "$target" "$link"
    [ ! -f "$target" ] || cp "$target" "$TMP/linked-target-before"
    # A packaged metadata change would otherwise cause a symlink to be replaced.
    template="$APPDIR/usr/share/applications/$APP_ID.desktop"
    cp "$template" "$TMP/link-template-before"
    printf '\nX-Link-Refresh=changed\n' >>"$template"
    if [ "$integration" = external ]; then
      ext="$XDG_DATA_HOME/applications/external.desktop"
      printf '[Desktop Entry]\nType=Application\nName=External\nExec="%s" %%F\n' "$APPIMAGE" >"$ext"
      DESKTOPINTEGRATION=AppImageLauncher "$APPDIR/AppRun" >/dev/null
    else
      "$APPDIR/AppRun" >/dev/null
    fi
    [ -L "$link" ] || fail "$integration replaced or removed $linked"
    [ "$(readlink "$link")" = "$target" ] || fail "$integration retargeted $linked"
    case "$linked" in
      dangling-*) [ ! -e "$target" ] || fail "$integration created dangling target" ;;
      *) cmp -s "$target" "$TMP/linked-target-before" || fail "$integration changed linked target" ;;
    esac
    [ -f "$XDG_DATA_HOME/icons/hicolor/256x256/apps/$APP_ID.png" ] || fail "link preservation skipped icons"
    cp "$TMP/link-template-before" "$template"
  done
done

# Reset the stub fixture so the duplicate-removal test starts with an untouched entry.
rm -rf "$XDG_DATA_HOME"
"$APPDIR/AppRun" >/dev/null

# 6. External AppImage integrator (#3623): if another .desktop file in $XDG_DATA_HOME/applications
#    already launches this AppImage (e.g. AppImageLauncher's appimagekit-*.desktop), AppRun removes
#    its own duplicate .desktop entry while keeping the hicolor icons installed.
ext_desktop="$XDG_DATA_HOME/applications/appimagekit-photocraft.desktop"
printf '[Desktop Entry]\nType=Application\nName=PhotoCraft\nExec="%s" %%F\nIcon=%s\n' "$(readlink -f "$APPIMAGE")" "$APP_ID" >"$ext_desktop"
"$APPDIR/AppRun" >/dev/null || fail "launch with external integrator desktop entry exited non-zero"
[[ ! -e "$DESKTOP" ]] || fail "duplicate desktop entry kept alongside external integrator entry"
[[ -f "$ext_desktop" ]] || fail "external integrator desktop entry removed"
[[ -f "$XDG_DATA_HOME/icons/hicolor/256x256/apps/$APP_ID.png" ]] || fail "icons missing with external integrator"
rm -f "$ext_desktop"

# 7. AppImageLauncher exports DESKTOPINTEGRATION even after X-AppImage-Integrate=false
#    made it skip the AppImage. That is not an existing launcher, so AppRun still
#    writes its own entry and a later launch must not delete it (#5534).
rm -rf "$XDG_DATA_HOME"
DESKTOPINTEGRATION=AppImageLauncher "$APPDIR/AppRun" >/dev/null || fail "DESKTOPINTEGRATION launch exited non-zero"
[[ -f "$DESKTOP" ]] || fail "DESKTOPINTEGRATION suppressed the desktop entry (#5534)"
grep -Fxq "Exec=\"$(readlink -f "$APPIMAGE")\" %F" "$DESKTOP" || fail "Exec missing under DESKTOPINTEGRATION"
grep -Fxq "X-AppImage-Integrate=false" "$DESKTOP" || fail "X-AppImage-Integrate=false missing under DESKTOPINTEGRATION"
[[ -f "$XDG_DATA_HOME/icons/hicolor/256x256/apps/$APP_ID.png" ]] || fail "icons not installed under DESKTOPINTEGRATION"
before="$(stat -c %i "$DESKTOP")"
DESKTOPINTEGRATION=AppImageLauncher "$APPDIR/AppRun" >/dev/null || fail "second DESKTOPINTEGRATION launch exited non-zero"
[[ -f "$DESKTOP" ]] || fail "DESKTOPINTEGRATION removed the desktop entry on a later launch"
[[ "$before" = "$(stat -c %i "$DESKTOP")" ]] || fail "desktop entry rewritten under DESKTOPINTEGRATION although unchanged"
# A real integrator entry still wins, including when DESKTOPINTEGRATION is also set (#3623).
ext_desktop="$XDG_DATA_HOME/applications/appimagekit-photocraft.desktop"
printf '[Desktop Entry]\nType=Application\nName=PhotoCraft\nExec="%s" %%F\nIcon=%s\n' "$(readlink -f "$APPIMAGE")" "$APP_ID" >"$ext_desktop"
DESKTOPINTEGRATION=AppImageLauncher "$APPDIR/AppRun" >/dev/null || fail "launch with integrator entry and DESKTOPINTEGRATION exited non-zero"
[[ ! -e "$DESKTOP" ]] || fail "duplicate desktop entry kept when an integrator entry exists"
[[ -f "$ext_desktop" ]] || fail "external integrator desktop entry removed"
[[ -f "$XDG_DATA_HOME/icons/hicolor/256x256/apps/$APP_ID.png" ]] || fail "icons missing when DESKTOPINTEGRATION and an integrator entry coexist"
rm -f "$ext_desktop"

# 8. Opt-out installs nothing.
rm -rf "$XDG_DATA_HOME"
PHOTOCRAFT_NO_DESKTOP_INTEGRATION=1 "$APPDIR/AppRun" >/dev/null || fail "opt-out launch exited non-zero"
[[ ! -e "$XDG_DATA_HOME" ]] || fail "opt-out still integrated"

# 9. An unwritable data dir never blocks the launch.
mkdir -p "$TMP/ro" && chmod a-w "$TMP/ro"
out="$(XDG_DATA_HOME="$TMP/ro/data" "$APPDIR/AppRun" out.txt)" || fail "launch blocked by unwritable data dir"
[[ "$out" = "stub: out.txt" ]] || fail "stub not run against unwritable data dir: $out"
chmod u+w "$TMP/ro"

# 10. No $APPIMAGE (extracted AppDir): launch only, nothing installed.
rm -rf "$XDG_DATA_HOME"
env -u APPIMAGE "$APPDIR/AppRun" >/dev/null || fail "extracted-AppDir launch exited non-zero"
[[ ! -e "$XDG_DATA_HOME" ]] || fail "integrated without $APPIMAGE"

# 11. A path that would need Exec escaping (here a `$`): launch only, nothing installed.
rm -rf "$XDG_DATA_HOME"
odd="$TMP/odd\$(id).AppImage"
: >"$odd"
out="$(APPIMAGE="$odd" "$APPDIR/AppRun" x)" || fail "launch with an odd path exited non-zero"
[[ "$out" = "stub: x" ]] || fail "odd path: $out"
[[ ! -e "$XDG_DATA_HOME" ]] || fail "integrated an AppImage path that needs escaping"

# 12. AM / AppMan (#6392): a photocraft-AM.desktop launcher in the user's applications
#     dir owns the menu entry. AppRun removes its own entry left from an earlier run,
#     writes no second one and still installs the icons.
rm -rf "$XDG_DATA_HOME"
"$APPDIR/AppRun" >/dev/null || fail "launch before AM install exited non-zero"
[[ -f "$DESKTOP" ]] || fail "no desktop entry before AM install"
am_desktop="$XDG_DATA_HOME/applications/photocraft-AM.desktop"
printf '[Desktop Entry]
Type=Application
Name=PhotoCraft
Exec=photocraft %%F
Icon=%s
' "$APP_ID" >"$am_desktop"
"$APPDIR/AppRun" >/dev/null || fail "launch with AM launcher exited non-zero"
[[ ! -e "$DESKTOP" ]] || fail "duplicate desktop entry kept alongside AM launcher (#6392)"
[[ -f "$am_desktop" ]] || fail "AM launcher was removed"
[[ -f "$XDG_DATA_HOME/icons/hicolor/256x256/apps/$APP_ID.png" ]] || fail "icons missing with AM launcher"

# 13. Once AM removes its launcher, AppRun writes its own entry again.
rm -f "$am_desktop"
"$APPDIR/AppRun" >/dev/null || fail "launch after AM removal exited non-zero"
[[ -f "$DESKTOP" ]] || fail "no desktop entry after the AM launcher was removed"

# 12. `--cli` runs the bundled CLI (MCP clients start `--cli mcp`), without integrating.
rm -rf "$XDG_DATA_HOME"
out="$("$APPDIR/AppRun" --cli mcp --headless)" || fail "--cli exited non-zero"
[ "$out" = "cli stub: mcp --headless" ] || fail "--cli output: $out"
[ ! -e "$XDG_DATA_HOME" ] || fail "--cli integrated the desktop entry"

echo "AppRun desktop integration ok"
