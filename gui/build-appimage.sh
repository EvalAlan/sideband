#!/usr/bin/env bash
# Build Sideband AppImage using linuxdeploy + Flutter plugin
# Run from repo root: ./gui/build-appimage.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
MACHINE_ARCH="$(uname -m)"
case "${MACHINE_ARCH}" in
    x86_64|amd64)
        FLUTTER_ARCH="x64"
        APPIMAGE_ARCH="x86_64"
        ;;
    aarch64|arm64)
        FLUTTER_ARCH="arm64"
        APPIMAGE_ARCH="aarch64"
        ;;
    *)
        echo "Unsupported AppImage architecture: ${MACHINE_ARCH}" >&2
        exit 1
        ;;
esac
BUILD_DIR="${SCRIPT_DIR}/build/linux/${FLUTTER_ARCH}/release/bundle"
APPDIR="${SCRIPT_DIR}/AppDir"
OUTPUT_DIR="${REPO_ROOT}/dist"

# Colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

log() { echo -e "${GREEN}[appimage]${NC} $*"; }
warn() { echo -e "${YELLOW}[appimage]${NC} $*"; }
err() { echo -e "${RED}[appimage]${NC} $*" >&2; }

# Check for Flutter
FLUTTER="${SCRIPT_DIR}/.tools/flutter/bin/flutter"
if [[ ! -x "${FLUTTER}" ]]; then
    FLUTTER="$(command -v flutter 2>/dev/null || true)"
fi
if [[ -z "${FLUTTER}" || ! -x "${FLUTTER}" ]]; then
    err "Flutter not found. Install Flutter or run from gui/ with .tools/flutter/"
    exit 1
fi

# Step 1: Build Flutter Linux release bundle
log "Building Sideband Rust release binary..."
cd "${REPO_ROOT}"
cargo build --release --bin sideband

log "Building Flutter Linux release bundle..."
cd "${SCRIPT_DIR}"
"${FLUTTER}" build linux --release

if [[ ! -x "${BUILD_DIR}/sideband_gui" ]]; then
    err "Build failed: ${BUILD_DIR}/sideband_gui not found"
    exit 1
fi
log "Flutter bundle ready at ${BUILD_DIR}/sideband_gui"

# Step 2: Install linuxdeploy if needed
LINUXDEPLOY="${HOME}/.local/bin/linuxdeploy-${APPIMAGE_ARCH}.AppImage"

mkdir -p "${HOME}/.local/bin"

if [[ ! -x "${LINUXDEPLOY}" ]]; then
    log "Downloading linuxdeploy..."
    wget -q -O "${LINUXDEPLOY}" \
        "https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-${APPIMAGE_ARCH}.AppImage"
    chmod +x "${LINUXDEPLOY}"
fi
export PATH="${HOME}/.local/bin:${PATH}"

# Step 3: Generate PNG icons from SVG (requires imagemagick or inkscape)
ICON_SVG="${SCRIPT_DIR}/linux/icons/sideband_gui.svg"
ICON_DIR="${APPDIR}/usr/share/icons/hicolor"

if [[ ! -f "${ICON_SVG}" ]]; then
    err "Icon SVG not found at ${ICON_SVG}"
    exit 1
fi

log "Generating PNG icons from SVG..."
rm -rf "${APPDIR}"
for size in 16 32 48 64 128 256 512; do
    mkdir -p "${ICON_DIR}/${size}x${size}/apps"
    if command -v magick >/dev/null 2>&1; then
        magick "${ICON_SVG}" -resize "${size}x${size}" "${ICON_DIR}/${size}x${size}/apps/sideband_gui.png"
    elif command -v convert >/dev/null 2>&1; then
        convert "${ICON_SVG}" -resize "${size}x${size}" "${ICON_DIR}/${size}x${size}/apps/sideband_gui.png"
    elif command -v inkscape >/dev/null 2>&1; then
        inkscape "${ICON_SVG}" --export-type=png --export-width="${size}" --export-height="${size}" \
            --export-filename="${ICON_DIR}/${size}x${size}/apps/sideband_gui.png"
    else
        warn "No imagemagick/inkscape found; skipping PNG generation for ${size}x${size}"
        warn "Install imagemagick (magick/convert) or inkscape for icon generation"
    fi
