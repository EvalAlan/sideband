#!/usr/bin/env bash
# Sideband GUI launcher for Linux desktop.
# Builds with Flutter, then runs the native bundle directly so crashes are
# captured in run-linux.log instead of being hidden behind `flutter run` noise.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOG="${SCRIPT_DIR}/run-linux.log"

# Pick Flutter
FLUTTER=""
for c in \
  "${SCRIPT_DIR}/.tools/flutter/bin/flutter" \
  "${SCRIPT_DIR}/../.tools/flutter/bin/flutter" \
  "$(command -v flutter 2>/dev/null)"; do
  [[ -n "$c" && -x "$c" ]] && FLUTTER="$c" && break
done
if [[ -z "$FLUTTER" ]]; then
  echo "Flutter SDK not found. Install or add to PATH."
  exit 1
fi

export GTK_THEME="${GTK_THEME:-Adwaita:dark}"
DEFAULT_SIDEBAND_BIN=0
if [[ -z "${SIDEBAND_BIN:-}" ]]; then
  SIDEBAND_BIN="${SCRIPT_DIR}/../target/debug/sideband"
  DEFAULT_SIDEBAND_BIN=1
  export SIDEBAND_BIN
fi
export SIDEBAND_PROFILE="${SIDEBAND_PROFILE:-${HOME}/.sideband}"

echo "[sideband] Flutter: $FLUTTER" | tee "$LOG"
echo "[sideband] Backend: $SIDEBAND_BIN" | tee -a "$LOG"
echo "[sideband] Profile: $SIDEBAND_PROFILE" | tee -a "$LOG"
echo "[sideband] GTK_THEME=$GTK_THEME" | tee -a "$LOG"
echo "[sideband] DISPLAY=${DISPLAY:-}" | tee -a "$LOG"
echo "[sideband] GDK_BACKEND=${GDK_BACKEND:-}" | tee -a "$LOG"

if [[ "$DEFAULT_SIDEBAND_BIN" == "1" ]]; then
  echo "[sideband] Building Rust backend..." | tee -a "$LOG"
  cargo build --manifest-path "${SCRIPT_DIR}/../Cargo.toml" 2>&1 | tee -a "$LOG"
else
  echo "[sideband] Using external SIDEBAND_BIN; not rebuilding backend." | tee -a "$LOG"
fi

cd "$SCRIPT_DIR"

find_bundle() {
  local candidate
  for candidate in \
    "${SCRIPT_DIR}/build/linux/x64/debug/bundle/sideband_gui" \
    "${SCRIPT_DIR}/build/linux/arm64/debug/bundle/sideband_gui" \
    "${SCRIPT_DIR}/build/linux/x64/release/bundle/sideband_gui" \
    "${SCRIPT_DIR}/build/linux/arm64/release/bundle/sideband_gui"; do
    if [[ -x "$candidate" ]]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done
  return 1
}

# Escape hatch for hot reload while actively hacking Flutter widgets.
if [[ "${1:-}" == "--flutter-run" ]]; then
  shift
  echo "[sideband] Mode: flutter run" | tee -a "$LOG"
  set +e
  "$FLUTTER" run -d linux "$@" 2>&1 | tee -a "$LOG"
  code=${PIPESTATUS[0]}
  set -e
  echo "[sideband] flutter run exited: $code" | tee -a "$LOG"
  exit "$code"
fi

echo "[sideband] Building Linux GUI bundle..." | tee -a "$LOG"
"$FLUTTER" build linux --debug 2>&1 | tee -a "$LOG"

BIN="$(find_bundle || true)"
if [[ -z "$BIN" ]]; then
  echo "[sideband] No built binary found after flutter build linux." | tee -a "$LOG"
  exit 1
fi

echo "[sideband] Binary: $BIN" | tee -a "$LOG"
echo "[sideband] Launching native bundle directly." | tee -a "$LOG"
set +e
"$BIN" 2>&1 | tee -a "$LOG"
code=${PIPESTATUS[0]}
set -e
echo "[sideband] Native bundle exited: $code" | tee -a "$LOG"
exit "$code"
