#!/usr/bin/env bash
# build-cmaj-wclap.sh — turn a .cmajorpatch into a wasm32 CLAP plugin.
#
# Pipeline:
#
#     .cmajorpatch
#         │
#         │  cmaj generate --target=cpp --output=<gen>.h
#         ▼
#     generated/cpp/<name>.h          (self-contained C++ DSP class)
#         │
#         │  ${WASI_SDK}/bin/clang++ scripts/cmaj-wclap-shim.cpp + that header
#         │    -include <gen>.h -DCMAJ_CLASS_NAME=Piano -DWCLAP_PLUGIN_ID="…" …
#         ▼
#     dist/<name>.wclap.wasm          (drops into bundle-wclap.mjs as-is)
#
# Why `--target=cpp` rather than `--target=clap`:
#
#   `cmaj generate --target=clap` emits a full native CLAP plugin project
#   (CMakeLists, choc, QuickJS, WebKit2GTK / `cmaj_PatchWebView.h`, the
#   works) — none of which compiles for `wasm32-wasi`. The wasm host has
#   no DOM, no webview, and no JS engine for the patch worker. The `cpp`
#   target emits just the DSP class as a single self-contained header
#   (stdlib-only deps), which is exactly the surface we want to wrap.
#
#   `scripts/cmaj-wclap-shim.cpp` is the thin CLAP entry shim that
#   adapts that DSP class to the CLAP ABI. One translation unit covers
#   entry + factory + a single plugin per .wasm.
#
# Usage:
#     build-cmaj-wclap.sh <plugin-dir> <patch-file> <output-wasm>
#
#   <plugin-dir> must contain `plugin.json` (the WCLAP manifest the host
#   reads) and the .cmajorpatch / .cmajor source. The shim derives all
#   its descriptor strings (id, name, vendor, version, features) from
#   `plugin.json`, so there's no duplication between the patch and the
#   WCLAP metadata.
#
# Env (override to point at your local toolchains):
#     CMAJ          — path to the cmaj binary             (default: cmaj on $PATH)
#     WASI_SDK      — path to wasi-sdk install            (default: /opt/wasi-sdk)
#     CLAP_INCLUDE  — path to free-audio/clap "include/"  (default: vendor/clap/include)
#
# Both toolchains are big & version-sensitive — we don't vendor them. CI
# is expected to install them at fixed versions; local devs grab them
# once. Missing tools cause a clean error with the env var to set, not
# a wall of C++ template diagnostics.

set -euo pipefail

PLUGIN_DIR="${1:?usage: build-cmaj-wclap.sh <plugin-dir> <patch-file> <output-wasm>}"
PATCH_FILE="${2:?missing <patch-file>}"
OUT_WASM="${3:?missing <output-wasm>}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

CMAJ="${CMAJ:-cmaj}"
WASI_SDK="${WASI_SDK:-/opt/wasi-sdk}"
CLAP_INCLUDE="${CLAP_INCLUDE:-${REPO_ROOT}/vendor/clap/include}"
SHIM_CPP="${SCRIPT_DIR}/cmaj-wclap-shim.cpp"

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
CLAP_INCLUDE to the existing "include" directory). The headers are
small enough to vendor; they're effectively the CLAP ABI itself.
EOF
    exit 1
fi

cd "${PLUGIN_DIR}"

if [[ ! -f plugin.json ]]; then
    echo "build-cmaj-wclap: ${PLUGIN_DIR}/plugin.json missing — the shim needs it for the CLAP descriptor" >&2
    exit 1
fi

GEN_DIR="generated/cpp"
DIST_DIR="$(dirname "${OUT_WASM}")"
WASM_BASENAME="$(basename "${OUT_WASM}" .wasm)"
GEN_HEADER="${GEN_DIR}/${WASM_BASENAME}.h"

rm -rf "${GEN_DIR}"
mkdir -p "${GEN_DIR}" "${DIST_DIR}"

# --- Stage 1: cmaj → C++ header ---------------------------------------------
#
# `--target=cpp` emits a single .h file containing the whole DSP class
# (stdlib deps only). The class name matches the [[main]] processor /
# graph name in the .cmajor source.

echo "[cmaj] generating C++ DSP class from ${PATCH_FILE}"
"${CMAJ}" generate \
    --target=cpp \
    --output="${GEN_HEADER}" \
    "${PATCH_FILE}"

# Strip the `throw std::runtime_error(...)` lines from the generated
# header. They appear only in endpoint-handle-mismatch error paths the
# shim never hits (we always pass valid handles), and they pull in
# `__cxa_throw` + `__cxa_allocate_exception` which wasi-sdk's libc++
# doesn't link against. Replacing them with `__builtin_trap()` keeps
# the same semantics (program ends loudly if we ever do mismatch a
# handle) and lets us compile without -fexceptions + the unwinder.
perl -i -pe 's{throw\s+std::runtime_error\s*\([^;]*\)\s*;}{__builtin_trap();}g' "${GEN_HEADER}"

