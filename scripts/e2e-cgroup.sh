#!/usr/bin/env bash
# E2E: cgroup v2 per-app-group enforcement (requires root).
set -euo pipefail

if [[ "${EUID:-$(id -u)}" -ne 0 ]]; then
    echo "SKIP: cgroup e2e requires root (run: sudo make e2e-cgroup)"
    exit 0
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if ! command -v stress-ng >/dev/null 2>&1; then
    echo "stress-ng is required (install: apt install stress-ng)" >&2
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
    rm -rf /sys/fs/cgroup/wrangler 2>/dev/null || true
}
trap cleanup EXIT

echo "==> starting root daemon (cgroup backend expected)"
"$BIN" --daemon --no-tray --foreground --app-cap 25 --pressure-threshold 50 --interval 500 &
DAEMON_PID=$!

for _ in $(seq 1 50); do
    if "$BIN" --status >/dev/null 2>&1; then
        break
    fi
    sleep 0.2
done

BACKEND=$("$BIN" --status | python3 -c "import json,sys; print(json.load(sys.stdin).get('throttle_backend',''))")
if [[ "$BACKEND" != "cgroup" ]]; then
    echo "E2E FAIL: expected cgroup backend, got '$BACKEND'" >&2
    exit 1
fi
echo "==> throttle backend: $BACKEND"

echo "==> starting CPU hog"
stress-ng --cpu 0 --timeout 30s >/dev/null 2>&1 &
HOG_PID=$!

echo "==> polling for cgroup directory and throttling"
for _ in $(seq 1 90); do
    if "$BIN" --status | python3 -c "
import json, sys, glob
state = json.load(sys.stdin)
pids = state.get('throttled_pids') or []
groups = state.get('throttled_groups') or []
cgroup_dirs = glob.glob('/sys/fs/cgroup/wrangler/group-*')
if pids and groups and cgroup_dirs:
    sys.exit(0)
sys.exit(1)
"; then
        echo "E2E PASS: cgroup throttle active under /sys/fs/cgroup/wrangler/"
        ls -la /sys/fs/cgroup/wrangler/ 2>/dev/null || true
        exit 0
    fi
    sleep 0.5
done

echo "E2E FAIL: cgroup throttling not observed" >&2
"$BIN" --status >&2 || true
ls -la /sys/fs/cgroup/wrangler/ 2>/dev/null || true
exit 1
