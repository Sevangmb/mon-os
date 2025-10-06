#!/usr/bin/env bash
set -euo pipefail

if ! command -v nasm >/dev/null 2>&1; then echo "Missing nasm" >&2; exit 1; fi
if ! command -v make >/dev/null 2>&1; then echo "Missing make" >&2; exit 1; fi
if ! command -v ld >/dev/null 2>&1; then echo "Missing ld (binutils)" >&2; exit 1; fi
if ! command -v cargo >/dev/null 2>&1; then echo "Missing cargo/rustup" >&2; exit 1; fi

echo "Building disk image..."
make -j"${JOBS:-$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 1)}"
echo "Done: disk.img"

