#!/usr/bin/env bash
# AppImage CLI selection, argument boundaries, stdio and exit status (#3385, #3176).
# No build or AppImage mount required; the bundled binaries are stand-ins.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
test_dir="$(mktemp -d)"
trap 'rm -rf "$test_dir"' EXIT
app_dir="$test_dir/PhotoCraft test.AppDir"
mkdir -p "$app_dir/usr/bin" "$app_dir/usr/share/applications"
cp "$HERE/AppRun" "$app_dir/AppRun"
printf '[Desktop Entry]\nType=Application\nName=PhotoCraft\nExec=photocraft %%F\n' \
  >"$app_dir/usr/share/applications/ai.storyteller.photocraft.desktop"
cat >"$app_dir/usr/bin/photocraft" <<'EOF'
#!/bin/sh
printf 'gui\n'
for arg do printf '<%s>\n' "$arg"; done
EOF
cat >"$app_dir/usr/bin/photocraft-cli" <<'EOF'
#!/bin/sh
case "${1:-}" in
  mcp)
    IFS= read -r request
    printf '%s\n' "$request"
    printf 'cli stderr\n' >&2
    ;;
  --exit-37) exit 37 ;;
  *)
    printf 'cli\n'
    for arg do printf '<%s>\n' "$arg"; done
    ;;
esac
EOF
chmod +x "$app_dir/AppRun" "$app_dir/usr/bin/photocraft" "$app_dir/usr/bin/photocraft-cli"
export APPIMAGE="$test_dir/PhotoCraft test.AppImage"
: >"$APPIMAGE"
export ARGV0="$APPIMAGE"
export XDG_DATA_HOME="$test_dir/data"

fail() { echo "FAIL: $*" >&2; exit 1; }

out="$("$app_dir/AppRun" --cli convert 'input file.png' '' '*.psd' --out 'output file.psd')"
expected=$'cli\n<convert>\n<input file.png>\n<>\n<*.psd>\n<--out>\n<output file.psd>'
[ "$out" = "$expected" ] || fail "CLI arguments changed: $out"
[ "$("$app_dir/AppRun" --cli)" = cli ] || fail "empty CLI invocation"

# The AppImage runtime exposes the original symlink name through ARGV0.
for argv0 in photocraft-cli ./photocraft-cli "$test_dir/with spaces/photocraft-cli"; do
  out="$(ARGV0="$argv0" "$app_dir/AppRun" --version)"
  [ "$out" = $'cli\n<--version>' ] || fail "CLI symlink not selected: $argv0"
done
out="$(ARGV0=photocraft-cli "$app_dir/AppRun" --cli --version)"
[ "$out" = $'cli\n<--version>' ] || fail "CLI selector not removed with a symlink"

request='{"jsonrpc":"2.0","id":1,"method":"initialize"}'
out="$(printf '%s\n' "$request" | "$app_dir/AppRun" --cli mcp 2>"$test_dir/stderr")"
[ "$out" = "$request" ] || fail "MCP stdin/stdout changed: $out"
[ "$(cat "$test_dir/stderr")" = 'cli stderr' ] || fail "CLI stderr lost"
status=0
"$app_dir/AppRun" --cli --exit-37 || status=$?
[ "$status" = 37 ] || fail "CLI exit status changed: $status"
[ ! -e "$XDG_DATA_HOME" ] || fail "CLI performed desktop integration"

# GUI invocations still forward all arguments; only a leading --cli is a selector.
out="$("$app_dir/AppRun" 'input file.png' --cli)"
[ "$out" = $'gui\n<input file.png>\n<--cli>' ] || fail "GUI arguments changed: $out"
[ -f "$XDG_DATA_HOME/applications/ai.storyteller.photocraft.desktop" ] || fail "GUI integration lost"
out="$(ARGV0=another-photocraft-cli "$app_dir/AppRun" mcp)"
[ "$out" = $'gui\n<mcp>' ] || fail "an unrelated filename selected CLI"

# A broken package must fail visibly, rather than silently opening the GUI.
rm "$app_dir/usr/bin/photocraft-cli"
status=0
"$app_dir/AppRun" --cli --version >"$test_dir/missing-out" 2>"$test_dir/missing-err" || status=$?
[ "$status" != 0 ] || fail "missing CLI succeeded"
[ ! -s "$test_dir/missing-out" ] || fail "missing CLI launched the GUI"
[ -s "$test_dir/missing-err" ] || fail "missing CLI had no diagnostic"

echo "AppRun CLI selection ok"
