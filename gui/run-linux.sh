#!/usr/bin/env bash
# Sideband GUI launcher for Linux desktop
# Forces consistent dark GTK theme so Flutter rendering isn't mangled
# by host GTK theme/extension quirks.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FLUTTER="${SCRIPT_DIR}/../.tools/flutter/bin/flutter"

export GTK_THEME="${GTK_THEME:-Adwaita:dark}"

if [ ! -x "$FLUTTER" ]; then
  echo "Flutter SDK not found at $FLUTTER".
  echo "If Flutter is on PATH, just run: flutter run -d linux"
  exit 1
fi

cd "$SCRIPT_DIR"
"$FLUTTER" run -d linux "$@"
