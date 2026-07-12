#!/usr/bin/env bash
#
# Unified build script for all Sideband clients.
#
#   ./build.sh                 # build everything (tui + desktop + android)
#   ./build.sh all
#   ./build.sh tui             # the Rust CLI/TUI binary -> target/release/sideband
#   ./build.sh desktop         # the Linux desktop GUI as an AppImage -> dist/
#   ./build.sh android         # Rust jniLibs for all ABIs + the release APK
#   ./build.sh tui android     # any combination of targets
#
# Env:
#   ANDROID_NDK_HOME   NDK path (auto-detected from ~/Android/Sdk/ndk/* if unset)
#   ANDROID_API_LEVEL  Android minSdk for the native libs (default 21)
#
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GUI_DIR="${REPO_ROOT}/gui"
cd "${REPO_ROOT}"

GREEN='\033[0;32m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; NC='\033[0m'
log()  { echo -e "${GREEN}[build]${NC} $*"; }
warn() { echo -e "${YELLOW}[build]${NC} $*"; }
err()  { echo -e "${RED}[build]${NC} $*" >&2; }

usage() {
  sed -n '2,20p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
  exit "${1:-0}"
}

# --- shared helpers ----------------------------------------------------------

resolve_flutter() {
  local f="${GUI_DIR}/.tools/flutter/bin/flutter"
  [[ -x "$f" ]] || f="$(command -v flutter 2>/dev/null || true)"
  [[ -x "$f" ]] || f="${HOME}/flutter/bin/flutter"
  if [[ ! -x "$f" ]]; then err "Flutter not found (install it or add to PATH)"; exit 1; fi
  FLUTTER="$f"
}

resolve_ndk() {
  if [[ -z "${ANDROID_NDK_HOME:-}" ]]; then
    warn "ANDROID_NDK_HOME not set; probing Android Studio default locations"
    for p in "$HOME/Android/Sdk/ndk"/* "$HOME/Android/Sdk/ndk-bundle" "/opt/android-ndk" "/usr/lib/android-ndk"; do
      [[ -d "$p/toolchains/llvm/prebuilt/linux-x86_64/bin" ]] && export ANDROID_NDK_HOME="$p"
    done
  fi
  if [[ -z "${ANDROID_NDK_HOME:-}" || ! -d "$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin" ]]; then
    err "ANDROID_NDK_HOME is not a valid NDK. e.g. export ANDROID_NDK_HOME=~/Android/Sdk/ndk/28.2.13676358"
    exit 1
  fi
  TOOLCHAIN="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin"
  AR_BIN="$TOOLCHAIN/llvm-ar"
  [[ -x "$AR_BIN" ]] || { err "llvm-ar not found at $AR_BIN"; exit 1; }
}

# --- tui ---------------------------------------------------------------------

build_tui() {
  log "Building Rust CLI/TUI (release)..."
  cargo build --release --bin sideband
  log "Built ${REPO_ROOT}/target/release/sideband"
}

# --- android -----------------------------------------------------------------

_android_build_lib() {
  local target="$1" abi="$2" clang="$3" cc_var="$4" linker_var="$5"
  local cc_path="$TOOLCHAIN/$clang"
  [[ -x "$cc_path" ]] || { err "Missing Android clang: $cc_path"; exit 1; }
  log "  $target ($abi)"
  # openssl-sys/cc-rs read CC/AR and the target-specific CC_*/linker vars.
  # export supports dynamic variable names ("$cc_var"=...); re-set each ABI.
  export CC="$cc_path" AR="$AR_BIN"
  export "${cc_var}=$cc_path"
  export "${linker_var}=$cc_path"
  cargo build --lib --target "$target" --release
  local out="gui/android/app/src/main/jniLibs/$abi"
  mkdir -p "$out"
  cp "target/$target/release/libsideband.so" "$out/libsideband.so"
}

build_android() {
  local api="${ANDROID_API_LEVEL:-21}"
  resolve_ndk
  resolve_flutter

  log "Ensuring Rust Android targets are installed..."
  rustup target list --installed | grep -q "aarch64-linux-android" || \
    rustup target add aarch64-linux-android armv7-linux-androideabi \
                      x86_64-linux-android i686-linux-android

  log "Building Rust jniLibs for all ABIs..."
  _android_build_lib aarch64-linux-android arm64-v8a "aarch64-linux-android${api}-clang" \
      CC_aarch64_linux_android CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER
  _android_build_lib armv7-linux-androideabi armeabi-v7a "armv7a-linux-androideabi${api}-clang" \
      CC_armv7_linux_androideabi CARGO_TARGET_ARMV7_LINUX_ANDROIDEABI_LINKER
  _android_build_lib x86_64-linux-android x86_64 "x86_64-linux-android${api}-clang" \
      CC_x86_64_linux_android CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER
  _android_build_lib i686-linux-android x86 "i686-linux-android${api}-clang" \
      CC_i686_linux_android CARGO_TARGET_I686_LINUX_ANDROID_LINKER

  log "Building the release APK..."
  ( cd "${GUI_DIR}" && "$FLUTTER" build apk --release )
  log "Built ${GUI_DIR}/build/app/outputs/flutter-apk/app-release.apk"
}

# --- desktop (Linux AppImage) ------------------------------------------------

build_desktop() {
  resolve_flutter
  local arch; arch="$(uname -m)"
  local flutter_arch appimage_arch
  case "$arch" in
    x86_64|amd64)  flutter_arch="x64";   appimage_arch="x86_64" ;;
    aarch64|arm64) flutter_arch="arm64"; appimage_arch="aarch64" ;;
    *) err "Unsupported arch: $arch"; exit 1 ;;
  esac

  local build_dir="${GUI_DIR}/build/linux/${flutter_arch}/release/bundle"
  local appdir="${GUI_DIR}/AppDir"
  local out_dir="${REPO_ROOT}/dist"

  build_tui

  log "Building Flutter Linux release..."
  ( cd "${GUI_DIR}" && "$FLUTTER" build linux --release )
  [[ -x "${build_dir}/sideband_gui" ]] || { err "Flutter Linux build failed"; exit 1; }

  mkdir -p "${HOME}/.local/bin"
  local appimagetool="${HOME}/.local/bin/appimagetool-${appimage_arch}.AppImage"
  if [[ ! -x "${appimagetool}" ]]; then
    log "Downloading appimagetool..."
    wget -q -O "${appimagetool}" \
      "https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-${appimage_arch}.AppImage"
    chmod +x "${appimagetool}"
  fi
  export PATH="${HOME}/.local/bin:${PATH}"

  local linuxdeploy="${HOME}/.local/bin/linuxdeploy-${appimage_arch}.AppImage"
  if [[ ! -x "${linuxdeploy}" ]]; then
    log "Downloading linuxdeploy..."
    wget -q -O "${linuxdeploy}" \
      "https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-${appimage_arch}.AppImage"
    chmod +x "${linuxdeploy}"
  fi

  # linuxdeploy's bundled strip is too old for .relr.dyn on newer distros;
  # repack it with a no-op strip.
  local linuxdeploy_patched="${HOME}/.local/bin/linuxdeploy-patched-${appimage_arch}.AppImage"
  if [[ ! -x "${linuxdeploy_patched}" ]]; then
    log "Patching linuxdeploy bundled strip..."
    local extract; extract="$(mktemp -d)"
    ( cd "${extract}" && "${linuxdeploy}" --appimage-extract >/dev/null 2>&1
      [[ -f squashfs-root/usr/bin/strip ]] && cp /usr/bin/true squashfs-root/usr/bin/strip
      APPIMAGE_EXTRACT_AND_RUN=1 "${appimagetool}" squashfs-root "${linuxdeploy_patched}" )
    chmod +x "${linuxdeploy_patched}"
    rm -rf "${extract}"
  fi
  linuxdeploy="${linuxdeploy_patched}"

  local icon_source="${GUI_DIR}/assets/icon_512x512.png"
  local icon_dir="${appdir}/usr/share/icons/hicolor"
  [[ -f "${icon_source}" ]] || { err "Icon source not found: ${icon_source}"; exit 1; }

  log "Generating icons..."
  rm -rf "${appdir}"
  for size in 16 32 48 64 128 256 512; do
    mkdir -p "${icon_dir}/${size}x${size}/apps"
    local dest="${icon_dir}/${size}x${size}/apps/com.evalalan.sideband.png"
    if   command -v magick  >/dev/null 2>&1; then magick  "${icon_source}" -resize "${size}x${size}" "${dest}"
    elif command -v convert >/dev/null 2>&1; then convert "${icon_source}" -resize "${size}x${size}" "${dest}"
    else warn "No ImageMagick/convert — skipping icon generation"; break
    fi
  done

  log "Creating AppDir..."
  mkdir -p "${appdir}/usr/bin" "${appdir}/usr/share/applications" "${appdir}/usr/share/metainfo"
  cp -r "${build_dir}/." "${appdir}/usr/bin/"
  mv "${appdir}/usr/bin/sideband_gui" "${appdir}/usr/bin/sideband_gui.bin"
  cp "${REPO_ROOT}/target/release/sideband" "${appdir}/usr/bin/sideband"
  chmod +x "${appdir}/usr/bin/sideband"

  cat > "${appdir}/usr/bin/sideband_gui" <<'WRAPPER'
