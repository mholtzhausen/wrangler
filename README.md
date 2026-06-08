# Wrangler

Process monitor and CPU throttle utility for Ubuntu/Linux. Wrangler groups processes by app, and when the system is under pressure caps the hottest offender so one runaway app cannot starve the machine.

## Features

- Real-time process list sorted by CPU usage
- App-group CPU budgets as a % of the whole machine (not per-core)
- Pressure gate: throttling only engages when global CPU is high
- SIGSTOP/SIGCONT duty-cycle throttling (default, no root required)
- cgroups v2 `cpu.max` per app group when running as root
- Background daemon with system tray (Linux)
- Attachable dashboard that shares state with the daemon
- Persistent settings in `~/.config/wrangler/config.toml`

## Requirements

- Linux (Ubuntu tested)
- Rust 1.70+ (to build from source)
- For system tray: a desktop with StatusNotifierItem support (GNOME, KDE, etc.)
- For cgroups mode: root and unified cgroups v2 with `cpu` controller

## Build

```bash
make release
# binary: target/release/wrangler
```

## Quick start

### Interactive TUI (standalone)

```bash
cargo run
# or
make run
```

Keys: **g** flat/grouped view (grouped by default), **o** expand group, **+/-** app cap, **Up/Down** scroll, **q/Esc** quit. Grouped view shows CPU as % of total machine capacity.

### Background daemon + tray

```bash
make run-daemon
# or
target/release/wrangler --tray
# equivalent to --daemon (detaches from the terminal; lives in the system tray)
```

Right-click the tray icon:

- **Open Dashboard** — opens the TUI attached to the daemon
- **Quit** — stops the daemon

Headless daemon (no tray):

```bash
wrangler --daemon --no-tray
```

When started from an interactive terminal, daemon mode returns control to your shell immediately and keeps running in the background. Use `--foreground` to stay attached (systemd, debugging).

### Attach to a running daemon

```bash
wrangler --attach
```

If a daemon is already running, launching `wrangler` without flags automatically attaches instead of starting a second monitor.

## Configuration

Settings are stored at `~/.config/wrangler/config.toml`:

```toml
app_cap = 40.0              # max % of machine CPU per app group
pressure_threshold = 85.0   # global CPU % before throttling may engage (0 = always)
top_offenders = 1           # how many hottest groups to consider
grouping = "tree"           # tree (process tree root) or name (executable name)
protected_apps = []         # extra never-throttle app names (builtins always protected)
interval_ms = 1000
use_cgroups = false
```

The legacy `threshold` key is still accepted on load and maps to `app_cap`. On first load, legacy configs are rewritten to the current schema automatically.

**Dashboard keys:** `g` toggle flat/grouped view, `o`/`Enter` expand/collapse a group, `+/-` app cap, `Up/Down` scroll.

CLI flags override the file for that invocation. App cap changes from the dashboard are saved automatically.

## Systemd user service

```bash
make install-systemd
systemctl --user enable --now wrangler.service
```

Unit file: `contrib/systemd/user/wrangler.service`

## CLI reference

| Flag | Description |
|------|-------------|
| `--app-cap`, `--threshold` | Max % of machine CPU per app group (default: 40) |
| `--pressure-threshold` | Global CPU % before throttling may engage (default: 85) |
| `--grouping` | App grouping: `tree` (default) or `name` |
| `--interval` | Poll interval in ms (default: from config or 1000) |
| `--cgroups` | Request cgroups v2 (effective when running as root) |
| `--daemon`, `--tray` | Run monitor/throttle in background (detaches from terminal) |
| `--foreground` | Keep daemon attached (systemd, debugging) |
| `--no-tray` | Daemon without system tray |
| `--attach` | Connect TUI to running daemon |
| `--status` | Print daemon state as JSON and exit |

## Permissions

- **Signal throttling** works on processes owned by your user. Root-owned processes require matching privileges.
- **cgroups v2** is used automatically when running as root. Example: `sudo wrangler --tray`
- Do not use `setcap cap_sys_ptrace` on the binary; prefer matching UID or cgroups as root.

## Testing

```bash
make test       # unit tests
make ci         # fmt-check + clippy + test (same as CI check job)
make e2e        # end-to-end throttle smoke test (requires stress-ng)
```

CI runs on every push/PR to `main` via [`.github/workflows/ci.yml`](.github/workflows/ci.yml):
- **check** — `cargo fmt --check`, `clippy`, unit tests
- **e2e** — headless daemon + `stress-ng` CPU hog; verifies throttling via `wrangler --status`

Additional local e2e targets:

```bash
make e2e-multiproc   # forked stress-ng; verifies multi-PID app group throttling
sudo make e2e-cgroup # cgroup v2 dirs + cpu.max (root only)
```

Query a running daemon:

```bash
wrangler --status   # JSON snapshot of processes, app cap, throttled groups
```

## Architecture

```
Monitor (sysinfo) ──► Event Hub ◄── TUI / IPC clients
                         │
                    Throttle engine
                    (signals or cgroups)
```

The daemon exposes a Unix socket at `$XDG_RUNTIME_DIR/wrangler.sock` for dashboard attach and app cap sync.

## License

See repository for license details.
