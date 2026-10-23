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
