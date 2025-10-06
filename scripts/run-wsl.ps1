# Runs mon-os build and QEMU from WSL (Ubuntu) with minimal setup.
# Usage examples (from PowerShell):
#   scripts\run-wsl.ps1                 # build + run with AI (default)
#   scripts\run-wsl.ps1 -KernelOnly     # build + run kernel only (no AI)
#   scripts\run-wsl.ps1 -Preset aggr    # AI with aggressive preset
#   scripts\run-wsl.ps1 -Preset conservative

[CmdletBinding()]
param(
  [ValidateSet('Ubuntu')]
  [string] $Distro = 'Ubuntu',

  [switch] $KernelOnly,

  [ValidateSet('default','aggr','conservative')]
  [string] $Preset = 'default',

  [string] $Repo
)

function Invoke-WSL {
  param([string]$Command)
  & wsl -d $Distro -- bash -lc $Command
}

if (-not (Get-Command wsl -ErrorAction SilentlyContinue)) {
  Write-Error 'WSL is not available on this system. Install WSL and Ubuntu first: wsl --install -d Ubuntu'; exit 1
}

try { Invoke-WSL 'true' | Out-Null } catch { Write-Error "WSL distro '$Distro' not found. Install via: wsl --install -d Ubuntu"; exit 1 }


# Resolve repository path (default: parent of this script)
try {
  if ([string]::IsNullOrWhiteSpace($Repo)) {
    $RepoRoot = Resolve-Path (Join-Path $PSScriptRoot '..')
  } else {
    $RepoRoot = Resolve-Path $Repo
  }
} catch {
  Write-Error "Invalid repository path. Pass -Repo 'C:\\Users\\...\\mon-os' or run the script from the repo."; exit 1
}

# Convert Windows path to WSL path
try {
  $RepoWSL = (wsl -d $Distro -- wslpath -a -u "${RepoRoot}").Trim()
} catch {
  Write-Error "wslpath failed. Ensure the Windows path is valid (e.g. C:\\Users\\sevans\\Desktop\\dev\\mon-os)."; exit 1
}

if ([string]::IsNullOrWhiteSpace($RepoWSL)) {
  Write-Error "Could not convert repo path to WSL path. Try: scripts\\run-wsl.ps1 -Repo 'C:\\Users\\sevans\\Desktop\\dev\\mon-os'"; exit 1
}

Write-Host "Repo (Windows): $RepoRoot" -ForegroundColor Cyan
Write-Host "Repo (WSL)    : $RepoWSL" -ForegroundColor Cyan

$BaseSetup = @'
set -e
sudo apt-get update
sudo apt-get install -y build-essential nasm qemu-system-x86 binutils make coreutils cpio python3
if [ ! -x "$HOME/.cargo/bin/rustup" ]; then curl https://sh.rustup.rs -sSf | sh -s -- -y; fi
source "$HOME/.cargo/env"
rustup toolchain install nightly
rustup component add rust-src --toolchain nightly
'@

Invoke-WSL $BaseSetup

if ($KernelOnly) {
  $Cmd = @"
set -e
cd $RepoWSL
make clean
make
make run
"@
  Invoke-WSL $Cmd
  exit 0
}

switch ($Preset) {
  'default' {
    $Cmd = @"
set -e
cd $RepoWSL
make run-ai
"@
  }
  'aggr' {
    $Cmd = @"
set -e
cd $RepoWSL
make ai
make initrd
make FEATURES="ai_agent,ai_cfg_aggr" disk.img
qemu-system-x86_64 \
  -drive file=disk.img,format=raw \
  -m 2048 -smp 2 -enable-kvm -cpu host -net none \
  -serial file:ai_journal.log -debugcon file:debugcon.log \
  -device isa-debug-exit,iobase=0xf4,iosize=0x04 -no-reboot -no-shutdown \
  -device qemu-xhci -device usb-kbd
"@
  }
  'conservative' {
    $Cmd = @"
set -e
cd $RepoWSL
make ai
make initrd
make FEATURES="ai_agent,ai_cfg_conservative" disk.img
qemu-system-x86_64 \
  -drive file=disk.img,format=raw \
  -m 2048 -smp 2 -enable-kvm -cpu host -net none \
  -serial file:ai_journal.log -debugcon file:debugcon.log \
  -device isa-debug-exit,iobase=0xf4,iosize=0x04 -no-reboot -no-shutdown \
  -device qemu-xhci -device usb-kbd
"@
  }
}

Invoke-WSL $Cmd