# Pull the top-level class name out of the generated header so the shim
# knows what type to instantiate. cmaj emits exactly one `struct <Name>`
# at the top level (matches the patch's mainProcessor / graph name);
# nested DSP classes are namespaced with underscores.
CMAJ_CLASS="$(awk '/^struct [A-Z][A-Za-z0-9_]*$/ { print $2; exit }' "${GEN_HEADER}")"
if [[ -z "${CMAJ_CLASS}" ]]; then
    echo "build-cmaj-wclap: could not extract top-level struct name from ${GEN_HEADER}" >&2
    exit 1
fi
echo "  class: ${CMAJ_CLASS}"

# --- Read plugin.json for the CLAP descriptor strings -----------------------
#
# We extract id / name / vendor / version / description / features
# straight from plugin.json (the same manifest the host's shelf reads)
# so descriptor strings don't get out of sync between the WCLAP
# metadata and the embedded CLAP plugin descriptor. node -p is the
# simplest portable JSON extractor we already depend on.

read_pj() {
    node -e "const p = require('./plugin.json'); process.stdout.write(String(p.$1 ?? ''));"
}

# Extract the (handle, init) pairs for every input value endpoint from
# the cmaj-generated header. cmaj embeds the `init:` annotations in the
# `programDetailsJSON` static string and matches each name to a
# numeric handle in `EndpointHandles` — but it does NOT apply those
# inits during `_initialise` (state is zero-cleared instead). The host
# is expected to push initial values via `setValue`. We do that in the
# shim, driven by this generated .inc.
PARAM_INC="$(pwd)/${GEN_DIR}/params.inc"
node --input-type=module -e "
import fs from 'node:fs';
const src = fs.readFileSync('${GEN_HEADER}', 'utf8');
// programDetailsJSON literal: concatenated C++ string literals, one
// JSON line each (\"…\\n\"). Strip the wrapping to recover the JSON.
const m = src.match(/programDetailsJSON\s*=\s*([\s\S]*?);\s*\n/);
if (!m) { console.error('build-cmaj-wclap: programDetailsJSON not found'); process.exit(1); }
const json = JSON.parse(
    m[1]
      .split(/\n/)
      .map(l => l.trim())
      .filter(l => l.startsWith('\"'))
      .map(l => l.slice(1, l.lastIndexOf('\"')))
      .join('')
      .replace(/\\\\n/g, '')
      .replace(/\\\\\"/g, '\"')
);
// Match each input endpointID to its numeric handle in EndpointHandles.
const handles = new Map();
const enumBlock = src.match(/enum class EndpointHandles[\s\S]*?\};/);
if (enumBlock) {
    for (const line of enumBlock[0].split(/\n/)) {
        const mm = line.match(/^\s*(\w+)\s*=\s*(\d+)/);
        if (mm) handles.set(mm[1], Number(mm[2]));
    }
}
const escape = s => String(s).replace(/\\\\/g, '\\\\\\\\').replace(/\"/g, '\\\\\"');
const f = n => Number(n).toFixed(6) + 'f';
const lines = [];
let count = 0;
for (const ep of (json.inputs || [])) {
    if (ep.endpointType !== 'value') continue;
    const h = handles.get(ep.endpointID);
    if (h === undefined) continue;
    const ann = ep.annotation || {};
    const name = ann.name || ep.endpointID;
    const minV = typeof ann.min === 'number' ? ann.min : 0;
    const maxV = typeof ann.max === 'number' ? ann.max : 1;
    const defV = typeof ann.init === 'number' ? ann.init : minV;
    const step = typeof ann.step === 'number' ? ann.step : 0;
    lines.push(\`    { \${h}u, \"\${escape(name)}\", \${f(minV)}, \${f(maxV)}, \${f(defV)}, \${f(step)} },\`);
    count++;
}
fs.writeFileSync('${PARAM_INC}', lines.join('\n') + (lines.length ? '\n' : ''));
console.log(\`  params: \${count} parameter(s) → \${'${PARAM_INC}'}\`);
"

PLUGIN_ID="$(read_pj id)"
PLUGIN_NAME="$(read_pj name)"
PLUGIN_VENDOR="$(read_pj vendor)"
PLUGIN_VERSION="$(read_pj version)"
PLUGIN_DESC="$(read_pj description)"
PLUGIN_IS_INSTRUMENT="$(node -e "const p = require('./plugin.json'); const f = p.features || []; process.stdout.write(f.includes('instrument') ? '1' : '0');")"
PLUGIN_HAS_UI="$(node -e "const p = require('./plugin.json'); process.stdout.write(p.has_ui === true ? '1' : '0');")"

for var in PLUGIN_ID PLUGIN_NAME PLUGIN_VENDOR PLUGIN_VERSION; do
    if [[ -z "${!var}" ]]; then
        echo "build-cmaj-wclap: plugin.json missing field for ${var}" >&2
        exit 1
    fi
done

# Escape any embedded double quotes so the values land safely as C string
# literals via -D. The description is the only field likely to contain
# anything tricky; we still run the others through the same filter.
esc() { printf '%s' "$1" | sed 's/"/\\"/g'; }

PLUGIN_ID_ESC="$(esc "${PLUGIN_ID}")"
PLUGIN_NAME_ESC="$(esc "${PLUGIN_NAME}")"
PLUGIN_VENDOR_ESC="$(esc "${PLUGIN_VENDOR}")"
PLUGIN_VERSION_ESC="$(esc "${PLUGIN_VERSION}")"
PLUGIN_DESC_ESC="$(esc "${PLUGIN_DESC}")"

# --- Stage 2: clang++ → wasm32 CLAP -----------------------------------------
#
# Flags worth knowing:
#
#   --target=wasm32-wasi    wasi-sdk's default; gives us a libc/libc++
#                           that compiles. cmaj-generated code uses STL
#                           (std::array, <cmath>, std::memcpy), so
#                           dropping to wasm32-unknown-unknown is a
#                           non-starter.
#
#   -fno-rtti -fno-exceptions
#                           CLAP's ABI is C; the C++ is plugin-internal
#                           only. The shim never throws, and we sed the
#                           `throw std::runtime_error(...)` lines out of
#                           the generated DSP header (replacing them
#                           with __builtin_trap), so neither flag costs
#                           us anything. Both shrink the binary
#                           noticeably and keep `__cxa_throw` /
#                           `__cxa_allocate_exception` (not provided by
#                           wasi-sdk's libc++) out of the link.
#
#   -Oz                     size-optimised. cmaj output is branch-light
#                           and inlines well; -Oz holds up.
#
#   -Wl,--no-entry          there's no `main`; wasi-sdk would complain.
#   -Wl,--export=clap_entry the symbol wclap-host-js walks to find the
#                           plugin factory. Without an explicit export
#                           the linker GCs it (no callers inside wasm).
#   -Wl,--export-table      the host reads the wasm function table from
#   -Wl,--growable-table    plugin exports and grows it at runtime to
#                           install trampolines. Default lld setup has
#                           max == initial → first Table.grow() traps.
#                           Same flags `crates/wclap-plugin`'s build.rs
#                           sets for the Rust path.

echo "[wasi-sdk] compiling ${CMAJ_CLASS} to wasm32 CLAP"
"${WASI_SDK}/bin/clang++" \
    --target=wasm32-wasi \
    --sysroot="${WASI_SDK}/share/wasi-sysroot" \
    -fno-rtti \
    -fno-exceptions \
    -fvisibility=hidden \
    -std=c++17 \
    -Oz \
    -I"${CLAP_INCLUDE}" \
    -include "${GEN_HEADER}" \
    -DCMAJ_CLASS_NAME="${CMAJ_CLASS}" \
    -DCMAJ_HEADER_PATH="\"${GEN_HEADER}\"" \
    -DCMAJ_PARAMS_INC="\"${PARAM_INC}\"" \
    -DWCLAP_PLUGIN_ID="\"${PLUGIN_ID_ESC}\"" \
    -DWCLAP_PLUGIN_NAME="\"${PLUGIN_NAME_ESC}\"" \
    -DWCLAP_PLUGIN_VENDOR="\"${PLUGIN_VENDOR_ESC}\"" \
    -DWCLAP_PLUGIN_VERSION="\"${PLUGIN_VERSION_ESC}\"" \
    -DWCLAP_PLUGIN_DESC="\"${PLUGIN_DESC_ESC}\"" \
    -DWCLAP_IS_INSTRUMENT="${PLUGIN_IS_INSTRUMENT}" \
    -DWCLAP_HAS_UI="${PLUGIN_HAS_UI}" \
    -Wl,--no-entry \
    -Wl,--export=clap_entry \
    -Wl,--export=malloc \
    -Wl,--export=free \
    -Wl,--export-table \
    -Wl,--growable-table \
    -o "${OUT_WASM}" \
    "${SHIM_CPP}"

# Optional: wasm-opt pass for size, if available. Cmaj-generated DSP
# benefits a lot from a final -Oz pass after lld.
if command -v wasm-opt >/dev/null 2>&1; then
    echo "[wasm-opt] -Oz"
    wasm-opt -Oz --strip-debug -o "${OUT_WASM}.opt" "${OUT_WASM}"
    mv "${OUT_WASM}.opt" "${OUT_WASM}"
fi

size=$(stat -c%s "${OUT_WASM}" 2>/dev/null || stat -f%z "${OUT_WASM}")
echo "  ✓ ${OUT_WASM} (${size} B)"
