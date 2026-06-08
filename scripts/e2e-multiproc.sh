#!/usr/bin/env bash
# E2E: multi-process CPU hog is grouped and throttled as one app.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if ! command -v stress-ng >/dev/null 2>&1; then
    echo "stress-ng is required (install: sudo apt install stress-ng)" >&2
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

echo "==> starting daemon (app cap 25%, pressure 50%)"
"$BIN" --daemon --no-tray --foreground --app-cap 25 --pressure-threshold 50 --interval 500 &
DAEMON_PID=$!

for _ in $(seq 1 50); do
    if "$BIN" --status >/dev/null 2>&1; then
        break
    fi
    sleep 0.2
done

echo "==> starting forked CPU hog (stress-ng --fork 3 --cpu 2)"
stress-ng --fork 3 --cpu 2 --timeout 30s >/dev/null 2>&1 &
HOG_PID=$!

echo "==> polling for grouped throttling"
for _ in $(seq 1 90); do
    if "$BIN" --status | python3 -c "
import json, sys
state = json.load(sys.stdin)
groups = state.get('throttled_groups') or []
if not groups:
    sys.exit(1)
group = groups[0]
pids = group.get('pids') or []
if len(pids) >= 2:
    sys.exit(0)
sys.exit(1)
"; then
        echo "E2E PASS: multi-process group throttled"
        exit 0
    fi
    sleep 0.5
done

echo "E2E FAIL: expected throttled group with multiple pids" >&2
"$BIN" --status >&2 || true
exit 1
