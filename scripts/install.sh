#!/usr/bin/env bash
# Install wrangler from the latest GitHub release into /usr/local/bin.
#
#   curl -fsSL https://raw.githubusercontent.com/mholtzhausen/wrangler/main/scripts/install.sh | bash
#
# Optional environment variables:
#   WRANGLER_REPO=owner/repo     GitHub repository (default: mholtzhausen/wrangler)
#   WRANGLER_VERSION=0.1.0       Pin a release version instead of latest
#   WRANGLER_INSTALL_DIR=...     Target directory (default: /usr/local/bin)
#   GITHUB_TOKEN=...             Optional token for GitHub API (rate limits / private repos)

set -euo pipefail

REPO="${WRANGLER_REPO:-mholtzhausen/wrangler}"
INSTALL_DIR="${WRANGLER_INSTALL_DIR:-/usr/local/bin}"
BINARY_NAME="wrangler"

log() {
	printf '==> %s\n' "$*"
}

die() {
	printf 'error: %s\n' "$*" >&2
	exit 1
}

need_cmd() {
	command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

detect_arch() {
	local machine
	machine="$(uname -m)"
	case "$machine" in
	x86_64 | amd64) echo "x86_64" ;;
	aarch64 | arm64) echo "aarch64" ;;
	*) die "unsupported architecture: $machine (linux x86_64 and aarch64 only)" ;;
	esac
}

github_curl() {
	local url="$1"
	local -a args=(-fsSL)
	if [[ -n "${GITHUB_TOKEN:-}" ]]; then
		args+=(-H "Authorization: Bearer ${GITHUB_TOKEN}")
	fi
	curl "${args[@]}" "$url"
}

release_api_url() {
	if [[ -n "${WRANGLER_VERSION:-}" ]]; then
		printf 'https://api.github.com/repos/%s/releases/tags/v%s' "$REPO" "$WRANGLER_VERSION"
	else
		printf 'https://api.github.com/repos/%s/releases/latest' "$REPO"
	fi
}

pinned_asset_url() {
	local arch="$1"
	printf 'https://github.com/%s/releases/download/v%s/wrangler-%s-linux-%s.tar.gz' \
		"$REPO" "$WRANGLER_VERSION" "$WRANGLER_VERSION" "$arch"
}

asset_download_url() {
	local arch="$1"

	if [[ -n "${WRANGLER_VERSION:-}" ]]; then
		pinned_asset_url "$arch"
		return
	fi

	local api_url json
	api_url="$(release_api_url)"
	json="$(github_curl "$api_url")" || die "failed to fetch release metadata from GitHub"

	printf '%s' "$json" | grep -oE "https://github.com/[^\"]+wrangler-[^\"]*linux-${arch}[^\"]*\\.tar\\.gz" | head -n 1
}

has_install_subcommand() {
	local bin="$1"
	"$bin" --help 2>&1 | grep -qE '^  install[[:space:]]'
}

has_kill_subcommand() {
	local bin="$1"
	"$bin" --help 2>&1 | grep -qE '^  kill[[:space:]]'
}

stop_wrangler() {
	local bin="$1"
	if ! [[ -x "$bin" ]]; then
		return 0
	fi
	if ! has_kill_subcommand "$bin"; then
		return 0
	fi
	log "stopping wrangler instances via ${bin}"
	WRANGLER_INSTALL_DIR="$INSTALL_DIR" "$bin" kill --all 2>/dev/null || true
}

main() {
	need_cmd curl
	need_cmd tar
	need_cmd chmod

	if [[ "$(uname -s)" != "Linux" ]]; then
		die "wrangler install script supports Linux only"
	fi

	local arch asset_url tmpdir bin_path
	arch="$(detect_arch)"
	asset_url="$(asset_download_url "$arch")"
	[[ -n "$asset_url" ]] || die "no linux-${arch} release asset found for ${REPO}"

	log "downloading ${asset_url}"
	tmpdir="$(mktemp -d)"
	trap 'rm -rf "$tmpdir"' EXIT

	curl -fsSL -o "${tmpdir}/wrangler.tar.gz" "$asset_url"
	tar xzf "${tmpdir}/wrangler.tar.gz" -C "$tmpdir"
	chmod +x "${tmpdir}/${BINARY_NAME}"

	bin_path="${INSTALL_DIR}/${BINARY_NAME}"
	stop_wrangler "$bin_path"
	stop_wrangler "${tmpdir}/${BINARY_NAME}"

	export WRANGLER_INSTALL_DIR="$INSTALL_DIR"
	if has_install_subcommand "${tmpdir}/${BINARY_NAME}"; then
		log "installing to ${bin_path} via wrangler install --sudo"
		"${tmpdir}/${BINARY_NAME}" install --sudo
	else
		log "release predates wrangler install; copying binary to ${bin_path}"
		need_cmd sudo
		sudo install -m 755 "${tmpdir}/${BINARY_NAME}" "$bin_path"
	fi

	log "installed ${bin_path}"
	log "run: wrangler --tray   or: wrangler service install --sudo"
}

main "$@"
