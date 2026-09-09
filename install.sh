#!/bin/sh
# Install a prebuilt fi binary from GitHub releases.
#   curl -fsSL https://raw.githubusercontent.com/jantomec/FileInspector/main/install.sh | sh
# Environment: FI_REPO (owner/name), FI_VERSION (tag, default latest),
#              FI_INSTALL_DIR (default /usr/local/bin if writable, else ~/.local/bin)
set -eu
REPO="${FI_REPO:-jantomec/FileInspector}"
VERSION="${FI_VERSION:-latest}"

case "$(uname -s)" in
  Darwin) os=apple-darwin ;;
  Linux) os=unknown-linux-gnu ;;
  *) echo "install.sh: unsupported operating system: $(uname -s)" >&2; exit 1 ;;
esac
case "$(uname -m)" in
  arm64 | aarch64) arch=aarch64 ;;
  x86_64 | amd64) arch=x86_64 ;;
  *) echo "install.sh: unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac
target="$arch-$os"

if [ "$VERSION" = latest ]; then
  VERSION=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
    | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -1)
  [ -n "$VERSION" ] || { echo "install.sh: could not determine the latest release" >&2; exit 1; }
fi

name="fi-$VERSION-$target"
base="https://github.com/$REPO/releases/download/$VERSION"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

echo "Downloading $name.tar.gz"
curl -fsSL "$base/$name.tar.gz" -o "$tmp/$name.tar.gz"
curl -fsSL "$base/$name.tar.gz.sha256" -o "$tmp/$name.tar.gz.sha256"
if command -v shasum >/dev/null 2>&1; then
  (cd "$tmp" && shasum -a 256 -c "$name.tar.gz.sha256" >/dev/null)
else
  (cd "$tmp" && sha256sum -c "$name.tar.gz.sha256" >/dev/null)
fi
tar -xzf "$tmp/$name.tar.gz" -C "$tmp"

dest="${FI_INSTALL_DIR:-}"
if [ -z "$dest" ]; then
  if [ -w /usr/local/bin ]; then dest=/usr/local/bin; else dest="$HOME/.local/bin"; fi
fi
mkdir -p "$dest"
install -m 0755 "$tmp/$name/fi" "$dest/fi"
echo "Installed fi $VERSION to $dest/fi"
case ":$PATH:" in
  *":$dest:"*) ;;
  *) echo "Note: $dest is not on your PATH; add it or run $dest/fi directly." ;;
esac
