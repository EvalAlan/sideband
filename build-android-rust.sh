#!/usr/bin/env bash
# Build Rust library for Android (flutter_rust_bridge)
# Run from repo root: ./build-android-rust.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

log() { echo -e "${GREEN}[android-rust]${NC} $*"; }
warn() { echo -e "${YELLOW}[android-rust]${NC} $*"; }
err() { echo -e "${RED}[android-rust]${NC} $*" >&2; }

# Check rustup targets
log "Checking Android targets..."
rustup target list --installed | grep -q "aarch64-linux-android" || {
    log "Installing Android targets..."
    rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android i686-linux-android
}

# Check NDK
if [[ -z "${ANDROID_NDK_HOME:-}" && -z "${ANDROID_SDK_ROOT:-}" ]]; then
    warn "ANDROID_NDK_HOME or ANDROID_SDK_ROOT not set"
    warn "Assuming Android Studio installed NDK at default location"
    # Try to find NDK
    NDK_PATHS=(
        "$HOME/Android/Sdk/ndk-bundle"
        "$HOME/Android/Sdk/ndk"
        "/opt/android-ndk"
        "/usr/lib/android-ndk"
    )
    for p in "${NDK_PATHS[@]}"; do
        if [[ -d "$p" ]]; then
            export ANDROID_NDK_HOME="$p"
            log "Found NDK at $ANDROID_NDK_HOME"
            break
        fi
    done
fi

# Build for each Android architecture
TARGETS=(
    "aarch64-linux-android"
    "armv7-linux-androideabi"
    "x86_64-linux-android"
    "i686-linux-android"
)

cd "$REPO_ROOT"

for target in "${TARGETS[@]}"; do
    log "Building for $target..."
    cargo build --lib --target "$target" --release 2>&1 | tail -5
done

log "Android Rust builds complete"
log "Libraries at: target/<target>/release/libsideband.so"