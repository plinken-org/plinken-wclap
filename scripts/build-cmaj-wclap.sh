#!/usr/bin/env bash
# build-cmaj-wclap.sh — turn a .cmajorpatch into a wasm32 CLAP plugin (module.wasm).
#
# Pipeline:
#
#     .cmajorpatch
#         │
#         │  cmaj generate --target=clap --output=<gen>/  --clapIncludePath=<clap-headers>
#         ▼
#     <gen>/*.cpp + <gen>/CMakeLists.txt  (CLAP C++, self-contained DSP)
#         │
#         │  ${WASI_SDK}/bin/clang++ --target=wasm32-wasi -fno-exceptions -fno-rtti -Oz
#         │    -Wl,--export-dynamic -Wl,--export-table -Wl,--growable-table
#         │    -Wl,--export=clap_entry -Wl,--no-entry  -I${CLAP_INCLUDE}
#         ▼
#     module.wasm                          (drops into bundle-wclap.mjs as-is)
#
# Usage:
#     build-cmaj-wclap.sh <plugin-dir> <patch-file> <output-wasm>
#
# Env (override to point at your local toolchains):
#     CMAJ          — path to the cmaj binary             (default: cmaj on $PATH)
#     WASI_SDK      — path to wasi-sdk install            (default: /opt/wasi-sdk)
#     CLAP_INCLUDE  — path to free-audio/clap "include/"  (default: vendor/clap/include)
#
# Both toolchains are big & version-sensitive — we don't vendor them. CI is
# expected to install them at fixed versions; local devs grab them once.
# Missing tools cause a clean error with the env var to set, not a wall of
# C++ template diagnostics.

set -euo pipefail

PLUGIN_DIR="${1:?usage: build-cmaj-wclap.sh <plugin-dir> <patch-file> <output-wasm>}"
PATCH_FILE="${2:?missing <patch-file>}"
OUT_WASM="${3:?missing <output-wasm>}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

CMAJ="${CMAJ:-cmaj}"
WASI_SDK="${WASI_SDK:-/opt/wasi-sdk}"
CLAP_INCLUDE="${CLAP_INCLUDE:-${REPO_ROOT}/vendor/clap/include}"

# --- Tool / sysroot presence checks -----------------------------------------

if ! command -v "${CMAJ}" >/dev/null 2>&1; then
    cat >&2 <<EOF
build-cmaj-wclap: \`cmaj\` not found.

Install Cmajor from https://github.com/cmajor-lang/cmajor/releases (the
\`cmaj\` CLI ships in every release), or set CMAJ=/path/to/cmaj if you
have it elsewhere.
EOF
    exit 1
fi

if [[ ! -x "${WASI_SDK}/bin/clang++" ]]; then
    cat >&2 <<EOF
build-cmaj-wclap: WASI-SDK not found at \${WASI_SDK} = ${WASI_SDK}.

Grab a release from https://github.com/WebAssembly/wasi-sdk/releases and
either install it to /opt/wasi-sdk or export WASI_SDK pointing at the
unpacked directory (the one containing bin/clang++).
EOF
    exit 1
fi

if [[ ! -d "${CLAP_INCLUDE}/clap" ]]; then
    cat >&2 <<EOF
build-cmaj-wclap: CLAP headers not found at \${CLAP_INCLUDE} = ${CLAP_INCLUDE}.

Clone https://github.com/free-audio/clap into vendor/clap (or set
CLAP_INCLUDE to the existing "include" directory). The headers are small
enough to vendor; they're effectively the CLAP ABI itself.
EOF
    exit 1
fi

cd "${PLUGIN_DIR}"

GEN_DIR="generated/clap"
DIST_DIR="$(dirname "${OUT_WASM}")"

rm -rf "${GEN_DIR}"
mkdir -p "${GEN_DIR}" "${DIST_DIR}"

# --- Stage 1: cmaj → C++ -----------------------------------------------------
#
# The CLAP generator emits a self-contained C++ project (no JIT, no Cmajor
# runtime) with a clap_entry symbol baked in. We don't use its CMakeLists —
# we drive clang++ directly so the wasm-specific linker flags stay in one
# place. The generated *.cpp files are what we feed to clang.

echo "[cmaj] generating CLAP project from ${PATCH_FILE}"
"${CMAJ}" generate \
    --target=clap \
    --output="${GEN_DIR}" \
    --clapIncludePath="${CLAP_INCLUDE}" \
    "${PATCH_FILE}"

# --- Stage 2: wasi-sdk → module.wasm ----------------------------------------

SRCS=( "${GEN_DIR}"/*.cpp )
if [[ ${#SRCS[@]} -eq 0 || ! -f "${SRCS[0]}" ]]; then
    echo "build-cmaj-wclap: no .cpp files in ${GEN_DIR} — cmaj generate produced an unexpected layout" >&2
    exit 1
fi

CLANG="${WASI_SDK}/bin/clang++"

# Why these flags:
#
#   --target=wasm32-wasi    wasi-sdk's default; gives us a libc/libc++ that
#                           compiles, so cmaj's generated code (uses STL)
#                           links without dropping to wasm32-unknown-unknown.
#
#   -fno-exceptions -fno-rtti
#                           CLAP's ABI is C; the only C++ is internal to the
#                           DSP. Both flags shrink the binary materially and
#                           avoid pulling in unwinder code that doesn't make
#                           sense in a hosted CLAP context.
#
#   -Oz                     size optimisation. Cmajor-generated DSP is
#                           branch-light and inlines well; -Oz holds up.
#
#   -Wl,--no-entry          there's no `main` — wasi-sdk would complain.
#   -Wl,--export=clap_entry the symbol the WCLAP host walks to find the
#                           plugin factory. Without an explicit export the
#                           linker GCs it (it has no callers inside wasm).
#   -Wl,--export-table      JS-side `wclap-host-js` reads the function table
#   -Wl,--growable-table    out of the wasm exports and grows it at runtime
#                           to install host trampolines. Default lld setup
#                           has max == initial, which traps the grow().
#
# (`crates/wclap-plugin`'s build.rs sets the same two table flags for the
# Rust path — see plinken-org/CLAUDE.md "build.rs needs BOTH linker flags".)

echo "[wasi-sdk] compiling $(basename "${PWD}") to wasm32 CLAP"
"${CLANG}" \
    --target=wasm32-wasi \
    --sysroot="${WASI_SDK}/share/wasi-sysroot" \
    -fno-exceptions -fno-rtti \
    -fvisibility=hidden \
    -std=c++17 \
    -Oz \
    -I"${CLAP_INCLUDE}" \
    -I"${GEN_DIR}" \
    -Wl,--no-entry \
    -Wl,--export=clap_entry \
    -Wl,--export-table \
    -Wl,--growable-table \
    -o "${OUT_WASM}" \
    "${SRCS[@]}"

# Optional: pipe through wasm-opt if present. Big size win on cmaj output.
if command -v wasm-opt >/dev/null 2>&1; then
    echo "[wasm-opt] -Oz"
    wasm-opt -Oz --strip-debug -o "${OUT_WASM}.opt" "${OUT_WASM}"
    mv "${OUT_WASM}.opt" "${OUT_WASM}"
fi

size=$(stat -c%s "${OUT_WASM}" 2>/dev/null || stat -f%z "${OUT_WASM}")
echo "  ✓ ${OUT_WASM} (${size} B)"
