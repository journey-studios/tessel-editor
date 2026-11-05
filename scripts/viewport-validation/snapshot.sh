#!/bin/sh
# Copies the sources that define a build into $TEV/snap/<fingerprint>, with a hash manifest.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd); TEV=${TEV:-/tmp/tev}; cd "$ROOT"; mkdir -p "$TEV/snap"
find crates runtime/src runtime/worker runtime/host runtime/scripts runtime/test runtime/package.json runtime/pnpm-lock.yaml \
  Cargo.toml Cargo.lock rust-toolchain.toml .mbx.toml assets scripts -type f -not -name '*.pyc' -not -path 'scripts/viewport-validation/*' | LC_ALL=C sort > "$TEV/files.txt"
xargs -I{} shasum -a 256 "{}" < "$TEV/files.txt" > "$TEV/manifest.txt"
FP=$(shasum -a 256 "$TEV/manifest.txt" | cut -c1-16); S="$TEV/snap/$FP"; mkdir -p "$S"
rsync -a --files-from="$TEV/files.txt" ./ "$S/"; cp "$TEV/manifest.txt" "$S/MANIFEST.sha256"; date +%Y-%m-%dT%H:%M:%S%z > "$S/SNAPSHOT_TIME"
[ -e "$S/runtime/node_modules" ] || ln -s "$ROOT/runtime/node_modules" "$S/runtime/node_modules"
echo "$S"
