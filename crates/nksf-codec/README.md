# nksf-codec

Pure-Rust codec for the **NKSF** (Native Kontrol Standard) preset container —
the canonical artifact of the Plinken/Taluvi sound library. Parses and encodes
the RIFF (`NIKS`) container: `NISI` (summary metadata, MessagePack), `NICA`
(controller pages, reserved), `PLID` (plugin id), `PCHK` (opaque plugin state,
never interpreted here).

This crate is the single implementation behind the `plinken:nksf` **WIT world**
(`wit/nksf.wit`):

- **native** (`rlib`) — used by the runner (wasmtime host) and by `cargo test`;
- **wasm component** (`cargo component build --features component`) — transpiled
  to JS with **jco** for the browser; the TS `Listing`/facet types and these
  Rust structs both derive from the same WIT records.

The CLAP host/plugin ABI (`clap.state` etc.) is **not** part of this crate — it
stays C-ABI core wasm. This codec only wraps/unwraps the container around that
opaque state blob.

```sh
cargo test -p nksf-codec                                   # native, 7 tests
cargo component build --release --features component        # wasm component
```

Spec: `../../spec/wclap-preset.md`. Online library: `Plinken/docs/sound-library-nks.md`.

## Status

Implemented and tested (12 tests): RIFF container; `NISI` summary; `PLID`
including `CLAP.id`, `VST.magic`, `VST3.uid`; `NICA` `ni8` controller pages
(remote-controls); `PCHK` verbatim. MessagePack covers the container's full
subset — nil, bool, integers (fixint / int8-64 / uint8-64), string, array, map.

Follow-ups: real-file interop fixtures from shipping NKS libraries, and any
NISI fields that use floats (none in our schema today).
