#!/bin/sh
# One-shot publish: create the public GitHub repo if needed, push main, tag the
# current Cargo.toml version, create the release, and upload local macOS builds.
# The tag push also triggers .github/workflows/release.yml, which adds Linux
# builds and re-uploads the macOS ones.
#
# Requires: gh (authenticated), git, cargo with both macOS targets installed.
# Usage: FILEINSPECTOR_REPO=owner/name scripts/publish.sh   (default jantomec/FileInspector)
set -eu
cd "$(dirname "$0")/.."
REPO="${FILEINSPECTOR_REPO:-jantomec/FileInspector}"
VERSION="v$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"

gh auth status >/dev/null
if ! gh repo view "$REPO" >/dev/null 2>&1; then
  gh repo create "$REPO" --public \
    --description "Interactive disk-usage explorer for the terminal (fileinspector)"
fi
git remote get-url origin >/dev/null 2>&1 || git remote add origin "https://github.com/$REPO.git"
git push -u origin HEAD:main

if ! git rev-parse -q --verify "refs/tags/$VERSION" >/dev/null; then
  git tag -a "$VERSION" -m "fileinspector $VERSION"
fi
git push origin "$VERSION"

for target in aarch64-apple-darwin x86_64-apple-darwin; do
  sh scripts/package.sh "$target" "$VERSION"
done

gh release view "$VERSION" >/dev/null 2>&1 \
  || gh release create "$VERSION" --title "fileinspector $VERSION" --generate-notes
gh release upload "$VERSION" dist/*.tar.gz dist/*.sha256 --clobber
echo "published https://github.com/$REPO/releases/tag/$VERSION"