#!/usr/bin/env bash
HERE="$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")"
ROOT="$(dirname "$(dirname "${HERE}")")"
export LD_LIBRARY_PATH="${ROOT}/usr/lib:${ROOT}/usr/bin/lib:${LD_LIBRARY_PATH:-}"
export XDG_DATA_DIRS="${ROOT}/usr/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
export FONTCONFIG_PATH="${ROOT}/etc/fonts:${FONTCONFIG_PATH:-}"
exec "${HERE}/sideband_gui.bin" "$@"
WRAPPER
  chmod +x "${appdir}/usr/bin/sideband_gui"

  local desktop_id="com.evalalan.sideband.desktop"
  cp "${GUI_DIR}/linux/sideband_gui.desktop" "${appdir}/usr/share/applications/${desktop_id}"

  # Prefer the bundled color emoji font over monochrome system fallbacks.
  mkdir -p "${appdir}/etc/fonts"
  cat > "${appdir}/etc/fonts/local.conf" <<'FONTCONF'
<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
<fontconfig>
  <alias><family>SidebandEmoji</family><default><family>Noto Color Emoji</family></default></alias>
  <alias><family>Noto Color Emoji</family><prefer><family>SidebandEmoji</family></prefer></alias>
</fontconfig>
FONTCONF

  cat > "${appdir}/usr/share/metainfo/sideband_gui.metainfo.xml" <<'XML'
