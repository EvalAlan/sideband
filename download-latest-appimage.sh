#!/bin/bash
# download-latest-appimage.sh
# Downloads the latest successful AppImage build from CI and prepares it for use.
# Usage: ./download-latest-appimage.sh [output-dir]
# Default output-dir: /tmp/sideband-appimage

set -euo pipefail

OUTPUT_DIR="${1:-/tmp/sideband-appimage}"
REPO="EvalAlan/sideband"
WORKFLOW="appimage.yml"

# Find the latest successful run
echo "🔍 Finding latest successful AppImage build…"
RUN_ID=$(gh run list \
  --repo "$REPO" \
  --workflow "$WORKFLOW" \
  --status success \
  --limit 1 \
  --json databaseId \
  --jq '.[0].databaseId')

if [[ -z "$RUN_ID" ]]; then
  echo "❌ No successful AppImage build found." >&2
  exit 1
fi

# Get run info for display
RUN_INFO=$(gh run view "$RUN_ID" \
  --repo "$REPO" \
  --json headSha,createdAt,displayTitle \
  --jq '"  commit: " + .headSha[:8] + "\n  date:   " + .createdAt + "\n  title:  " + .displayTitle')

echo "✅ Found run #$RUN_ID"
echo "$RUN_INFO"
echo ""

# Clean and download
rm -rf "$OUTPUT_DIR"
mkdir -p "$OUTPUT_DIR"

echo "⬇️  Downloading artifact…"
gh run download "$RUN_ID" \
  --repo "$REPO" \
  --name sideband-appimage-x86_64 \
  --dir "$OUTPUT_DIR"

# Make executable
chmod +x "$OUTPUT_DIR"/*.AppImage

echo ""
echo "🚀 AppImage ready: $OUTPUT_DIR/"
ls -lh "$OUTPUT_DIR"/*.AppImage
echo ""
echo "To launch:"
echo "  pkill -f 'sideband serve --profile'  # kill stale listeners"
echo "  $OUTPUT_DIR/$(ls "$OUTPUT_DIR"/*.AppImage | head -1 | xargs basename)"
