#!/usr/bin/env bash
# Build Rust library for Android.
# Run from repo root: ./build-android-rust.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ANDROID_API_LEVEL="${ANDROID_API_LEVEL:-21}"

# Colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

log() { echo -e "${GREEN}[android-rust]${NC} $*"; }
warn() { echo -e "${YELLOW}[android-rust]${NC} $*"; }
err() { echo -e "${RED}[android-rust]${NC} $*" >&2; }

log "Checking Android Rust targets..."
rustup target list --installed | grep -q "aarch64-linux-android" || {
    log "Installing Android targets..."
    rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android i686-linux-android
}

# Locate NDK
if [[ -z "${ANDROID_NDK_HOME:-}" ]]; then
    warn "ANDROID_NDK_HOME not set; trying Android Studio default locations"
    for p in "$HOME/Android/Sdk/ndk"/* "$HOME/Android/Sdk/ndk-bundle" "/opt/android-ndk" "/usr/lib/android-ndk"; do
        if [[ -d "$p/toolchains/llvm/prebuilt/linux-x86_64/bin" ]]; then
            export ANDROID_NDK_HOME="$p"
        fi
    done
fi

if [[ -z "${ANDROID_NDK_HOME:-}" || ! -d "$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin" ]]; then
    err "ANDROID_NDK_HOME is not set to a valid NDK. Example: export ANDROID_NDK_HOME=~/Android/Sdk/ndk/28.2.13676358"
    exit 1
fi

TOOLCHAIN="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin"
AR_BIN="$TOOLCHAIN/llvm-ar"

if [[ ! -x "$AR_BIN" ]]; then
    err "llvm-ar not found at $AR_BIN"
    exit 1
fi

cd "$REPO_ROOT"

build_target() {
    local target="$1"
    local abi="$2"
    local clang="$3"
    local cc_var="$4"
    local linker_var="$5"

    local cc_path="$TOOLCHAIN/$clang"
    if [[ ! -x "$cc_path" ]]; then
        err "Missing Android clang: $cc_path"
        exit 1
    fi

    log "Building $target ($abi) with $clang..."

    # openssl-sys/cc-rs look at CC/AR and target-specific CC_* variables.
    export CC="$cc_path"
    export AR="$AR_BIN"
    export "${cc_var}=$cc_path"
    export "${linker_var}=$cc_path"

    cargo build --lib --target "$target" --release

    local out_dir="gui/android/app/src/main/jniLibs/$abi"
    mkdir -p "$out_dir"
    cp "target/$target/release/libsideband.so" "$out_dir/libsideband.so"
    log "Copied $out_dir/libsideband.so"
}

build_target "aarch64-linux-android" "arm64-v8a" "aarch64-linux-android${ANDROID_API_LEVEL}-clang" \
    "CC_aarch64_linux_android" "CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER"
build_target "armv7-linux-androideabi" "armeabi-v7a" "armv7a-linux-androideabi${ANDROID_API_LEVEL}-clang" \
    "CC_armv7_linux_androideabi" "CARGO_TARGET_ARMV7_LINUX_ANDROIDEABI_LINKER"
build_target "x86_64-linux-android" "x86_64" "x86_64-linux-android${ANDROID_API_LEVEL}-clang" \
    "CC_x86_64_linux_android" "CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER"
build_target "i686-linux-android" "x86" "i686-linux-android${ANDROID_API_LEVEL}-clang" \
    "CC_i686_linux_android" "CARGO_TARGET_I686_LINUX_ANDROID_LINKER"

log "Android Rust builds complete"
log "Libraries copied to gui/android/app/src/main/jniLibs/<abi>/libsideband.so"
