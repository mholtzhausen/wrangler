CARGO ?= cargo
BINARY := wrangler
TARGET_DEBUG := target/debug/$(BINARY)
TARGET_RELEASE := target/release/$(BINARY)

.PHONY: all build release run run-release run-daemon run-daemon-release test check clippy fmt fmt-check ci e2e clean install install-systemd help

all: build

build:
	$(CARGO) build

release:
	$(CARGO) build --release

run: build
	$(CARGO) run

run-release: release
	$(TARGET_RELEASE)

run-daemon: build
	$(CARGO) run -- --daemon

run-daemon-release: release
	$(TARGET_RELEASE) --daemon

test:
	$(CARGO) test

check:
	$(CARGO) check

clippy:
	$(CARGO) clippy --all-targets -- -D warnings

fmt:
	$(CARGO) fmt --all

fmt-check:
	$(CARGO) fmt --all -- --check

ci: fmt-check clippy test

e2e:
	bash scripts/e2e-smoke.sh

clean:
	$(CARGO) clean

install: release
	$(CARGO) install --path . --force

install-systemd: install
	mkdir -p $(HOME)/.config/systemd/user
	sed 's|%h/.cargo/bin/wrangler|$(HOME)/.cargo/bin/wrangler|' contrib/systemd/user/wrangler.service > $(HOME)/.config/systemd/user/wrangler.service
	systemctl --user daemon-reload
	@echo "Installed $(HOME)/.config/systemd/user/wrangler.service"
	@echo "Enable with: systemctl --user enable --now wrangler.service"

help:
	@echo "Wrangler — process monitor and CPU throttle TUI"
	@echo ""
	@echo "Targets:"
	@echo "  make build        Build debug binary ($(TARGET_DEBUG))"
	@echo "  make release      Build optimized binary ($(TARGET_RELEASE))"
	@echo "  make run          Build and run via cargo"
	@echo "  make run-release  Run release binary directly"
	@echo "  make run-daemon   Run background daemon with system tray"
	@echo "  make run-daemon-release  Run release daemon"
	@echo "  make install-systemd     Install user systemd unit"
	@echo "  make test         Run tests"
	@echo "  make ci           Run fmt-check, clippy, and test (matches CI check job)"
	@echo "  make e2e          Run end-to-end throttle smoke test (needs stress-ng)"
	@echo "  make check        Type-check without producing binaries"
	@echo "  make clippy       Lint with clippy (-D warnings)"
	@echo "  make fmt          Format source with rustfmt"
	@echo "  make clean        Remove build artifacts"
	@echo "  make install      Install release binary to ~/.cargo/bin"
	@echo ""
	@echo "Runtime flags (examples):"
	@echo "  cargo run -- --app-cap 50 --interval 500"
	@echo "  sudo $(TARGET_RELEASE) --tray --app-cap 40"
	@echo "  $(TARGET_RELEASE) --tray"
	@echo "  $(TARGET_RELEASE) --daemon --no-tray"
