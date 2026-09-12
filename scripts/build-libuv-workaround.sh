#!/usr/bin/env bash
# Build the libuv-1.44.2 workaround library.
#
# Why: the precompiled MapLibre core bundles its own libuv while also
# linking the system one (>= 1.51); the version-mixed loop aborts in
# io_uring_enter. Preloading 1.44.2 restores a consistent loop.
# Output: <repo>/target/libuv-1.44.2/build/libuv.so.1 (gitignored).
# Real fix: from-source core build (map fog), not this shim.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/target/libuv-1.44.2"
VER="1.44.2"
if [ -f "$OUT/build/libuv.so.1" ]; then
  echo "already built: $OUT/build/libuv.so.1"
  exit 0
fi
rm -rf /tmp/libuv-$VER /tmp/libuv-$VER.tar.gz
curl -sL -A "tfg/0.1" "https://github.com/libuv/libuv/archive/refs/tags/v$VER.tar.gz" \
  -o /tmp/libuv-$VER.tar.gz
tar xzf /tmp/libuv-$VER.tar.gz -C /tmp
cmake -S /tmp/libuv-$VER -B /tmp/libuv-$VER/build \
  -DCMAKE_BUILD_TYPE=Release -DCMAKE_POLICY_VERSION_MINIMUM=3.5 > /dev/null
cmake --build /tmp/libuv-$VER/build -j"$(nproc)" > /dev/null
mkdir -p "$OUT"
cp -r /tmp/libuv-$VER/build "$OUT/build"
echo "built: $OUT/build/libuv.so.1"
