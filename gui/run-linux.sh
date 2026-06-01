#!/usr/bin/env bash
# Sideband GUI launcher for Linux desktop
# Forces consistent dark GTK theme so Flutter rendering isn't mangled
# by host GTK theme/extension quirks.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

export GTK_THEME="${GTK_THEME:-Adwaita:dark}"

# Check multiple Flutter SDK locations
for candidate in \
  "${SCRIPT_DIR}/.tools/flutter/bin/flutter" \
  "${SCRIPT_DIR}/../.tools/flutter/bin/flutter" \
  "$(command -v flutter 2>/dev/null)"; do
  if [ -n "$candidate" ] && [ -x "$candidate" ]; then
    FLUTTER="$candidate"
    break
  fi
done

if [ -z "${FLUTTER:-}" ]; then
  echo "Flutter SDK not found."
  echo "Install it or add 'flutter' to PATH, then run: flutter run -d linux"
  exit 1
fi

echo "[sideband] Flutter: $FLUTTER"
cd "$SCRIPT_DIR"
"$FLUTTER" run -d linux "$@"
