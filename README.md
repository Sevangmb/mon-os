# mon-os

Petit noyau x86_64 écrit en Rust, amorcé via un bootsector (MBR) et un stage2 en assembleur, puis un kernel Rust en long mode.

## Prérequis (Linux/WSL recommandés)

- Outils système: `build-essential nasm qemu-system-x86 binutils coreutils make`
- Rust nightly et Cargo: `rustup` + toolchain `nightly`

Exemple d’installation (Ubuntu/WSL):

```
sudo apt update && sudo apt install -y build-essential nasm qemu-system-x86 binutils make coreutils
curl https://sh.rustup.rs -sSf | sh -s -- -y
source "$HOME/.cargo/env"
rustup toolchain install nightly
```

## Construire l’image

```
make
```

Produit `disk.img` à la racine. Utilisez `make clean` pour nettoyer.

## Exécuter sous QEMU

Rapide (console série dans le terminal):

```
make run
```

Équivalent direct:

```
qemu-system-x86_64 \
  -drive file=disk.img,format=raw \
  -serial stdio -debugcon stdio \
  -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
  -no-reboot -no-shutdown
```

Astuce: le noyau écrit sur le port `0xE9` (debugcon) et COM1; QEMU avec `-debugcon stdio` affiche ces logs.

## Scripts utiles

- `scripts/build.sh`: construit l’image (`make`).
- `scripts/run-qemu.sh`: lance QEMU avec les options recommandées.
- `scripts/test-smoke.sh`: lance QEMU headless et vérifie que "Hello Kernel" apparaît sur la sortie série.

## Intégration Continue

Un workflow GitHub Actions (`.github/workflows/ci.yml`) construit l’image et exécute un smoke test QEMU en CI (Ubuntu).

## Notes

- Le Makefile cible un environnement de type Unix (Linux/WSL). Sous Windows natif, privilégiez WSL pour éviter les divergences d’outils.
- Voir `AGENTS.md` pour l’organisation du projet et les conventions.
