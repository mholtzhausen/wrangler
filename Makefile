CARGO ?= cargo
BINARY := wrangler
TARGET_DEBUG := target/debug/$(BINARY)
TARGET_RELEASE := target/release/$(BINARY)

.PHONY: all build release run run-release test check clippy fmt clean install help

all: build

build:
	$(CARGO) build

release:
	$(CARGO) build --release

run: build
	$(CARGO) run

run-release: release
	$(TARGET_RELEASE)

test:
	$(CARGO) test

check:
	$(CARGO) check

clippy:
	$(CARGO) clippy --all-targets -- -D warnings

fmt:
	$(CARGO) fmt --all

clean:
	$(CARGO) clean

install: release
	$(CARGO) install --path . --force

help:
	@echo "Wrangler — process monitor and CPU throttle TUI"
	@echo ""
	@echo "Targets:"
	@echo "  make build        Build debug binary ($(TARGET_DEBUG))"
	@echo "  make release      Build optimized binary ($(TARGET_RELEASE))"
	@echo "  make run          Build and run via cargo"
	@echo "  make run-release  Run release binary directly"
	@echo "  make test         Run tests"
	@echo "  make check        Type-check without producing binaries"
	@echo "  make clippy       Lint with clippy (-D warnings)"
	@echo "  make fmt          Format source with rustfmt"
	@echo "  make clean        Remove build artifacts"
	@echo "  make install      Install release binary to ~/.cargo/bin"
	@echo ""
	@echo "Runtime flags (examples):"
	@echo "  cargo run -- --threshold 50 --interval 500"
	@echo "  sudo $(TARGET_RELEASE) --cgroups --threshold 80"
