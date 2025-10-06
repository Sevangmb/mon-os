#!/usr/bin/env bash
set -euo pipefail

IMG=${1:-disk.img}
QEMU=${QEMU:-qemu-system-x86_64}

if [ ! -f "$IMG" ]; then
  echo "Missing $IMG. Build first (make)." >&2
  exit 1
fi

exec "$QEMU" \
  -drive file="$IMG",format=raw \
  -serial stdio -debugcon stdio \
  -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
  -no-reboot -no-shutdown "$@"

