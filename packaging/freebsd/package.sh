#!/usr/bin/env bash
# Build and package PhotoCraft for FreeBSD:
#
#   $DIST/photocraft-<version>-freebsd-<arch>.tar.gz   a /usr/local-style tree (arch x86_64 or aarch64):
#       photocraft-<version>-freebsd-<arch>/{bin, man/man1, share/applications, share/icons,
#       share/mime, share/metainfo, share/doc/photocraft, share/{bash-completion,zsh,fish}}
#
# Install by copying the tree's contents into /usr/local:
#   tar -xzf photocraft-<version>-freebsd-x86_64.tar.gz --strip-components 1 -C /usr/local
#
# Usage: packaging/freebsd/package.sh [--skip-build] [--dry-run]
#   --dry-run   stage the tree from stub binaries and list it, without building (works on any OS)
#
# Needs: bash, cargo, and the packages installed in .github/workflows/freebsd.yml.
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
LINUX="$ROOT/packaging/linux"
APP_ID=ai.storyteller.photocraft

SKIP_BUILD=0
DRY_RUN=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1; shift ;;
    --dry-run) DRY_RUN=1; SKIP_BUILD=1; shift ;;
    -h | --help) sed -n '2,15p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

# FreeBSD's uname says amd64; release file names use x86_64 like the Linux and Windows ones.
# CARGO_BUILD_TARGET is set when the release VM cross-compiles aarch64 from an x86_64 guest.
MACHINE="$(uname -m)"
case "$MACHINE" in
  amd64 | x86_64) HOST_ARCH=x86_64 ;;
  arm64 | aarch64) HOST_ARCH=aarch64 ;;
  *) echo "unsupported architecture $MACHINE" >&2; exit 2 ;;
esac
ARCH="$HOST_ARCH"
if [[ -n "${CARGO_BUILD_TARGET:-}" ]]; then
  # cargo's host-tuple alias means the host triple; resolve it so the binary dir matches cargo's output.
  if [[ "$CARGO_BUILD_TARGET" = host-tuple ]]; then
    CARGO_BUILD_TARGET="$(rustc -vV | sed -n 's/^host: //p')"
    export CARGO_BUILD_TARGET
  fi
  case "$CARGO_BUILD_TARGET" in
    aarch64-unknown-freebsd) ARCH=aarch64 ;;
    x86_64-unknown-freebsd) ARCH=x86_64 ;;
    *)
      echo "error: unsupported CARGO_BUILD_TARGET $CARGO_BUILD_TARGET (use x86_64-unknown-freebsd or aarch64-unknown-freebsd)" >&2
      exit 2
      ;;
  esac
fi
# A cross-built binary cannot run on this host, so the --version check and the man-page example are skipped.
CROSS=0
if [[ "$ARCH" != "$HOST_ARCH" ]]; then CROSS=1; fi
if [[ "$DRY_RUN" = 0 ]] && [[ "$(uname -s)" != FreeBSD ]]; then
  echo "error: build on FreeBSD (or pass --dry-run to check the tree layout)" >&2
  exit 2
fi
BASENAME="photocraft-$VERSION-freebsd-$ARCH"

echo "==> PhotoCraft $VERSION for FreeBSD $ARCH"

# The release VM has 12 GB; full parallelism on the biggest crates runs it out of memory.
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"
# The man-and-completions example writes photocraft-cli's man pages and shell completions below. It
# is built in its own cargo run: built with the binaries, its dev-dependencies' features would be
# unified into them. cargo reads CARGO_BUILD_TARGET itself, so cross builds need no --target flag.
if [[ "$SKIP_BUILD" = 0 ]]; then
  (cd "$ROOT" && cargo build --release --locked -p photocraft -p photocraft-cli --features heif --bins)
  if [[ "$CROSS" = 0 ]]; then
    (cd "$ROOT" && cargo build --release --locked -p photocraft-cli --features heif --example man-and-completions)
  fi
fi
WORK="$CARGO_TARGET_DIR/freebsd-package"
STAGE="$WORK/$BASENAME"
rm -rf "$WORK"
mkdir -p "$WORK"

BIN="$CARGO_TARGET_DIR/release"
if [[ -n "${CARGO_BUILD_TARGET:-}" ]]; then BIN="$CARGO_TARGET_DIR/$CARGO_BUILD_TARGET/release"; fi
OUT_DIR="$DIST"
if [[ "$DRY_RUN" = 1 ]]; then
  # Stub binaries, and the tarball stays in the work dir so dist/ only ever holds real builds.
  BIN="$WORK/stub-bin"
  OUT_DIR="$WORK"
  mkdir -p "$BIN"
  for b in photocraft photocraft-cli; do
    printf '#!/bin/sh\necho "%s %s (dry-run stub)"\n' "$b" "$VERSION" >"$BIN/$b"
    chmod 755 "$BIN/$b"
  done
