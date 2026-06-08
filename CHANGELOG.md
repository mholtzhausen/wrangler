# Changelog

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
