SHELL      := /bin/bash
NASM       := nasm
CARGO      := cargo
BUILD_DIR  := build
BOOT_BIN   := $(BUILD_DIR)/boot.bin
STAGE2_BIN := $(BUILD_DIR)/stage2.bin
KERNEL_ELF := kernel/target/x86_64-kernel/release/kernel
DISK_IMG   := disk.img
QEMU       ?= qemu-system-x86_64
QEMU_FLAGS ?= -serial stdio -debugcon stdio -device isa-debug-exit,iobase=0xf4,iosize=0x04 -no-reboot -no-shutdown -device qemu-xhci -device usb-kbd
FEATURES   ?=

all: $(DISK_IMG)

$(BUILD_DIR):
	mkdir -p $(BUILD_DIR)

$(BOOT_BIN): boot/boot.asm $(STAGE2_BIN) | $(BUILD_DIR)
	@STAGE2_SIZE=$$(wc -c < $(STAGE2_BIN)); \
	STAGE2_SECTORS=$$(( (STAGE2_SIZE + 511) / 512 )); \
	$(NASM) -f bin $< -o $@ -DSTAGE2_SECTORS=$$STAGE2_SECTORS

$(STAGE2_BIN): stage2/stage2.asm | $(BUILD_DIR)
	$(NASM) -f bin $< -o $@.tmp
	@STAGE2_SIZE=$$(wc -c < $@.tmp); \
	STAGE2_SECTORS=$$(( (STAGE2_SIZE + 511) / 512 )); \
	KERNEL_LBA=$$((1 + STAGE2_SECTORS)); \
	$(NASM) -f bin $< -o $@ -DKERNEL_LBA=$$KERNEL_LBA
	rm -f $@.tmp

$(KERNEL_ELF):
	cd kernel && $(CARGO) +nightly build --release -Zbuild-std=core,compiler_builtins -Zbuild-std-features=compiler-builtins-mem --features "$(FEATURES)"

$(DISK_IMG): $(BOOT_BIN) $(STAGE2_BIN) $(KERNEL_ELF) $(INITRD_IMG)
	@STAGE2_SIZE=$$(wc -c < $(STAGE2_BIN)); \
	STAGE2_SECTORS=$$(( (STAGE2_SIZE + 511) / 512 )); \
	KERNEL_LBA=$$((1 + STAGE2_SECTORS)); \
	KERNEL_SIZE=$$(wc -c < $(KERNEL_ELF)); \
	KERNEL_SECTORS=$$(( (KERNEL_SIZE + 511) / 512 )); \
	INITRD_SIZE=$$(wc -c < $(INITRD_IMG) 2>/dev/null || echo 0); \
	INITRD_LBA=$$((KERNEL_LBA + KERNEL_SECTORS)); \
	dd if=/dev/zero of=$@ bs=512 count=8192 conv=notrunc; \
	dd if=$(BOOT_BIN) of=$@ conv=notrunc; \
	dd if=$(STAGE2_BIN) of=$@ bs=512 seek=1 conv=notrunc; \
	dd if=$(KERNEL_ELF) of=$@ bs=512 seek=$$KERNEL_LBA conv=notrunc; \
	if [ "$$INITRD_SIZE" -gt 0 ]; then \
	  python3 -c 'import sys,struct;sys.stdout.buffer.write(struct.pack("<Q", int(sys.argv[1])))' $$INITRD_SIZE | \
	    dd of=$@ bs=512 seek=$$INITRD_LBA conv=notrunc; \
	  INITRD_DATA_LBA=$$((INITRD_LBA + 1)); \
	  dd if=$(INITRD_IMG) of=$@ bs=512 seek=$$INITRD_DATA_LBA conv=notrunc; \
	fi

run: $(DISK_IMG)
	$(QEMU) -drive file=$(DISK_IMG),format=raw $(QEMU_FLAGS)

smoke:
	$(MAKE) clean
	$(MAKE) FEATURES=qemu_exit all
	bash scripts/test-smoke.sh

clean:
	rm -rf $(BUILD_DIR) $(DISK_IMG)
	rm -f *.log qemu.serial
	cd kernel && $(CARGO) clean

.PHONY: all clean

# --- Initrd packaging (cpio newc) ---
INITRD_IMG := initrd.img
AI_MOD     ?= ai.mod

initrd: $(INITRD_IMG)

$(INITRD_IMG): $(AI_MOD)
	rm -rf initrd && mkdir -p initrd
	cp $(AI_MOD) initrd/
	( cd initrd && find . | cpio -o -H newc > ../$(INITRD_IMG) )
	rm -rf initrd
	@echo "Built $(INITRD_IMG) with $(AI_MOD)"
