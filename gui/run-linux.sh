#!/usr/bin/env bash
# Sideband GUI launcher for Linux desktop
# Forces consistent dark GTK theme so Flutter rendering isn't mangled
# by host GTK theme/extension quirks.

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
echo "[sideband] Flutter: $FLUTTER" | tee "$LOG"
echo "[sideband] GTK_THEME=$GTK_THEME" | tee -a "$LOG"
echo "[sideband] DISPLAY=$DISPLAY" | tee -a "$LOG"
echo "[sideband] GDK_BACKEND=$GDK_BACKEND" | tee -a "$LOG"

cd "$SCRIPT_DIR"

# Try flutter run first; if it fails, fall back to direct binary exec with stderr capture
if "$FLUTTER" run -d linux "$@" 2>&1 | tee -a "$LOG"; then
  echo "[sideband] Exited cleanly." | tee -a "$LOG"
else
  echo "" | tee -a "$LOG"
  echo "[sideband] ── flutter run failed, trying direct binary ──" | tee -a "$LOG"
  BIN="$(find "${SCRIPT_DIR}/build" -name sideband_gui -type f 2>/dev/null | head -1)"
  if [[ -z "$BIN" ]]; then
    echo "[sideband] No built binary found. Run: $FLUTTER build linux" | tee -a "$LOG"
    exit 1
  fi
  echo "[sideband] Binary: $BIN" | tee -a "$LOG"
  "$BIN" 2>&1 | tee -a "$LOG"
fi
