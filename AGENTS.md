# Repository Guidelines

## Project Structure & Module Organization
The boot flow is split across `boot/boot.asm` (MBR boot sector) and `stage2/stage2.asm` (transition to long mode and load the kernel). The Rust kernel lives in `kernel/` with `src/main.rs` orchestrating modules such as `gdt.rs`, `idt.rs`, and `serial.rs`. Linker and target metadata reside in `linker.ld` and `kernel/x86_64-kernel.json`. Build outputs land in `build/` and the final `disk.img`; remove them before committing. Place new low-level assets next to their stage (`boot/` for real-mode code, `kernel/src/` for Rust subsystems) and keep emulator artefacts like `qemu-ide.log` out of Git.

## Build, Test, and Development Commands
Run `make` to assemble both stages, compile the kernel with nightly Rust, and emit `disk.img` under `build/`. Use `make clean` to drop artefacts. Development inside `kernel/` supports direct cargo invocations, e.g. `cargo +nightly build --release --target x86_64-kernel.json` from that directory. Launch the image with `qemu-system-x86_64 -drive file=disk.img,format=raw -serial stdio` to observe the serial log.

## Coding Style & Naming Conventions
Assembly files follow NASM syntax with uppercase mnemonics and lowercase labels; document new routines with succinct comments. Rust modules are `#![no_std]`; run `cargo fmt` before committing and favor `snake_case` module names (`kernel/src/interrupts.rs`). Keep public interfaces small and gate experimental code behind `cfg` flags so release builds stay minimal.

## Testing Guidelines
There is no automated harness yet, so lean on QEMU boots as smoke tests. When adding pure logic, place it in `kernel/src/<module>.rs` and add `#[cfg(test)]` unit tests guarded by `#![cfg_attr(test, no_main)]`, which you can run with `cargo +nightly test --lib`. Capture manual verification steps in the PR (e.g. serial output transcript) to keep regressions traceable.

## Commit & Pull Request Guidelines
Write commits in imperative mood (`boot: mask interrupts before load`) and limit subjects to 72 characters. Each PR should link to its issue, describe observable behaviour changes, note any tooling requirements (nightly updates, new QEMU flags), and attach the latest emulator output. Flag boot protocol or build-script changes with `[BREAKING]` so reviewers can coordinate downstream images.
