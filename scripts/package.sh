#!/bin/sh
# Build (unless --no-build) and package fi for one target into dist/.
# Usage: scripts/package.sh <target-triple> [version-tag] [--no-build]
set -eu
target="${1:?target triple required, e.g. aarch64-apple-darwin}"
version="${2:-v$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)}"
build=1
for arg in "$@"; do [ "$arg" = "--no-build" ] && build=0; done
if [ "$build" = 1 ]; then
  cargo build --release --locked --target "$target"
fi
name="fi-$version-$target"
rm -rf "dist/$name"
mkdir -p "dist/$name"
cp "target/$target/release/fi" README.md LICENSE "dist/$name/"
tar -C dist -czf "dist/$name.tar.gz" "$name"
rm -rf "dist/$name"
if command -v shasum >/dev/null 2>&1; then
  (cd dist && shasum -a 256 "$name.tar.gz" > "$name.tar.gz.sha256")
else
  (cd dist && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")
fi
echo "packaged dist/$name.tar.gz"