fi

# ---- stage a /usr/local-style tree --------------------------------------------------------------
# FreeBSD's install(1) has no -D: create the directories first.
mkdir -p "$STAGE/bin" "$STAGE/share/applications" "$STAGE/share/mime/packages" \
  "$STAGE/share/metainfo" "$STAGE/share/icons" "$STAGE/share/doc/photocraft"
install -m 755 "$BIN/photocraft" "$BIN/photocraft-cli" "$STAGE/bin/"
strip "$STAGE/bin/photocraft" "$STAGE/bin/photocraft-cli" 2>/dev/null || true
# The desktop entry, MIME type, metainfo and icons are the freedesktop files Linux ships.
install -m 644 "$LINUX/$APP_ID.desktop" "$STAGE/share/applications/$APP_ID.desktop"
install -m 644 "$LINUX/$APP_ID.mime.xml" "$STAGE/share/mime/packages/$APP_ID.xml"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@DATE@/$PHOTOCRAFT_BUILD_DATE/g" \
  "$LINUX/$APP_ID.metainfo.xml.in" >"$STAGE/share/metainfo/$APP_ID.metainfo.xml"
cp -R "$ROOT/assets/app-icon/hicolor" "$STAGE/share/icons/"
copy_docs "$STAGE/share/doc/photocraft"
for f in NOTICE ATTRIBUTION.md; do
  if [[ -f "$ROOT/$f" ]]; then install -m 644 "$ROOT/$f" "$STAGE/share/doc/photocraft/"; fi
done
# photocraft-cli's man pages and shell completions, from its clap definition, written by the
# example built above. A dry run builds nothing, so it leaves them out, as a --skip-build run whose
# prebuilt binaries came without the example does.
CLI_DOCS="$BIN/examples/man-and-completions"
if [[ "$DRY_RUN" = 0 ]] && [[ "$CROSS" = 1 ]]; then
  warn "cross-building $ARCH, so photocraft-cli's man pages and completions are left out"
elif [[ "$DRY_RUN" = 0 ]] && [[ ! -x "$CLI_DOCS" ]]; then
  warn "$CLI_DOCS is not built, so photocraft-cli's man pages and completions are left out"
elif [[ "$DRY_RUN" = 0 ]]; then
  "$CLI_DOCS" "$WORK/cli-docs"
  mkdir -p "$STAGE/man/man1" "$STAGE/share/bash-completion/completions" \
    "$STAGE/share/zsh/site-functions" "$STAGE/share/fish/vendor_completions.d"
  install -m 644 "$WORK"/cli-docs/*.1 "$STAGE/man/man1/"
  install -m 644 "$WORK/cli-docs/photocraft-cli.bash" "$STAGE/share/bash-completion/completions/photocraft-cli"
  install -m 644 "$WORK/cli-docs/_photocraft-cli" "$STAGE/share/zsh/site-functions/_photocraft-cli"
  install -m 644 "$WORK/cli-docs/photocraft-cli.fish" "$STAGE/share/fish/vendor_completions.d/photocraft-cli.fish"
fi

for f in bin/photocraft bin/photocraft-cli "share/applications/$APP_ID.desktop" \
  "share/icons/hicolor/256x256/apps/$APP_ID.png" "share/icons/hicolor/scalable/apps/$APP_ID.svg" \
  share/doc/photocraft/LICENSE-MIT share/doc/photocraft/LICENSE-APACHE; do
  if [[ ! -e "$STAGE/$f" ]]; then echo "error: $f is missing from the package" >&2; exit 1; fi
done

# ---- .tar.gz ------------------------------------------------------------------------------------
mkdir -p "$OUT_DIR"
tar -C "$WORK" -czf "$OUT_DIR/$BASENAME.tar.gz" "$BASENAME"
echo "wrote $OUT_DIR/$BASENAME.tar.gz"

if [[ -n "${CARGO_BUILD_TARGET:-}" ]]; then
  echo "==> skipping photocraft-cli --version: CARGO_BUILD_TARGET is set, so the binary is not run on this host"
else
  "$STAGE/bin/photocraft-cli" --version
fi
if [[ "$DRY_RUN" = 1 ]]; then
  echo "==> dry run: tarball contents"
  tar -tzvf "$OUT_DIR/$BASENAME.tar.gz"
fi
echo "==> done"
