#!/usr/bin/env bash
# Build Sideband AppImage
# Run from repo root: ./gui/build-appimage.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
MACHINE_ARCH="$(uname -m)"

case "${MACHINE_ARCH}" in
    x86_64|amd64) FLUTTER_ARCH="x64"; APPIMAGE_ARCH="x86_64" ;;
    aarch64|arm64) FLUTTER_ARCH="arm64"; APPIMAGE_ARCH="aarch64" ;;
    *) echo "Unsupported arch: ${MACHINE_ARCH}" >&2; exit 1 ;;
esac

BUILD_DIR="${SCRIPT_DIR}/build/linux/${FLUTTER_ARCH}/release/bundle"
APPDIR="${SCRIPT_DIR}/AppDir"
OUTPUT_DIR="${REPO_ROOT}/dist"

GREEN='\033[0;32m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; NC='\033[0m'
log()  { echo -e "${GREEN}[appimage]${NC} $*"; }
warn() { echo -e "${YELLOW}[appimage]${NC} $*"; }
err()  { echo -e "${RED}[appimage]${NC} $*" >&2; }

FLUTTER="${SCRIPT_DIR}/.tools/flutter/bin/flutter"
[[ ! -x "${FLUTTER}" ]] && FLUTTER="$(command -v flutter 2>/dev/null || true)"
if [[ -z "${FLUTTER}" || ! -x "${FLUTTER}" ]]; then err "Flutter not found"; exit 1; fi

# Step 1: Build Rust binary
log "Building Rust release binary..."
cd "${REPO_ROOT}"
if [[ "${CI:-}" == "true" ]]; then
    warn "Cleaning Cargo target dir in CI to avoid stale cross-run release artifacts..."
    cargo clean
fi
cargo build --release --bin sideband

# Step 2: Build Flutter Linux release
log "Building Flutter Linux release..."
cd "${SCRIPT_DIR}"
"${FLUTTER}" build linux --release
[[ ! -x "${BUILD_DIR}/sideband_gui" ]] && { err "Flutter build failed"; exit 1; }

# Step 3: Install tools
mkdir -p "${HOME}/.local/bin"

# Download appimagetool first (needed to patch linuxdeploy)
APPIMAGETOOL="${HOME}/.local/bin/appimagetool-${APPIMAGE_ARCH}.AppImage"
if [[ ! -x "${APPIMAGETOOL}" ]]; then
    log "Downloading appimagetool..."
    wget -q -O "${APPIMAGETOOL}" \
        "https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-${APPIMAGE_ARCH}.AppImage"
    chmod +x "${APPIMAGETOOL}"
fi
export PATH="${HOME}/.local/bin:${PATH}"

# Download linuxdeploy
LINUXDEPLOY="${HOME}/.local/bin/linuxdeploy-${APPIMAGE_ARCH}.AppImage"
if [[ ! -x "${LINUXDEPLOY}" ]]; then
    log "Downloading linuxdeploy..."
    wget -q -O "${LINUXDEPLOY}" \
        "https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-${APPIMAGE_ARCH}.AppImage"
    chmod +x "${LINUXDEPLOY}"
fi

# Patch linuxdeploy's bundled strip to be a no-op.
# linuxdeploy uses its own bundled strip binary (from its extracted AppImage)
# which is too old to handle .relr.dyn ELF sections on newer distros.
# Replacing it with /usr/bin/true makes all strip calls harmless.
LINUXDEPLOY_PATCHED="${HOME}/.local/bin/linuxdeploy-patched-${APPIMAGE_ARCH}.AppImage"
if [[ ! -x "${LINUXDEPLOY_PATCHED}" ]]; then
    log "Patching linuxdeploy bundled strip..."
    LINUXDEPLOY_EXTRACT="$(mktemp -d)"
    cd "${LINUXDEPLOY_EXTRACT}"
    "${LINUXDEPLOY}" --appimage-extract >/dev/null 2>&1
    SQUASHFS_ROOT="squashfs-root"
    if [[ -f "${SQUASHFS_ROOT}/usr/bin/strip" ]]; then
        cp /usr/bin/true "${SQUASHFS_ROOT}/usr/bin/strip"
    fi
    # Re-pack using appimagetool
    APPIMAGE_EXTRACT_AND_RUN=1 "${APPIMAGETOOL}" "${SQUASHFS_ROOT}" "${LINUXDEPLOY_PATCHED}"
    chmod +x "${LINUXDEPLOY_PATCHED}"
    rm -rf "${LINUXDEPLOY_EXTRACT}"
fi
LINUXDEPLOY="${LINUXDEPLOY_PATCHED}"

# Step 4: Generate icons
ICON_SVG="${SCRIPT_DIR}/linux/icons/sideband_gui.svg"
ICON_DIR="${APPDIR}/usr/share/icons/hicolor"
[[ ! -f "${ICON_SVG}" ]] && { err "Icon SVG not found"; exit 1; }

log "Generating icons..."
rm -rf "${APPDIR}"
for size in 16 32 48 64 128 256 512; do
    mkdir -p "${ICON_DIR}/${size}x${size}/apps"
    DEST="${ICON_DIR}/${size}x${size}/apps/com.evalalan.sideband.png"
    if command -v magick   >/dev/null 2>/dev/null; then
      magick "${ICON_SVG}" -resize "${size}x${size}" "${DEST}"
    elif command -v convert >/dev/null 2>/dev/null; then
      convert "${ICON_SVG}" -resize "${size}x${size}" "${DEST}"
    elif command -v inkscape >/dev/null 2>/dev/null; then
      inkscape "${ICON_SVG}" --export-type=png --export-width="${size}" --export-height="${size}" --export-filename="${DEST}"
    else
      warn "No ImageMagick/convert/inkscape — skipping icon generation for ${size}x${size}"
      break
    fi
done
mkdir -p "${ICON_DIR}/scalable/apps"
cp "${ICON_SVG}" "${ICON_DIR}/scalable/apps/com.evalalan.sideband.svg"

# Step 5: Create AppDir
log "Creating AppDir..."
mkdir -p "${APPDIR}/usr/bin" "${APPDIR}/usr/share/applications" "${APPDIR}/usr/share/metainfo"

cp -r "${BUILD_DIR}/." "${APPDIR}/usr/bin/"
mv "${APPDIR}/usr/bin/sideband_gui" "${APPDIR}/usr/bin/sideband_gui.bin"
cp "${REPO_ROOT}/target/release/sideband" "${APPDIR}/usr/bin/sideband"
chmod +x "${APPDIR}/usr/bin/sideband"

cat > "${APPDIR}/usr/bin/sideband_gui" <<'WRAPPER'
#!/usr/bin/env bash
HERE="$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")"
ROOT="$(dirname "$(dirname "${HERE}")")"
export LD_LIBRARY_PATH="${ROOT}/usr/lib:${ROOT}/usr/bin/lib:${LD_LIBRARY_PATH:-}"
export XDG_DATA_DIRS="${ROOT}/usr/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
exec "${HERE}/sideband_gui.bin" "$@"
WRAPPER
chmod +x "${APPDIR}/usr/bin/sideband_gui"

