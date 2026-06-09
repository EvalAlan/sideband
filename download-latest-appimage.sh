#!/bin/bash
# download-latest-appimage.sh
# Downloads the newest x86_64 AppImage artifact from CI and prepares it for use.
# Usage: ./download-latest-appimage.sh [output-dir]
# Default output-dir: /tmp/sideband-appimage

set -euo pipefail

OUTPUT_DIR="${1:-/tmp/sideband-appimage}"
REPO="EvalAlan/sideband"
WORKFLOW="appimage.yml"
ARTIFACT="sideband-appimage-x86_64"

echo "🔍 Finding newest x86_64 AppImage artifact…"

RUNS_JSON=$(gh run list \
  --repo "$REPO" \
  --workflow "$WORKFLOW" \
  --limit 25 \
  --json databaseId,headSha,createdAt,displayTitle,status,conclusion)

RUN_ID=""
RUN_SHA=""
RUN_DATE=""
RUN_TITLE=""
RUN_CONCLUSION=""

while IFS= read -r encoded; do
  row=$(printf '%s' "$encoded" | base64 --decode)
  id=$(printf '%s' "$row" | jq -r '.databaseId')
  sha=$(printf '%s' "$row" | jq -r '.headSha')
  created=$(printf '%s' "$row" | jq -r '.createdAt')
  title=$(printf '%s' "$row" | jq -r '.displayTitle')
  status=$(printf '%s' "$row" | jq -r '.status')
  conclusion=$(printf '%s' "$row" | jq -r '.conclusion // ""')
  [[ -z "$id" || "$status" != "completed" ]] && continue
  if gh api "repos/$REPO/actions/runs/$id/artifacts" \
      --jq ".artifacts[] | select(.name == \"$ARTIFACT\" and .expired == false) | .id" \
      | grep -q .; then
    RUN_ID="$id"
    RUN_SHA="$sha"
    RUN_DATE="$created"
    RUN_TITLE="$title"
    RUN_CONCLUSION="$conclusion"
    break
  fi
done < <(printf '%s' "$RUNS_JSON" | jq -r '.[] | @base64')

if [[ -z "$RUN_ID" ]]; then
  echo "❌ No downloadable $ARTIFACT artifact found in recent AppImage runs." >&2
  exit 1
fi

if [[ "$RUN_CONCLUSION" != "success" ]]; then
  echo "⚠️  Workflow conclusion is '$RUN_CONCLUSION', but the x86_64 AppImage artifact exists."
  echo "   This is expected when another matrix job failed or an artifact upload conflicted."
fi

echo "✅ Found run #$RUN_ID"
echo "  commit: ${RUN_SHA:0:8}"
echo "  date:   $RUN_DATE"
echo "  title:  $RUN_TITLE"
echo ""

rm -rf "$OUTPUT_DIR"
mkdir -p "$OUTPUT_DIR"

echo "⬇️  Downloading $ARTIFACT…"
gh run download "$RUN_ID" \
  --repo "$REPO" \
  --name "$ARTIFACT" \
  --dir "$OUTPUT_DIR"

chmod +x "$OUTPUT_DIR"/*.AppImage

echo ""
echo "🚀 AppImage ready: $OUTPUT_DIR/"
ls -lh "$OUTPUT_DIR"/*.AppImage
echo ""
echo "To launch:"
echo "  pkill -f 'sideband serve --profile'  # kill stale listeners"
echo "  $OUTPUT_DIR/$(ls "$OUTPUT_DIR"/*.AppImage | head -1 | xargs basename)"
