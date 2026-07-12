#!/usr/bin/env bash
#
# Tiered automated test runner for all three Sideband clients. See TESTING.md.
#
#   ./run-tests.sh fast   # default: core + interop + FFI + desktop-CLI + Dart. No Tor, no emulator. Seconds.
#   ./run-tests.sh ui     # fast + Flutter integration_test on Linux (and emulator if one is attached).
#   ./run-tests.sh e2e     # the full two-peer test over real Tor (slow, flaky; nightly/manual).
#   ./run-tests.sh all     # fast + ui + e2e.
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

TIER="${1:-fast}"
FLUTTER="$(command -v flutter 2>/dev/null || echo "$HOME/flutter/bin/flutter")"
ADB="$(command -v adb 2>/dev/null || true)"
if [ -z "$ADB" ] && [ -x "$HOME/Android/Sdk/platform-tools/adb" ]; then
  ADB="$HOME/Android/Sdk/platform-tools/adb"
fi

green() { printf '\033[0;32m%s\033[0m\n' "$*"; }
red()   { printf '\033[0;31m%s\033[0m\n' "$*"; }
hr()    { printf '\n=== %s ===\n' "$*"; }

# --- Desktop client contract -------------------------------------------------
# The desktop GUI is a thin shell over the `sideband` CLI (Process.run). This
# exercises the exact subcommands it issues, against an isolated profile, with
# no Tor. If these pass, the desktop backend contract is intact.
cli_contract() {
  hr "desktop CLI contract"
  local bin="./target/debug/sideband"
  [ -x "$bin" ] || { red "build the binary first (cargo build)"; return 1; }
  # Run in a subshell so the temp dir + cleanup trap are fully contained.
  (
    prof="$(mktemp -d)"
    trap 'rm -rf "$prof"' EXIT

    # Rocky's real keys: base64 with + / and = padding.
    onion="qdnx34k2b3fzp3umv7ryvzxtbzjluzkvuvqixvuooy43b5n6lddaspid.onion"
    ed="fLo7TRtqCxE2wtjTvNvUJjRDBewhYV7bkW3P/F/451w="
    x="K4+eWfSYw8TtmsViirLxsNs7zAWzKQ/YtJtQFVcncUk="

    "$bin" init --profile "$prof" --name Mercury >/dev/null
    "$bin" contact add --profile "$prof" --name Rocky --onion "$onion" \
        --pubkey "$ed" --x25519-pubkey "$x" >/dev/null

    json="$("$bin" contact list --profile "$prof" --json)"
    echo "$json" | grep -q "Rocky"  || { red "contact list missing Rocky"; exit 1; }
    echo "$json" | grep -qF "$ed"   || { red "contact list dropped the ed25519 key"; exit 1; }

    # history --json (what the GUI polls) must be valid JSON for a known contact.
    "$bin" history --profile "$prof" --contact Rocky --json >/dev/null
    # group list --json (GUI sidebar) must be valid JSON even when empty.
    "$bin" group list --profile "$prof" --json >/dev/null

    "$bin" contact delete --profile "$prof" --name Rocky >/dev/null
    "$bin" contact list --profile "$prof" | grep -q "no contacts" \
        || { red "contact delete did not remove Rocky"; exit 1; }

    green "desktop CLI contract OK"
  )
}

run_fast() {
  hr "cargo test (core + interop harness + FFI contract)"
  cargo test
  cli_contract
  hr "flutter test (Dart logic + widgets)"
  (cd gui && "$FLUTTER" test)
  green "FAST tier passed"
}

run_ui() {
  hr "flutter integration_test on Linux desktop"
  if [ -d gui/integration_test ]; then
    (
      prof="$(mktemp -d)"
      trap 'rm -rf "$prof"' EXIT
      ./target/debug/sideband init --profile "$prof" --name UiTest >/dev/null
      cd gui
      SIDEBAND_BIN="$SCRIPT_DIR/target/debug/sideband" \
        SIDEBAND_PROFILE="$prof" \
        "$FLUTTER" test integration_test -d linux
    )
  else
    red "gui/integration_test not present yet — skipping UI tier"
  fi
  local android_device=""
  if [ -n "$ADB" ]; then
    while read -r serial state; do
      if [[ "$serial" == emulator-* && "$state" = "device" ]]; then
        android_device="$serial"
        break
      fi
    done < <("$ADB" devices)
  fi
  if [ -n "$android_device" ]; then
    hr "flutter integration_test on attached Android device/emulator"
    "$ADB" -s "$android_device" shell pm clear com.example.sideband_gui \
      >/dev/null 2>&1 || true
    (cd gui && "$FLUTTER" test integration_test -d "$android_device")
  else
    red "no Android emulator attached — skipping Android UI tier"
  fi
}

run_e2e() {
  hr "two-peer end-to-end over Tor"
  ./test_e2e.sh
}

case "$TIER" in
  fast) run_fast ;;
  ui)   run_fast; run_ui ;;
  e2e)  run_e2e ;;
  all)  run_fast; run_ui; run_e2e ;;
  *)    red "unknown tier '$TIER' (use: fast | ui | e2e | all)"; exit 2 ;;
esac