DESKTOP_ID="com.evalalan.sideband.desktop"
cp "${SCRIPT_DIR}/linux/sideband_gui.desktop" "${APPDIR}/usr/share/applications/${DESKTOP_ID}"

cat > "${APPDIR}/usr/share/metainfo/sideband_gui.metainfo.xml" <<'XML'
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

# Step 6: Deploy dependencies with linuxdeploy
log "Deploying dependencies..."
cd "${SCRIPT_DIR}"

# Note: we do NOT pass --output appimage here, so linuxdeploy only deploys
# dependencies and does NOT try to strip libraries. We use appimagetool
# separately for the final packaging, which uses the host system's strip.

APPIMAGE_EXTRACT_AND_RUN=1 "${LINUXDEPLOY}" \
    --appdir "${APPDIR}" \
    --executable "${APPDIR}/usr/bin/sideband_gui.bin" \
    --executable "${APPDIR}/usr/bin/sideband" \
    --desktop-file "${APPDIR}/usr/share/applications/${DESKTOP_ID}" \
    --icon-file "${ICON_DIR}/scalable/apps/com.evalalan.sideband.svg"

if [[ -f "${APPDIR}/usr/lib/libtray_manager_plugin.so" ]] && \
   [[ ! -e "${APPDIR}/usr/lib/libayatana-appindicator3.so.1" ]] && \
   [[ ! -e "${APPDIR}/usr/lib/libappindicator3.so.1" ]]; then
    err "AppImage missing bundled AppIndicator library required by tray_manager"
    exit 1
fi

cat > "${APPDIR}/AppRun" <<'APPRUN'
#!/usr/bin/env bash
set -euo pipefail
ROOT="${APPDIR:-$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")}"
export LD_LIBRARY_PATH="${ROOT}/usr/lib:${ROOT}/usr/bin/lib:${LD_LIBRARY_PATH:-}"
export XDG_DATA_DIRS="${ROOT}/usr/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
exec "${ROOT}/usr/bin/sideband_gui.bin" "$@"
APPRUN
chmod +x "${APPDIR}/AppRun"

if ! grep -q 'usr/lib' "${APPDIR}/AppRun"; then
    err "AppRun does not include bundled usr/lib in LD_LIBRARY_PATH"
    exit 1
fi

# Step 7: Package with appimagetool (avoids linuxdeploy's bundled strip)
log "Packaging AppImage..."
mkdir -p "${OUTPUT_DIR}"
APPIMAGE_EXTRACT_AND_RUN=1 "${APPIMAGETOOL}" "${APPDIR}" "${OUTPUT_DIR}/Sideband-${APPIMAGE_ARCH}.AppImage"

chmod +x "${OUTPUT_DIR}/Sideband-${APPIMAGE_ARCH}.AppImage"
log "AppImage created: ${OUTPUT_DIR}/Sideband-${APPIMAGE_ARCH}.AppImage"
ls -lh "${OUTPUT_DIR}"/*.AppImage
log "Done!"
