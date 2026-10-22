#!/bin/sh
# Install tailscale-keepalive from GitHub Releases.
#
#   curl -fsSL https://raw.githubusercontent.com/punisher1/tailscale-keepalive/main/install.sh | sh
#
# Optional environment variables:
#   TS_KEEPALIVE_VERSION         release tag to install (default: latest)
#   TS_KEEPALIVE_INSTALL_DIR     where to put the binary (default: /usr/local/bin, else ~/.local/bin)
#   TS_KEEPALIVE_TARGET          peer IP; written to the service config when installing the service
#   TS_KEEPALIVE_INSTALL_SERVICE set to 1 to also register the system service (needs root)
set -eu

REPO="punisher1/tailscale-keepalive"
BIN="tailscale-keepalive"

say() { printf '%s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

need() {
    command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

need uname
need tar

if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsSL --retry 3 --proto '=https' -o "$2" "$1"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -q -O "$2" "$1"; }
else
    die "need curl or wget"
fi

os=$(uname -s)
arch=$(uname -m)

case "$os" in
    Linux)  os_part="unknown-linux-gnu" ;;
    Darwin) os_part="apple-darwin" ;;
    *)      die "unsupported operating system: $os (this script covers Linux and macOS)" ;;
esac

case "$arch" in
    x86_64|amd64)  arch_part="x86_64" ;;
    arm64|aarch64) arch_part="aarch64" ;;
    *)             die "unsupported architecture: $arch" ;;
esac

target="${arch_part}-${os_part}"

version="${TS_KEEPALIVE_VERSION:-}"
if [ -n "$version" ]; then
    base="https://github.com/${REPO}/releases/download/${version}"
else
    base="https://github.com/${REPO}/releases/latest/download"
fi
url="${base}/${BIN}-${target}.tar.gz"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM

say "downloading ${url}"
fetch "$url" "$tmp/pkg.tar.gz" || die "download failed: ${url}"

tar -xzf "$tmp/pkg.tar.gz" -C "$tmp"
if [ -f "$tmp/$BIN" ]; then
    bin="$tmp/$BIN"
else
    bin=$(find "$tmp" -maxdepth 3 -type f -name "$BIN" | head -n 1)
fi
[ -n "${bin:-}" ] && [ -f "$bin" ] || die "archive did not contain ${BIN}"

if [ -n "${TS_KEEPALIVE_INSTALL_DIR:-}" ]; then
    install_dir="$TS_KEEPALIVE_INSTALL_DIR"
elif [ "$(id -u)" = "0" ]; then
    install_dir="/usr/local/bin"
else
    install_dir="$HOME/.local/bin"
fi

mkdir -p "$install_dir"
install -m 0755 "$bin" "$install_dir/$BIN" 2>/dev/null ||
    { cp "$bin" "$install_dir/$BIN" && chmod 0755 "$install_dir/$BIN"; }

say "installed ${install_dir}/${BIN}"

case ":$PATH:" in
    *":$install_dir:"*) ;;
    *) say "note: ${install_dir} is not in PATH" ;;
esac

if [ "${TS_KEEPALIVE_INSTALL_SERVICE:-0}" = "1" ]; then
    [ "$(id -u)" = "0" ] || die "installing the service needs root; re-run with sudo"
    [ -n "${TS_KEEPALIVE_TARGET:-}" ] || die "set TS_KEEPALIVE_TARGET to the peer's Tailscale IP"

    say "registering the system service"
    "$install_dir/$BIN" -t "$TS_KEEPALIVE_TARGET" install
    say "check it with: $BIN status"
else
    say ""
    say "next steps:"
    say "  ${BIN} -t 100.x.y.z              # try it in the foreground"
    say "  sudo ${BIN} -t 100.x.y.z install # register the service"
fi