<?xml version="1.0" encoding="UTF-8"?>
<component type="desktop-application">
  <id>com.evalalan.sideband</id>
  <name>Sideband</name>
  <summary>Tor-based decentralized messenger</summary>
  <description><p>Peer-to-peer messenger using Tor onion services.</p></description>
  <project_license>MIT</project_license>
  <url type="homepage">https://github.com/EvalAlan/sideband</url>
  <categories><category>Network</category><category>InstantMessaging</category></categories>
  <launchable type="desktop-id">com.evalalan.sideband.desktop</launchable>
</component>
XML

  log "Deploying dependencies..."
  APPIMAGE_EXTRACT_AND_RUN=1 "${linuxdeploy}" \
    --appdir "${appdir}" \
    --executable "${appdir}/usr/bin/sideband_gui.bin" \
    --executable "${appdir}/usr/bin/sideband" \
    --desktop-file "${appdir}/usr/share/applications/${desktop_id}" \
    --icon-file "${icon_dir}/256x256/apps/com.evalalan.sideband.png"

  if [[ -f "${appdir}/usr/lib/libtray_manager_plugin.so" ]] && \
     [[ ! -e "${appdir}/usr/lib/libayatana-appindicator3.so.1" ]] && \
     [[ ! -e "${appdir}/usr/lib/libappindicator3.so.1" ]]; then
    err "AppImage missing bundled AppIndicator library required by tray_manager"; exit 1
  fi

  # Keep the AppDir-root desktop/icon identity in sync so KDE/GNOME resolve the
  # Sideband logo instead of a generic fallback.
  rm -f "${appdir}/.DirIcon"
  cp "${icon_dir}/256x256/apps/com.evalalan.sideband.png" "${appdir}/com.evalalan.sideband.png"
  cp "${icon_dir}/256x256/apps/com.evalalan.sideband.png" "${appdir}/.DirIcon"

  cat > "${appdir}/AppRun" <<'APPRUN'
#!/usr/bin/env bash
set -euo pipefail
ROOT="${APPDIR:-$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")}"
export LD_LIBRARY_PATH="${ROOT}/usr/lib:${ROOT}/usr/bin/lib:${LD_LIBRARY_PATH:-}"
export XDG_DATA_DIRS="${ROOT}/usr/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
export FONTCONFIG_PATH="${ROOT}/etc/fonts:${FONTCONFIG_PATH:-}"
exec "${ROOT}/usr/bin/sideband_gui.bin" "$@"
APPRUN
  chmod +x "${appdir}/AppRun"

  grep -q '^Icon=com.evalalan.sideband$' "${appdir}/${desktop_id}" \
    || { err "Root desktop file does not reference the Sideband icon"; exit 1; }

  log "Packaging AppImage..."
  mkdir -p "${out_dir}"
  local final="${out_dir}/Sideband-${appimage_arch}.AppImage"
  local tmp; tmp="$(mktemp "${out_dir}/Sideband-${appimage_arch}.tmp.XXXXXX.AppImage")"
  trap 'rm -f "${tmp}"' EXIT
  APPIMAGE_EXTRACT_AND_RUN=1 "${appimagetool}" "${appdir}" "${tmp}"
  chmod +x "${tmp}"
  mv -f "${tmp}" "${final}"
  trap - EXIT
  log "Built ${final}"
}

# --- dispatch ----------------------------------------------------------------

[[ "${1:-}" == "-h" || "${1:-}" == "--help" ]] && usage 0

targets=("$@")
[[ ${#targets[@]} -eq 0 ]] && targets=("all")

for t in "${targets[@]}"; do
  case "$t" in
    tui)     build_tui ;;
    desktop) build_desktop ;;
    android) build_android ;;
    all)     build_tui; build_android; build_desktop ;;
    *)       err "unknown target '$t' (use: tui | desktop | android | all)"; usage 2 ;;
  esac
done

log "Done."
