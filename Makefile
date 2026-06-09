CARGO ?= cargo
BINARY := wrangler
TARGET_DEBUG := target/debug/$(BINARY)
TARGET_RELEASE := target/release/$(BINARY)

VERSION := $(shell grep '^version' Cargo.toml | head -1 | sed -E 's/.*"([^"]+)".*/\1/')
RELEASE_TARBALL := dist/wrangler-$(VERSION)-linux-x86_64.tar.gz

.PHONY: all build release release-dist run run-release run-daemon run-daemon-release test check clippy fmt fmt-check ci e2e e2e-multiproc e2e-cgroup clean install install-systemd help

all: build

build:
	$(CARGO) build

release:
	$(CARGO) build --release

release-dist: release
	mkdir -p dist
	tar czf $(RELEASE_TARBALL) -C target/release wrangler
	@echo "Built $(RELEASE_TARBALL)"

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

e2e-multiproc:
	bash scripts/e2e-multiproc.sh

e2e-cgroup:
	bash scripts/e2e-cgroup.sh

clean:
	$(CARGO) clean

install: release
	$(CARGO) install --path . --force

install-systemd: release
	$(TARGET_RELEASE) service install

help:
	@echo "Wrangler — process monitor and CPU throttle TUI"
	@echo ""
	@echo "Targets:"
	@echo "  make build        Build debug binary ($(TARGET_DEBUG))"
	@echo "  make release      Build optimized binary ($(TARGET_RELEASE))"
	@echo "  make release-dist Build $(RELEASE_TARBALL)"
	@echo "  make run          Build and run via cargo"
	@echo "  make run-release  Run release binary directly"
	@echo "  make run-daemon   Run background daemon with system tray"
	@echo "  make run-daemon-release  Run release daemon"
	@echo "  make install-systemd     Install user systemd unit"
	@echo "  curl -fsSL .../scripts/install.sh | bash   Install latest release"
	@echo "  wrangler install --sudo           Install local build to /usr/local/bin"
	@echo "  wrangler service install          Install user systemd unit (tray)"
	@echo "  wrangler service install --sudo   Install system systemd unit (cgroups + tray)"
	@echo "  wrangler kill [--sudo|--all]        Stop running wrangler processes"
	@echo "  make test         Run tests"
	@echo "  make ci           Run fmt-check, clippy, and test (matches CI check job)"
	@echo "  make e2e          Run end-to-end throttle smoke test (needs stress-ng)"
	@echo "  make e2e-multiproc  E2E multi-process app group throttling"
	@echo "  make e2e-cgroup     E2E cgroup backend (root only; skips otherwise)"
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
