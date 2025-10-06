#!/usr/bin/env bash
set -euo pipefail

IMG=${1:-disk.img}
QEMU=${QEMU:-qemu-system-x86_64}

if ! command -v "$QEMU" >/dev/null 2>&1; then echo "Missing qemu-system-x86_64" >&2; exit 1; fi
if [ ! -f "$IMG" ]; then echo "Missing $IMG (run make)" >&2; exit 1; fi

TMP_LOG=$(mktemp)
cleanup() { rm -f "$TMP_LOG"; }
trap cleanup EXIT

set +e
"$QEMU" \
  -drive file="$IMG",format=raw \
  -serial file:"$TMP_LOG" -debugcon file:"$TMP_LOG" \
  -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
  -display none -no-reboot -no-shutdown \
  -vga none -machine accel=tcg 2>/dev/null
QEMU_RC=$?
set -e

echo "--- Serial/Debug output ---"
cat "$TMP_LOG" || true
echo "---------------------------"

if ! grep -q "Hello Kernel" "$TMP_LOG"; then
  echo "Smoke test failed: missing 'Hello Kernel' in output" >&2
  exit 1
fi

EXIT_CODE=$(( (QEMU_RC >> 1) & 0xFF ))
if [ "$EXIT_CODE" != "0" ]; then
  echo "QEMU exited with non-zero code: $EXIT_CODE" >&2
  exit 1
fi

echo "Smoke test passed"

