# Changelog

## 1.2.0 (1465339)

### Features and Improvements

- `--sudo` flag for daemon/tray mode: re-execs via `sudo -E` and enables cgroups as root
- Root daemons spawn a `--tray-client` in the desktop user session so `sudo wrangler --tray` shows the tray icon
- Session restore sets `WRANGLER_SOCKET_UID` for IPC socket ownership under sudo

## 1.1.1 (fccffca)

### Bugfixes

- Curl install script stops running wrangler instances before upgrading the binary
- `wrangler install --sudo` honors `WRANGLER_INSTALL_DIR` (same target as the install script)

## 1.1.0 (a7c0f1d)

### Features and Improvements

- Bad actors panel replaces mitigation logs; tracks per-group throttle counts, peak/last CPU, and cumulative throttle time
- Scalable footer panel grows up to 10 lines or half the terminal height
- `wrangler kill` command to stop user, root, or all wrangler instances (`--sudo`, `--all`)
- MIT `LICENSE` and personal-use README footer

## 1.0.1 (b3af944)

### Bugfixes

- Close attached dashboards when quitting from the tray or when the daemon shuts down

## 1.0.0 (3c3336f)

### Features and Improvements

- App-group throttling with pressure gate and per-machine CPU budgets
- cgroups v2 per-app-group `cpu.max` when running as root
- Grouped dashboard (default) with flat toggle, expand/collapse, and selection highlight
- CPU %, Machine %, User, and Group columns in the process table
- Protected app list, tree/name grouping modes, and legacy config migration
- Background daemon with system tray, `--tray` alias, and terminal detach
- Systemd user and system service commands (`wrangler service install`)
- Split tray client for root system service (IPC-connected tray icon)
- Binary install commands (`wrangler install` / `wrangler uninstall`)
- Curl install script for GitHub releases
- Scrollbar, attachable dashboard, and `wrangler --status` JSON output

### Bugfixes

- Restore desktop session environment when launched via `sudo`
- Chown IPC socket to the desktop user so tray clients can connect
- Fix system tray unavailable under root systemd service

## 0.1.0

Initial release — process monitor and CPU throttle TUI with daemon and tray support.
