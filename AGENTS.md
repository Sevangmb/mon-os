# Repository Guidelines

## Project Structure & Module Organization
- `boot/boot.asm`: MBR boot sector (NASM).
- `stage2/stage2.asm`: passage en long mode et chargement du noyau.
- `kernel/`: noyau Rust `#![no_std]` (modules: `gdt.rs`, `idt.rs`, `pmm.rs`, `pci.rs`, `xhci.rs`, `vga.rs`, etc.). Point d’entrée: `src/main.rs`.
- `linker.ld`, `kernel/x86_64-kernel.json`: script d’édition de liens et cible Rust.
- `scripts/`: helpers (`build.sh`, `run-qemu.sh`, `test-smoke.sh`).
- Artefacts: `build/` et `disk.img` (ne pas committer).

## Build, Test, and Development Commands
- `make`: assemble les étapes, compile le kernel (nightly) et produit `disk.img`.
- `make run`: lance QEMU avec série/debugcon et périphériques USB (`qemu-xhci`, `usb-kbd`).
- `make smoke`: test headless, quitte QEMU via `isa-debug-exit` si OK.
- `make clean`: nettoie les artefacts.
- Développement kernel: `cd kernel && cargo +nightly build --release -Z build-std=core,compiler_builtins -Z build-std-features=compiler-builtins-mem --target x86_64-kernel.json`.

## Coding Style & Naming Conventions
- NASM: mnémoniques MAJUSCULES, labels minuscules; commentaires brefs sur les routines.
- Rust: `snake_case` pour fichiers/modules; API publiques minimales; garder les nouveautés derrière des `cfg` si expérimental.
- Formatage: exécuter `cargo fmt` dans `kernel/` avant commit.

## Testing Guidelines
- Pas de harnais complet: utiliser `make run` et `make smoke` comme tests fumée. Conserver les logs série/debugcon.
- Pour la logique pure, ajouter des tests unitaires sous `kernel/src/<module>.rs` avec `#[cfg(test)]` et documenter dans la PR comment reproduire.

## Commit & Pull Request Guidelines
- Messages impératifs et concis (≤72 caractères): `boot: mask interrupts before load`.
- PR: lier l’issue, décrire les changements observables, préciser les exigences (nightly, flags QEMU) et joindre une sortie QEMU récente.
- Marquer `[BREAKING]` pour toute modification du protocole d’amorçage ou du pipeline de build.

## Agent-Specific Instructions
- Placez le code au bon niveau: assembleur sous `boot/`/`stage2/`, Rust sous `kernel/src/`.
- N’émettez pas `disk.img`/`build/` dans Git; respectez ce guide dans tout le sous-arbre.
- Avant de pousser: `make`, `make run` (ou `make smoke`), et formatez le code.
