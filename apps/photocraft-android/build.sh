#!/usr/bin/env bash
# Build PhotoCraft's Android APK (arm64-v8a): the Rust library with cargo-ndk, then the APK with
# Gradle. Needs ANDROID_HOME (SDK with platform 35 and build-tools), ANDROID_NDK_HOME (r27+),
# JDK 17, `rustup target add aarch64-linux-android` and `cargo install cargo-ndk`.
#
#   apps/photocraft-android/build.sh            # release APK
#   PROFILE=dev apps/photocraft-android/build.sh  # unoptimised library, faster to build
#   RELEASE_BUILD=3 apps/photocraft-android/build.sh  # the 3rd Android release of this version
#
# The APK lands in target/android/photocraft-<version>[-<build>]-arm64-v8a.apk. A release of it
# is tagged android-v<version>[-<build>] with the APK attached: installed copies offer to update
# to it (android/app/src/main/java/.../Updater.java).
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
PROFILE="${PROFILE:-release}"
API="${ANDROID_API:-26}"

: "${ANDROID_HOME:?set ANDROID_HOME to the Android SDK}"
: "${ANDROID_NDK_HOME:?set ANDROID_NDK_HOME to the Android NDK (r27 or newer)}"
command -v cargo-ndk >/dev/null || { echo "error: cargo-ndk not found (cargo install cargo-ndk)" >&2; exit 1; }

# Interface translations: a storytold/craft-i18n checkout at the SHA in craft-i18n.rev. Without
# one the app is English only, so a checkout beside the repo (../craft-i18n) is used when
# CRAFT_I18N_DIR isn't set, and a release build stops if there is none.
if [[ -z "${CRAFT_I18N_DIR:-}" && -d "$ROOT/../craft-i18n" ]]; then
    CRAFT_I18N_DIR="$(cd "$ROOT/../craft-i18n" && pwd)"
    # Windows: cargo's build script needs a C:\ path, not Git Bash's /c/.
    if command -v cygpath >/dev/null; then CRAFT_I18N_DIR="$(cygpath -w "$CRAFT_I18N_DIR")"; fi
    export CRAFT_I18N_DIR
fi
if [[ "${RELEASE_BUILD:-0}" != 0 ]]; then export CRAFT_I18N_REQUIRED=1; fi
if [[ -n "${CRAFT_I18N_DIR:-}" ]]; then
    PIN="$(tr -d '[:space:]' < "$ROOT/craft-i18n.rev")"
    HAVE="$(git -C "$CRAFT_I18N_DIR" rev-parse HEAD 2>/dev/null || true)"
    if [[ "$HAVE" != "$PIN" ]]; then echo "warning: $CRAFT_I18N_DIR is at ${HAVE:-?}, craft-i18n.rev pins $PIN (git -C \"$CRAFT_I18N_DIR\" fetch && git -C \"$CRAFT_I18N_DIR\" checkout $PIN)" >&2; fi
fi

FEATURE_ARGS=()
if [[ -n "${FEATURES:-}" ]]; then FEATURE_ARGS=(--features "$FEATURES"); fi

# Keep this machine's folders (and the user name in them) out of the library: rustc records
# source paths for panic messages. Map the Cargo home, the workspace and the target directory to
# neutral prefixes (both spellings on Windows, where rustc sees `C:\...` paths).
remap() {
    printf ' --remap-path-prefix=%s=%s' "$1" "$2"
    if command -v cygpath >/dev/null; then printf ' --remap-path-prefix=%s=%s' "$(cygpath -w "$1")" "$2"; fi
}
REMAP="$(remap "${CARGO_HOME:-$HOME/.cargo}" /cargo)$(remap "$ROOT" /photocraft)$(remap "${CARGO_TARGET_DIR:-$ROOT/target}" /target)"
if [[ -n "${CRAFT_I18N_DIR:-}" ]]; then REMAP="$REMAP$(remap "$CRAFT_I18N_DIR" /craft-i18n)"; fi
export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="${CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS:-}$REMAP"

cd "$ROOT"
cargo ndk -t arm64-v8a -P "$API" -o "$HERE/android/app/src/main/jniLibs" \
    build -p photocraft-android --profile "$PROFILE" "${FEATURE_ARGS[@]}"

cd "$HERE/android"
if [[ "${OS:-}" == "Windows_NT" ]]; then ./gradlew.bat --no-daemon assembleRelease; else ./gradlew --no-daemon assembleRelease; fi

VERSION="${APP_VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)}"
if [[ "${RELEASE_BUILD:-0}" != 0 ]]; then VERSION="$VERSION-$RELEASE_BUILD"; fi
OUT="${CARGO_TARGET_DIR:-$ROOT/target}/android"
mkdir -p "$OUT"
cp app/build/outputs/apk/release/app-release.apk "$OUT/photocraft-$VERSION-arm64-v8a.apk"
echo "APK: $OUT/photocraft-$VERSION-arm64-v8a.apk"
