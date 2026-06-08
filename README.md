# Wrangler

Process monitor and CPU throttle utility for Ubuntu/Linux. Wrangler watches system processes, throttles CPU hogs that exceed a configurable threshold, and provides a Ratatui dashboard for live inspection.

## Features

- Real-time process list sorted by CPU usage
- SIGSTOP/SIGCONT duty-cycle throttling (default, no root required)
- Optional cgroups v2 `cpu.max` throttling (`--cgroups`, requires root)
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

Keys: **Up/Down** adjust threshold, **q/Esc** quit.

### Background daemon + tray

```bash
make run-daemon
# or
target/release/wrangler --daemon
```

Right-click the tray icon:

- **Open Dashboard** — opens the TUI attached to the daemon
- **Quit** — stops the daemon

Headless daemon (no tray):

```bash
wrangler --daemon --no-tray
```

### Attach to a running daemon

```bash
wrangler --attach
```

If a daemon is already running, launching `wrangler` without flags automatically attaches instead of starting a second monitor.

## Configuration

Settings are stored at `~/.config/wrangler/config.toml`:

```toml
threshold = 80.0
interval_ms = 1000
use_cgroups = false
```

CLI flags override the file for that invocation. Threshold changes from the dashboard are saved automatically.

## Systemd user service

```bash
make install-systemd
systemctl --user enable --now wrangler.service
```

Unit file: `contrib/systemd/user/wrangler.service`

## CLI reference

| Flag | Description |
|------|-------------|
| `--threshold` | CPU % threshold (default: from config or 80) |
| `--interval` | Poll interval in ms (default: from config or 1000) |
| `--cgroups` | Use cgroups v2 instead of signals |
| `--daemon` | Run monitor/throttle in background |
| `--no-tray` | Daemon without system tray |
| `--attach` | Connect TUI to running daemon |

## Permissions

- **Signal throttling** works on processes owned by your user. Root-owned processes require matching privileges.
- **cgroups v2** requires root. Example: `sudo wrangler --daemon --cgroups`
- Do not use `setcap cap_sys_ptrace` on the binary; prefer matching UID or cgroups as root.

## Testing

```bash
make test
make clippy
```

## Architecture

```
Monitor (sysinfo) ──► Event Hub ◄── TUI / IPC clients
                         │
                    Throttle engine
                    (signals or cgroups)
```

The daemon exposes a Unix socket at `$XDG_RUNTIME_DIR/wrangler.sock` for dashboard attach and threshold sync.

## License

See repository for license details.
