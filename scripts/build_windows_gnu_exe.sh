#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="x86_64-pc-windows-gnu"
PROFILE="${BUILD_WINDOWS_GNU_PROFILE:-release}"
BIN_NAME="oryx"
STATIC_CRT="${BUILD_WINDOWS_GNU_STATIC_CRT:-1}"

usage() {
  cat <<'EOF'
Build the Oryx Windows GNU executable without touching the network.

Usage:
  ./scripts/build_windows_gnu_exe.sh
  LLVM_MINGW_ROOT=/opt/llvm-mingw ./scripts/build_windows_gnu_exe.sh
  BUILD_WINDOWS_GNU_PROFILE=release ./scripts/build_windows_gnu_exe.sh
  BUILD_WINDOWS_GNU_STATIC_CRT=0 ./scripts/build_windows_gnu_exe.sh

Environment:
  LLVM_MINGW_ROOT
      Optional root of an llvm-mingw installation. Its bin directory will be
      added to PATH before tool detection.

  BUILD_WINDOWS_GNU_PROFILE
      Defaults to dev. Use release only on a native Windows build or after
      patching gpui, because gpui 0.2.2 does not generate its embedded shader
      blob correctly when cross-compiling Windows release builds from macOS.

  BUILD_WINDOWS_GNU_STATIC_CRT
      Defaults to 1. When set to 1, enables static CRT linking to reduce extra
      runtime DLL requirements for the first test binary.
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

if [[ -n "${LLVM_MINGW_ROOT:-}" ]]; then
  export PATH="${LLVM_MINGW_ROOT}/bin:${PATH}"
fi

require_command() {
  local cmd="$1"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "Missing required command: $cmd" >&2
    exit 1
  fi
}

require_command cargo
require_command rustup

if ! rustup target list --installed | grep -qx "${TARGET}"; then
  cat >&2 <<EOF
Rust target ${TARGET} is not installed.
Run the bootstrap script first:
  ./scripts/bootstrap_windows_gnu_toolchain.sh
EOF
  exit 1
fi

find_first_tool() {
  local tool
  for tool in "$@"; do
    if command -v "$tool" >/dev/null 2>&1; then
      command -v "$tool"
      return 0
    fi
  done
  return 1
}

LINKER="$(find_first_tool x86_64-w64-mingw32-clang x86_64-w64-mingw32-gcc)"
if [[ -z "${LINKER:-}" ]]; then
  cat >&2 <<'EOF'
Could not find a Windows GNU linker.
Expected one of:
  x86_64-w64-mingw32-clang
  x86_64-w64-mingw32-gcc

If llvm-mingw is installed in a custom location, set LLVM_MINGW_ROOT.
EOF
  exit 1
fi

AR_TOOL="$(find_first_tool llvm-ar x86_64-w64-mingw32-ar ar || true)"
WINDRES_TOOL="$(find_first_tool llvm-windres x86_64-w64-mingw32-windres windres || true)"

if [[ "${LINKER}" == *clang ]]; then
  CXX_TOOL="$(find_first_tool x86_64-w64-mingw32-clang++ clang++ || true)"
else
  CXX_TOOL="$(find_first_tool x86_64-w64-mingw32-g++ g++ || true)"
fi

export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="${LINKER}"
if [[ -n "${AR_TOOL}" ]]; then
  export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_AR="${AR_TOOL}"
fi
if [[ -n "${WINDRES_TOOL}" ]]; then
  export RC="${WINDRES_TOOL}"
fi
export CC_x86_64_pc_windows_gnu="${LINKER}"
if [[ -n "${CXX_TOOL:-}" ]]; then
  export CXX_x86_64_pc_windows_gnu="${CXX_TOOL}"
fi

RUSTFLAGS_VALUE="${RUSTFLAGS:-}"
if [[ "${STATIC_CRT}" == "1" ]]; then
  if [[ -n "${RUSTFLAGS_VALUE}" ]]; then
    RUSTFLAGS_VALUE="${RUSTFLAGS_VALUE} "
  fi
  RUSTFLAGS_VALUE="${RUSTFLAGS_VALUE}-C target-feature=+crt-static"
fi
if [[ -n "${RUSTFLAGS_VALUE}" ]]; then
  export RUSTFLAGS="${RUSTFLAGS_VALUE}"
fi

cd "${ROOT_DIR}"
if [[ "${PROFILE}" == "dev" ]]; then
  cargo build --target "${TARGET}" --bin "${BIN_NAME}"
  OUTPUT_SUBDIR="debug"
else
  cargo build --profile "${PROFILE}" --target "${TARGET}" --bin "${BIN_NAME}"
  OUTPUT_SUBDIR="${PROFILE}"
fi

OUTPUT_PATH="${ROOT_DIR}/target/${TARGET}/${OUTPUT_SUBDIR}/${BIN_NAME}.exe"
if [[ ! -f "${OUTPUT_PATH}" ]]; then
  echo "Build completed but ${OUTPUT_PATH} was not found." >&2
  exit 1
fi

echo "Built ${OUTPUT_PATH}"
