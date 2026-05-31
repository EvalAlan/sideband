#!/usr/bin/env bash
set -euo pipefail

cd /home/rocky/repos/sideband

echo "=== Step 1: Clean slate ==="
pkill -9 -f "tor -f" 2>/dev/null || true
sleep 1
echo "Done."

echo "=== Step 2: Start Alice serve (background) ==="
cargo run -- serve --profile ./profiles/alice >/tmp/alice_stdout.log 2>&1 &
ALICE_PID=$!
echo "Alice PID=$ALICE_PID"

echo "=== Step 3: Wait for Alice to be ready ==="
READY=0
ONION=""
for i in $(seq 1 90); do
  HAS_LISTENER=$(ss -tlnp 2>/dev/null | grep -c "sideband" || true)
  HAS_HOSTNAME=false
  if [ -f ./profiles/alice/tor/hs/hostname ]; then
    RAW=$(cat ./profiles/alice/tor/hs/hostname 2>/dev/null | tr -d '[:space:]')
    if echo "$RAW" | grep -q "onion$"; then
      HAS_HOSTNAME=true
      ONION="$RAW"
    fi
  fi
  if [ "$HAS_LISTENER" -gt 0 ] && [ "$HAS_HOSTNAME" = "true" ]; then
    READY=1
    break
  fi
  sleep 1
done

echo "listener=$HAS_LISTENER hostname=$HAS_HOSTNAME onion=$ONION"

if [ "$READY" -ne 1 ]; then
  echo "FAIL: Alice not ready after 90s"
  kill $ALICE_PID 2>/dev/null || true
  exit 1
fi

echo "=== ALICE READY ==="
echo "ONION=$ONION"
ss -tlnp | grep sideband

echo "=== Step 4: Bob sends ==="
cargo run -- send --profile ./profiles/bob --to "$ONION" --message "hello from bob" 2>&1
SEND_RC=$?
echo "Bob send rc=$SEND_RC"

echo "=== Step 5: Wait ==="
sleep 10

echo "=== Step 6: Alice stdout ==="
cat /tmp/alice_stdout.log 2>/dev/null || echo "(no stdout)"

echo "=== Step 7: Debug recv ==="
cat /tmp/alice_recv.log 2>/dev/null || echo "(no recv log)"

echo "=== Step 8: DB ==="
sqlite3 ./profiles/alice/messages.db "SELECT * FROM messages;" 2>/dev/null || echo "(empty)"

echo "=== Cleanup ==="
kill $ALICE_PID 2>/dev/null || true
pkill -9 -f "tor -f" 2>/dev/null || true
echo "Done."