done

# Also create a scalable SVG icon
mkdir -p "${ICON_DIR}/scalable/apps"
cp "${ICON_SVG}" "${ICON_DIR}/scalable/apps/sideband_gui.svg"

# Step 4: Create AppDir structure
log "Creating AppDir..."
mkdir -p "${APPDIR}/usr/bin"
mkdir -p "${APPDIR}/usr/share/applications"
mkdir -p "${APPDIR}/usr/share/metainfo"

# Copy Flutter bundle
cp -r "${BUILD_DIR}/." "${APPDIR}/usr/bin/"
mv "${APPDIR}/usr/bin/sideband_gui" "${APPDIR}/usr/bin/sideband_gui.bin"
cp "${REPO_ROOT}/target/release/sideband" "${APPDIR}/usr/bin/sideband"
chmod +x "${APPDIR}/usr/bin/sideband"

# Create wrapper script that sets up environment
cat > "${APPDIR}/usr/bin/sideband_gui" <<'WRAPPER'
#!/usr/bin/env bash
# Sideband AppImage wrapper
HERE="$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")"
export LD_LIBRARY_PATH="${HERE}/lib:${LD_LIBRARY_PATH:-}"
export XDG_DATA_DIRS="${HERE}/../share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
exec "${HERE}/sideband_gui.bin" "$@"
WRAPPER
chmod +x "${APPDIR}/usr/bin/sideband_gui"

# Copy desktop file
cp "${SCRIPT_DIR}/linux/sideband_gui.desktop" "${APPDIR}/usr/share/applications/sideband_gui.desktop"

# Create AppStream metainfo
cat > "${APPDIR}/usr/share/metainfo/sideband_gui.metainfo.xml" <<'METAINFO'
<?xml version="1.0" encoding="UTF-8"?>
<component type="desktop-application">
  <id>sideband_gui</id>
  <name>Sideband</name>
  <summary>Tor-based decentralized messenger</summary>
  <description>
    <p>Sideband is a peer-to-peer messenger using Tor onion services for transport.
    No central servers, no phone numbers, just cryptographic identities.</p>
    <p>Features:</p>
    <ul>
      <li>End-to-end encryption with Double Ratchet</li>
      <li>Tor hidden service connectivity</li>
      <li>Contact verification via QR codes</li>
      <li>File transfer support</li>
    </ul>
  </description>
  <project_license>MIT</project_license>
  <url type="homepage">https://github.com/EvalAlan/sideband</url>
  <url type="bugtracker">https://github.com/EvalAlan/sideband/issues</url>
  <categories>
    <category>Network</category>
    <category>InstantMessaging</category>
    <category>Security</category>
  </categories>
  <icon type="stock">sideband_gui</icon>
  <launchable type="desktop-id">sideband_gui.desktop</launchable>
  <provides>
    <binary>sideband_gui</binary>
  </provides>
</component>
METAINFO

# Step 5: Run linuxdeploy
log "Running linuxdeploy..."
mkdir -p "${OUTPUT_DIR}"

cd "${SCRIPT_DIR}"

APPIMAGE_EXTRACT_AND_RUN=1 "${LINUXDEPLOY}" \
    --appdir "${APPDIR}" \
    --executable "${APPDIR}/usr/bin/sideband_gui.bin" \
    --executable "${APPDIR}/usr/bin/sideband" \
    --desktop-file "${APPDIR}/usr/share/applications/sideband_gui.desktop" \
    --icon-file "${ICON_DIR}/scalable/apps/sideband_gui.svg" \
    --output appimage

# Find and move the generated AppImage
APPIMAGE=$(find "${SCRIPT_DIR}" -maxdepth 1 -name "*.AppImage" -type f | head -1)
if [[ -n "${APPIMAGE}" ]]; then
    mv "${APPIMAGE}" "${OUTPUT_DIR}/"
    log "AppImage created: ${OUTPUT_DIR}/$(basename "${APPIMAGE}")"
    ls -lh "${OUTPUT_DIR}"/*.AppImage
else
    err "AppImage not found after build"
    exit 1
fi

log "Done!"