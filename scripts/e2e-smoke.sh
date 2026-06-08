#!/usr/bin/env bash
# End-to-end smoke test: daemon detects a CPU hog and throttles it.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if ! command -v stress-ng >/dev/null 2>&1; then
    echo "stress-ng is required for e2e tests (install: sudo apt install stress-ng)" >&2
    exit 1
fi

echo "==> building release binary"
cargo build --release --quiet
BIN="$ROOT/target/release/wrangler"

RUNTIME="$(mktemp -d)"
CONFIG="$(mktemp -d)"
export XDG_RUNTIME_DIR="$RUNTIME"
export WRANGLER_CONFIG_DIR="$CONFIG"
export WRANGLER_RUNTIME_DIR="$RUNTIME"

cleanup() {
    if [[ -n "${DAEMON_PID:-}" ]]; then
        kill "$DAEMON_PID" 2>/dev/null || true
        wait "$DAEMON_PID" 2>/dev/null || true
    fi
    if [[ -n "${HOG_PID:-}" ]]; then
        kill "$HOG_PID" 2>/dev/null || true
        wait "$HOG_PID" 2>/dev/null || true
    fi
    rm -rf "$RUNTIME" "$CONFIG"
}
trap cleanup EXIT

echo "==> starting daemon (threshold 25%, interval 500ms)"
"$BIN" --daemon --no-tray --threshold 25 --interval 500 &
DAEMON_PID=$!

echo "==> waiting for daemon IPC"
ready=0
for _ in $(seq 1 50); do
    if "$BIN" --status >/dev/null 2>&1; then
        ready=1
        break
    fi
    sleep 0.2
done
if [[ "$ready" -ne 1 ]]; then
    echo "E2E FAIL: daemon did not become ready" >&2
    exit 1
fi

echo "==> starting CPU hog (stress-ng)"
stress-ng --cpu 1 --timeout 30s >/dev/null 2>&1 &
HOG_PID=$!

echo "==> polling for throttled processes"
for _ in $(seq 1 90); do
    if "$BIN" --status | python3 -c "
import json, sys
state = json.load(sys.stdin)
pids = state.get('throttled_pids') or []
if isinstance(pids, list) and len(pids) > 0:
    sys.exit(0)
sys.exit(1)
"; then
        echo "E2E PASS: throttling engaged"
        exit 0
    fi
    sleep 0.5
done

echo "E2E FAIL: no throttling observed within timeout" >&2
"$BIN" --status >&2 || true
exit 1
