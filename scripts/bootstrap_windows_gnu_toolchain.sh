#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="x86_64-pc-windows-gnu"

cat <<'EOF'
This script installs the Windows GNU cross-compilation prerequisites.
It may touch the network and should be run only when you are ready to allow
that, ideally with the environment/proxy settings you need.
EOF

require_command() {
  local cmd="$1"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "Missing required command: $cmd" >&2
    exit 1
  fi
}

require_command rustup

rustup target add "${TARGET}"

if command -v x86_64-w64-mingw32-clang >/dev/null 2>&1 || command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1; then
  echo "Windows GNU linker already present."
elif command -v brew >/dev/null 2>&1; then
  brew install mingw-w64
elif command -v pacman >/dev/null 2>&1; then
  pacman -S --needed llvm-mingw
else
  cat >&2 <<'EOF'
No supported package manager was detected for automatic Windows GNU toolchain install.

Install a Windows GNU cross toolchain manually, then rerun:
  ./scripts/build_windows_gnu_exe.sh

If you install it into a custom location, export:
  LLVM_MINGW_ROOT=/path/to/llvm-mingw
EOF
  exit 1
fi

cat <<EOF
Bootstrap complete.
Next step:
  cd "${ROOT_DIR}"
  ./scripts/build_windows_gnu_exe.sh
EOF
