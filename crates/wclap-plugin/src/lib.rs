//! Shared CLAP/WCLAP plugin scaffold.
//!
//! Plinken's Rust WCLAP plugins (synome, vocal-limiter, vocal-compressor,
//! vocal-eq) all need the same ~500 lines of hand-rolled CLAP ABI glue:
//! exported `clap_entry` wasm global, factory with one descriptor, plugin
//! vtable, audio-ports / note-ports extensions, dlmalloc + wasm panic
//! handler, and a `malloc` shim the JS host calls into.
//!
//! This crate owns all of that. A plugin crate provides:
//!
//! 1. A `static PLUGIN_DEF: PluginDef` describing identity + port shape.
//! 2. A type implementing [`Plugin`] with its DSP state and lifecycle.
//! 3. A one-line `_initialize` that calls [`init_plugin`].
//!
//! That's it — see `plugins/com.plinken/synome/src/lib.rs` for the
//! reference shape.

#![no_std]
// Single-threaded wasm: writing static mut once in `_initialize` and reading
// after is sound in this context.
#![allow(non_upper_case_globals, static_mut_refs)]

extern crate alloc;

use alloc::boxed::Box;
use core::ptr::{addr_of, addr_of_mut};

// ---------------------------------------------------------------------------
// Allocator + panic handler — provided by default for no_std cdylibs (like
// synome). Plugins that depend on std (e.g. for fundsp) must disable the
// `runtime` feature so std's own panic + allocator are used instead.
// ---------------------------------------------------------------------------

#[cfg(all(target_arch = "wasm32", feature = "runtime"))]
#[global_allocator]
static ALLOC: dlmalloc::GlobalDlmalloc = dlmalloc::GlobalDlmalloc;

#[cfg(all(target_arch = "wasm32", feature = "runtime"))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

// ---------------------------------------------------------------------------
// `malloc` — the JS host's `_wclapInstance.malloc32` calls into this to
// allocate inside our linear memory (host-side scratch buffers for port
// info, plugin struct, etc.).
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn malloc(size: u32) -> u32 {
    use alloc::alloc::{alloc, Layout};
    if size == 0 {
        return 0;
    }
    // SAFETY: 8-byte alignment fits every CLAP struct we'll be asked to
    // allocate. Size comes from the host and is bounded by CLAP's own
    // struct sizes.
    unsafe {
        let layout = Layout::from_size_align_unchecked(size as usize, 8);
        alloc(layout) as u32
    }
}

// ---------------------------------------------------------------------------
// CLAP struct offsets — exact match to the C `clap_*` structs as laid out
// on wasm32 (4-byte pointers, natural alignment). Kept private; consumers
// go through the safe wrappers below.
// ---------------------------------------------------------------------------

mod offsets {
    pub mod plugin {
        pub const DESC: usize = 0;
        pub const PLUGIN_DATA: usize = 4;
        pub const INIT: usize = 8;
        pub const DESTROY: usize = 12;
        pub const ACTIVATE: usize = 16;
        pub const DEACTIVATE: usize = 20;
        pub const START_PROCESSING: usize = 24;
        pub const STOP_PROCESSING: usize = 28;
        pub const RESET: usize = 32;
        pub const PROCESS: usize = 36;
        pub const GET_EXTENSION: usize = 40;
        pub const ON_MAIN_THREAD: usize = 44;
        pub const SIZE: usize = 48;
    }
    pub mod process_ {
        pub const FRAMES_COUNT: usize = 8;
        pub const AUDIO_INPUTS: usize = 16;
        pub const AUDIO_OUTPUTS: usize = 20;
        pub const AUDIO_INPUTS_COUNT: usize = 24;
        pub const AUDIO_OUTPUTS_COUNT: usize = 28;
    }
    pub mod audio_buffer {
        pub const DATA32: usize = 0;
        pub const CHANNEL_COUNT: usize = 8;
        pub const SIZE: usize = 24;
    }
    pub mod audio_port_info {
        pub const ID: usize = 0;
        pub const NAME_OFFSET: usize = 4;
        pub const FLAGS: usize = 260;
        pub const CHANNEL_COUNT: usize = 264;
        pub const SIZE: usize = 276;
    }
    pub mod note_port_info {
        pub const ID: usize = 0;
        pub const SUPPORTED_DIALECTS: usize = 4;
        pub const PREFERRED_DIALECT: usize = 8;
        pub const NAME_OFFSET: usize = 12;
        pub const SIZE: usize = 268;
    }
    pub mod webview {
        // clap_plugin_webview (draft v3) — three function pointers.
        pub const GET_URI: usize = 0;       // i32(plugin*, char* buf, u32 cap)
        pub const GET_RESOURCE: usize = 4;  // bool(plugin*, char* path, char* mime_buf, u32 mime_cap, ostream*)
        pub const RECEIVE: usize = 8;       // bool(plugin*, void* buf, u32 size)
        pub const SIZE: usize = 12;
    }
    pub mod params {
        // clap_plugin_params — six function pointers. Layout matches what
        // crates/wclap-host expects (param_info::SIZE = 1320 bytes).
        pub const COUNT: usize = 0;          // u32(plugin*)
        pub const GET_INFO: usize = 4;       // bool(plugin*, u32 idx, param_info*)
        pub const GET_VALUE: usize = 8;      // bool(plugin*, u32 id, f64* out)
        pub const VALUE_TO_TEXT: usize = 12; // bool(plugin*, u32 id, f64 v, char* buf, u32 cap)
        pub const TEXT_TO_VALUE: usize = 16; // bool(plugin*, u32 id, char* text, f64* out)
        pub const FLUSH: usize = 20;         // void(plugin*, in_events*, out_events*)
        pub const SIZE: usize = 24;
    }
    pub mod param_info {
        // 1320-byte struct host allocates; we write into it via offsets.
        pub const ID: usize = 0;
        pub const FLAGS: usize = 4;
        pub const COOKIE: usize = 8;
        pub const NAME: usize = 12;
        pub const NAME_CAP: usize = 256;
        pub const MODULE: usize = 268;
        pub const MODULE_CAP: usize = 1024;
        pub const MIN_VALUE: usize = 1296;
        pub const MAX_VALUE: usize = 1304;
        pub const DEFAULT_VALUE: usize = 1312;
        pub const SIZE: usize = 1320;
    }
    pub mod latency {
        // clap_plugin_latency — single function pointer.
        pub const GET: usize = 0; // u32(plugin*)
        pub const SIZE: usize = 4;
    }
    pub mod host {
        // clap_host — what the host passes to factory.create_plugin.
        // Offsets pinned to wclap-host's `clap.rs` so cross-module casts
        // hit the right fields.
        pub const GET_EXTENSION: usize = 32; // (host*, char*) -> ptr
    }
    pub mod host_webview {
        // clap_host_webview — single function pointer the plugin calls to
        // push bytes back to the iframe.
        pub const SEND: usize = 0; // (host*, buf, size) -> bool
        pub const SIZE: usize = 4;
    }
}

const FACTORY_ID: &[u8] = b"clap.plugin-factory\0";
const EXT_AUDIO_PORTS: &[u8] = b"clap.audio-ports\0";
const EXT_NOTE_PORTS: &[u8] = b"clap.note-ports\0";
const EXT_WEBVIEW: &[u8] = b"clap.webview/3\0";
const EXT_PARAMS: &[u8] = b"clap.params\0";
const EXT_LATENCY: &[u8] = b"clap.latency\0";
const EXT_STATE: &[u8] = b"clap.state\0";

/// CLAP param-info flag bits (subset).
pub const PARAM_IS_STEPPED: u32 = 1 << 0;
pub const PARAM_IS_PERIODIC: u32 = 1 << 1;
pub const PARAM_IS_HIDDEN: u32 = 1 << 2;
pub const PARAM_IS_READONLY: u32 = 1 << 3;
pub const PARAM_IS_BYPASS: u32 = 1 << 4;
pub const PARAM_IS_AUTOMATABLE: u32 = 1 << 5;

const NOTE_DIALECT_CLAP: u32 = 1 << 0;
const NOTE_DIALECT_MIDI: u32 = 1 << 1;

const PORT_FLAG_IS_MAIN: u32 = 1;

// ---------------------------------------------------------------------------
// Public surface.
// ---------------------------------------------------------------------------

/// CLAP process status return value.
#[repr(u32)]
#[derive(Copy, Clone)]
pub enum ProcessStatus {
    Continue = 1,
    ContinueIfNotQuiet = 2,
    Sleep = 3,
    Tail = 4,
}

/// Plugin identity + port shape. All byte slices must be NUL-terminated so
/// they can be exposed as `const char *` to the host.
pub struct PluginDef {
    pub id: &'static [u8],
    pub name: &'static [u8],
    pub vendor: &'static [u8],
    pub url: &'static [u8],
    pub version: &'static [u8],
    pub description: &'static [u8],
    /// Each feature is its own NUL-terminated tag (e.g. `b"audio-effect\0"`).
    pub features: &'static [&'static [u8]],
    /// Number of stereo audio input ports (0 or 1).
    pub audio_inputs: u8,
    /// Number of stereo audio output ports (0 or 1).
    pub audio_outputs: u8,
    /// Number of note input ports (0 or 1).
    pub note_inputs: u8,
    /// Path to the UI entrypoint inside the plugin's tarball, with leading
    /// slash and NUL terminator — e.g. `b"/ui/index.html\0"`. Combined at
    /// runtime with the host-supplied `modulePath` to form the full
    /// `file:` URI (`file:/plugin/<hash>/ui/index.html`). `None` means the
    /// plugin has no UI and the host won't load an iframe.
    pub ui_path: Option<&'static [u8]>,
}

/// Static descriptor for one automatable parameter.
pub struct ParamDef {
    /// CLAP param ID — must be unique within the plugin and stable across
    /// versions (used by hosts to look up saved automation).
    pub id: u32,
    /// Bitflag mask (combine `PARAM_IS_AUTOMATABLE`, `PARAM_IS_STEPPED`, …).
    pub flags: u32,
    /// Display name, NUL-terminated. Max 255 bytes useful (host buffer = 256).
    pub name: &'static [u8],
    /// Module path for parameter grouping in the host's automation lane
    /// browser. Typically `b"\0"` for a flat layout.
    pub module: &'static [u8],
    pub min: f64,
    pub max: f64,
    pub default: f64,
}

/// The plugin's DSP state. One instance per host-side voice/track.
pub trait Plugin: Sized + 'static {
    fn new() -> Self;
    fn activate(&mut self, _sample_rate: f64, _max_frames: u32) {}
    fn deactivate(&mut self) {}
    fn start_processing(&mut self) {}
    fn stop_processing(&mut self) {}
    fn reset(&mut self) {}
    fn process(&mut self, ctx: &mut ProcessCtx) -> ProcessStatus;

    /// Static list of parameters this plugin exposes. Default empty.
    /// Hosts call this once at load to enumerate automation lanes.
    fn params() -> &'static [ParamDef] {
        &[]
    }

    /// Read the current value of param `id`. Plugins that don't override
    /// `params()` can leave this default.
    fn get_param(&self, _id: u32) -> f64 {
        0.0
    }

    /// Apply a UI- or automation-driven param change. Called from inside
    /// the audio process block (param events) and from `webview.receive`
    /// (UI-driven `{set:[id,value]}` messages). The plugin should clamp
    /// to the param's declared range itself if it cares.
    fn set_param(&mut self, _id: u32, _value: f64) {}

    /// Reported via `clap.latency.get` — the number of samples the plugin
    /// delays its output relative to its input. 0 for feedback / feed-
    /// forward designs; N samples for lookahead designs. Hosts use this to
    /// add compensating delay on parallel chains and to schedule with the
    /// correct slack budget. Default 0.
    fn latency_samples(&self) -> u32 {
        0
    }
}

/// Safe view onto one block of audio passed by the host.
pub struct ProcessCtx {
    process_ptr: u32,
}

impl ProcessCtx {
    /// Number of frames in this block.
    pub fn frames(&self) -> usize {
        unsafe { read_u32(self.process_ptr + offsets::process_::FRAMES_COUNT as u32) as usize }
    }

    /// Borrow channel `ch` of input port `port`. Returns `None` if the port
    /// or channel is absent (e.g. instrument plugins with no inputs).
    pub fn input(&self, port: usize, ch: usize) -> Option<&[f32]> {
        unsafe { channel_slice(self.process_ptr, /*input=*/ true, port, ch, false) }
            .map(|(p, n)| unsafe { core::slice::from_raw_parts(p, n) })
    }

    /// Borrow channel `ch` of output port `port` mutably. Returns `None`
    /// when the port or channel is absent.
    pub fn output_mut(&mut self, port: usize, ch: usize) -> Option<&mut [f32]> {
        unsafe { channel_slice(self.process_ptr, /*input=*/ false, port, ch, false) }
            .map(|(p, n)| unsafe { core::slice::from_raw_parts_mut(p, n) })
    }

    /// Borrow main stereo input + output as four independent slices in one
    /// call — the common shape for stereo audio effects. Returns `None` if
    /// either port lacks two channels.
    ///
    /// The four slices alias-safely because CLAP guarantees input and
    /// output buffers are distinct allocations whenever `in_place_pair` is
    /// unset (which we never set).
    pub fn stereo_io(&mut self) -> Option<StereoIo<'_>> {
        unsafe {
            let (il, n) = channel_slice(self.process_ptr, true, 0, 0, false)?;
            let (ir, _) = channel_slice(self.process_ptr, true, 0, 1, false)?;
            let (ol, _) = channel_slice(self.process_ptr, false, 0, 0, true)?;
            let (or_, _) = channel_slice(self.process_ptr, false, 0, 1, true)?;
            Some(StereoIo {
                input_l: core::slice::from_raw_parts(il, n),
                input_r: core::slice::from_raw_parts(ir, n),
                output_l: core::slice::from_raw_parts_mut(ol, n),
                output_r: core::slice::from_raw_parts_mut(or_, n),
            })
        }
    }

    /// Borrow the main mono input + output (single channel each) as two
    /// independent slices. Returns `None` if either port doesn't expose
    /// at least one channel. Effects that want to handle both shapes try
    /// `stereo_io()` first, then fall back to this.
    pub fn mono_io(&mut self) -> Option<MonoIo<'_>> {
        unsafe {
            let (i, n) = channel_slice(self.process_ptr, true, 0, 0, false)?;
            let (o, _) = channel_slice(self.process_ptr, false, 0, 0, true)?;
            Some(MonoIo {
                input: core::slice::from_raw_parts(i, n),
                output: core::slice::from_raw_parts_mut(o, n),
            })
        }
    }

    /// Channel count of main input port 0 (`0` if no input port exists).
    pub fn input_channel_count(&self) -> u32 {
        unsafe { channel_count(self.process_ptr, true, 0) }
    }

    /// Channel count of main output port 0.
    pub fn output_channel_count(&self) -> u32 {
        unsafe { channel_count(self.process_ptr, false, 0) }
    }

    /// Push bytes to the plugin's UI iframe via `clap_host_webview.send`.
    /// No-op if the host didn't expose `clap.webview/3` or the plugin
    /// hasn't initialised (e.g. process called before init somehow).
    ///
    /// Typical use: encode a `{params:{<id>:<f64>, …}}` CBOR snapshot
    /// with current peak / GR / readonly param values and push at ~30 Hz.
    pub fn send_to_ui(&self, bytes: &[u8]) {
        unsafe {
            let host = DEF.host_ptr;
            let send_idx = DEF.host_webview_send;
            if host == 0 || send_idx == 0 || bytes.is_empty() {
                return;
            }
            type Send = extern "C" fn(host: u32, buf: u32, size: u32) -> u32;
            let f: Send = core::mem::transmute(send_idx as usize);
            f(host, bytes.as_ptr() as u32, bytes.len() as u32);
        }
    }
}

/// Four-slice view of one stereo block.
pub struct StereoIo<'a> {
    pub input_l: &'a [f32],
    pub input_r: &'a [f32],
    pub output_l: &'a mut [f32],
    pub output_r: &'a mut [f32],
}

/// Two-slice view of one mono block.
pub struct MonoIo<'a> {
    pub input: &'a [f32],
    pub output: &'a mut [f32],
}

/// Zero every output channel of the current block. Convenience for "I'm
/// not producing audio this block" — without this the host buffers carry
/// uninitialised or stale data.
pub fn silence(ctx: &mut ProcessCtx) {
    let frames = ctx.frames();
    let mut port = 0;
    while let Some(buf) = ctx.output_mut(port, 0) {
        // Got at least one channel — clear it and walk the rest.
        let n = buf.len().min(frames);
        buf[..n].fill(0.0);
        let mut ch = 1;
        while let Some(buf) = ctx.output_mut(port, ch) {
            let n = buf.len().min(frames);
            buf[..n].fill(0.0);
            ch += 1;
        }
        port += 1;
    }
}

// ---------------------------------------------------------------------------
// Static module-level state. `init_plugin<P>` populates these before
// returning from `_initialize`. Single-threaded wasm, single init call.
// ---------------------------------------------------------------------------

// Public so it can name the type of the exported `clap_entry` static.
// Host code never constructs one — it just reads fields by offset.
#[repr(C)]
pub struct ClapEntry {
    version_major: u32,
    version_minor: u32,
    version_revision: u32,
    init: u32,
    deinit: u32,
    get_factory: u32,
}

#[repr(C)]
struct ClapFactory {
    get_plugin_count: u32,
    get_plugin_descriptor: u32,
    create_plugin: u32,
}

#[repr(C)]
struct ClapDescriptor {
    version_major: u32,
    version_minor: u32,
    version_revision: u32,
    id: u32,
    name: u32,
    vendor: u32,
    url: u32,
    manual_url: u32,
    support_url: u32,
    version: u32,
    description: u32,
    features: u32,
}

#[repr(C)]
struct PortsExt {
    count: u32,
    get: u32,
}

#[repr(C)]
struct WebviewExt {
    get_uri: u32,
    get_resource: u32,
    receive: u32,
}

#[repr(C)]
struct ParamsExt {
    count: u32,
    get_info: u32,
    get_value: u32,
    value_to_text: u32,
    text_to_value: u32,
    flush: u32,
}

#[repr(C)]
struct LatencyExt {
    get: u32,
}

// The wasm global `clap_entry` exports the *address* of whatever static
// is named that way. By naming the actual ClapEntry struct `clap_entry`,
// the global directly points at the struct — no indirection slot, which
// is what the host expects (compare clack-plugin-gain).
#[no_mangle]
#[allow(non_upper_case_globals)]
pub static mut clap_entry: ClapEntry = ClapEntry {
    version_major: 1,
    version_minor: 2,
    version_revision: 2,
    init: 0,
    deinit: 0,
    get_factory: 0,
};

static mut FACTORY: ClapFactory = ClapFactory {
    get_plugin_count: 0,
    get_plugin_descriptor: 0,
    create_plugin: 0,
};

static mut DESCRIPTOR: ClapDescriptor = ClapDescriptor {
    version_major: 1,
    version_minor: 2,
    version_revision: 2,
    id: 0,
    name: 0,
    vendor: 0,
    url: 0,
    manual_url: 0,
    support_url: 0,
    version: 0,
    description: 0,
    features: 0,
};

static mut AUDIO_PORTS_EXT: PortsExt = PortsExt { count: 0, get: 0 };
static mut NOTE_PORTS_EXT: PortsExt = PortsExt { count: 0, get: 0 };
static mut WEBVIEW_EXT: WebviewExt = WebviewExt {
    get_uri: 0,
    get_resource: 0,
    receive: 0,
};
static mut PARAMS_EXT: ParamsExt = ParamsExt {
    count: 0,
    get_info: 0,
    get_value: 0,
    value_to_text: 0,
    text_to_value: 0,
    flush: 0,
};
static mut LATENCY_EXT: LatencyExt = LatencyExt { get: 0 };

/// `wclap_plugin_state` — two fn ptrs, save + load (clap.rs `state` module
/// in the host mirrors this layout).
#[repr(C)]
struct StateExt {
    save: u32,
    load: u32,
}
static mut STATE_EXT: StateExt = StateExt { save: 0, load: 0 };

// Room for up to 8 features. More than enough for any plugin we'll ship.
const MAX_FEATURES: usize = 8;
static mut FEATURES_TABLE: [u32; MAX_FEATURES + 1] = [0; MAX_FEATURES + 1];

/// Cached snapshot of the registered PluginDef. We hold raw pointers so the
/// C-ABI shims can read it without going through a Rust reference (avoiding
/// aliasing issues with `static mut`).
struct DefSnapshot {
    audio_inputs: u8,
    audio_outputs: u8,
    note_inputs: u8,
    id_ptr: *const u8,
    /// UI path bytes (NUL-terminated, leading slash) or null if no UI.
    ui_path_ptr: *const u8,
    /// Path length in bytes, EXCLUDING the trailing NUL.
    ui_path_len: u32,
    /// Pointer to the host-supplied `modulePath` C-string, captured during
    /// `entry.init`. Lives in plugin memory (host allocated it via our
    /// `malloc` and copied the bytes; it remains valid for our lifetime).
    /// Zero until `entry.init` runs.
    module_path_ptr: u32,
    /// `Plugin::params()` slice base + length. Cached at `init_plugin<P>`
    /// time so the C-ABI shims don't need to monomorphise on P themselves.
    params_ptr: *const ParamDef,
    params_len: u32,
    /// Per-module host pointer captured at `factory.create_plugin`. Used
    /// to call `host.get_extension` and (via `host_webview.send`) push
    /// bytes back to the iframe. WCLAP gives each plugin its own wasm
    /// module, so a static-per-module field is per-instance in practice.
    host_ptr: u32,
    /// Function-table index of `host_webview.send`. Resolved in
    /// `plugin.init` by calling `host.get_extension("clap.webview/3")`;
    /// 0 if the host doesn't expose the extension.
    host_webview_send: u32,
}
static mut DEF: DefSnapshot = DefSnapshot {
    audio_inputs: 0,
    audio_outputs: 0,
    note_inputs: 0,
    id_ptr: core::ptr::null(),
    ui_path_ptr: core::ptr::null(),
    ui_path_len: 0,
    module_path_ptr: 0,
    params_ptr: core::ptr::null(),
    params_len: 0,
    host_ptr: 0,
    host_webview_send: 0,
};

/// Per-plugin-type dispatch table. Populated by `init_plugin<P>` with
/// monomorphised thunks that own the `P` type.
struct VTable {
    new: Option<fn() -> *mut ()>,
    drop: Option<fn(*mut ())>,
    activate: Option<fn(*mut (), f64, u32)>,
    deactivate: Option<fn(*mut ())>,
    start_processing: Option<fn(*mut ())>,
    stop_processing: Option<fn(*mut ())>,
    reset: Option<fn(*mut ())>,
    process: Option<fn(*mut (), &mut ProcessCtx) -> ProcessStatus>,
    get_param: Option<fn(*mut (), u32) -> f64>,
    set_param: Option<fn(*mut (), u32, f64)>,
    latency_samples: Option<fn(*mut ()) -> u32>,
}

static mut VTABLE: VTable = VTable {
    new: None,
    drop: None,
    activate: None,
    deactivate: None,
    start_processing: None,
    stop_processing: None,
    reset: None,
    process: None,
    get_param: None,
    set_param: None,
    latency_samples: None,
};

// `clap_entry` itself is declared above as the actual ClapEntry struct;
// the wasm linker exports the global pointing directly at it. No wrapper
// indirection is needed.

// ---------------------------------------------------------------------------
// `init_plugin<P>` — called from the plugin's `_initialize`. Wires the
// PluginDef strings into the descriptor and installs P-typed thunks in the
// dispatch table.
// ---------------------------------------------------------------------------

pub fn init_plugin<P: Plugin>(def: &'static PluginDef) {
    assert!(
        def.features.len() <= MAX_FEATURES,
        "wclap-plugin: too many feature tags",
    );
    unsafe {
        clap_entry.init = entry_init as usize as u32;
        clap_entry.get_factory = entry_get_factory as usize as u32;

        FACTORY.get_plugin_count = factory_get_plugin_count as usize as u32;
        FACTORY.get_plugin_descriptor = factory_get_plugin_descriptor as usize as u32;
        FACTORY.create_plugin = factory_create_plugin as usize as u32;

        DESCRIPTOR.id = def.id.as_ptr() as u32;
        DESCRIPTOR.name = def.name.as_ptr() as u32;
        DESCRIPTOR.vendor = def.vendor.as_ptr() as u32;
        DESCRIPTOR.url = def.url.as_ptr() as u32;
        DESCRIPTOR.version = def.version.as_ptr() as u32;
        DESCRIPTOR.description = def.description.as_ptr() as u32;

        for (i, tag) in def.features.iter().enumerate() {
            FEATURES_TABLE[i] = tag.as_ptr() as u32;
        }
        // NULL terminator (slot at index = features.len()).
        FEATURES_TABLE[def.features.len()] = 0;
        DESCRIPTOR.features = FEATURES_TABLE.as_ptr() as u32;

        AUDIO_PORTS_EXT.count = audio_ports_count as usize as u32;
        AUDIO_PORTS_EXT.get = audio_ports_get as usize as u32;
        NOTE_PORTS_EXT.count = note_ports_count as usize as u32;
        NOTE_PORTS_EXT.get = note_ports_get as usize as u32;

        DEF.audio_inputs = def.audio_inputs;
        DEF.audio_outputs = def.audio_outputs;
        DEF.note_inputs = def.note_inputs;
        DEF.id_ptr = def.id.as_ptr();
        if let Some(path) = def.ui_path {
            // `len - 1` strips the trailing NUL we required in PluginDef.
            DEF.ui_path_ptr = path.as_ptr();
            DEF.ui_path_len = (path.len() as u32).saturating_sub(1);
            WEBVIEW_EXT.get_uri = webview_get_uri as usize as u32;
            WEBVIEW_EXT.get_resource = webview_get_resource as usize as u32;
            WEBVIEW_EXT.receive = webview_receive as usize as u32;
        }

        VTABLE.new = Some(thunk_new::<P>);
        VTABLE.drop = Some(thunk_drop::<P>);
        VTABLE.activate = Some(thunk_activate::<P>);
        VTABLE.deactivate = Some(thunk_deactivate::<P>);
        VTABLE.start_processing = Some(thunk_start::<P>);
        VTABLE.stop_processing = Some(thunk_stop::<P>);
        VTABLE.reset = Some(thunk_reset::<P>);
        VTABLE.process = Some(thunk_process::<P>);
        VTABLE.get_param = Some(thunk_get_param::<P>);
        VTABLE.set_param = Some(thunk_set_param::<P>);
        VTABLE.latency_samples = Some(thunk_latency_samples::<P>);

        // clap.latency is unconditionally exposed; plugins that don't
        // override `latency_samples()` simply report 0.
        LATENCY_EXT.get = latency_get as usize as u32;

        // clap.state — generic param-dump persistence (save = every declared
        // param's (id, value); load = replay via set_param). Exposed whenever
        // the plugin has params; plugins with richer internal state can get a
        // bespoke path later without breaking this blob (it's versioned).
        STATE_EXT.save = state_save as usize as u32;
        STATE_EXT.load = state_load as usize as u32;

        let params_slice = P::params();
        DEF.params_ptr = params_slice.as_ptr();
        DEF.params_len = params_slice.len() as u32;
        if !params_slice.is_empty() {
            PARAMS_EXT.count = params_count as usize as u32;
            PARAMS_EXT.get_info = params_get_info as usize as u32;
            PARAMS_EXT.get_value = params_get_value as usize as u32;
            PARAMS_EXT.value_to_text = params_value_to_text as usize as u32;
            PARAMS_EXT.text_to_value = params_text_to_value as usize as u32;
            PARAMS_EXT.flush = params_flush as usize as u32;
        }
    }
}

// ---------------------------------------------------------------------------
// Monomorphised thunks — one set per concrete Plugin type. They cast the
// opaque `*mut ()` from the plugin_data slot back to `*mut P`.
// ---------------------------------------------------------------------------

fn thunk_new<P: Plugin>() -> *mut () {
    Box::into_raw(Box::new(P::new())) as *mut ()
}

fn thunk_drop<P: Plugin>(p: *mut ()) {
    if !p.is_null() {
        unsafe {
            drop(Box::from_raw(p as *mut P));
        }
    }
}

fn thunk_activate<P: Plugin>(p: *mut (), sample_rate: f64, max_frames: u32) {
    unsafe { (*(p as *mut P)).activate(sample_rate, max_frames) }
}

fn thunk_deactivate<P: Plugin>(p: *mut ()) {
    unsafe { (*(p as *mut P)).deactivate() }
}

fn thunk_start<P: Plugin>(p: *mut ()) {
    unsafe { (*(p as *mut P)).start_processing() }
}

fn thunk_stop<P: Plugin>(p: *mut ()) {
    unsafe { (*(p as *mut P)).stop_processing() }
}

fn thunk_reset<P: Plugin>(p: *mut ()) {
    unsafe { (*(p as *mut P)).reset() }
}

fn thunk_process<P: Plugin>(p: *mut (), ctx: &mut ProcessCtx) -> ProcessStatus {
    unsafe { (*(p as *mut P)).process(ctx) }
}

fn thunk_get_param<P: Plugin>(p: *mut (), id: u32) -> f64 {
    unsafe { (*(p as *const P)).get_param(id) }
}

fn thunk_set_param<P: Plugin>(p: *mut (), id: u32, value: f64) {
    unsafe { (*(p as *mut P)).set_param(id, value) }
}

fn thunk_latency_samples<P: Plugin>(p: *mut ()) -> u32 {
    unsafe { (*(p as *const P)).latency_samples() }
}

// ---------------------------------------------------------------------------
// Linear-memory read/write helpers.
// ---------------------------------------------------------------------------

#[inline]
unsafe fn write_u32(addr: u32, value: u32) {
    core::ptr::write_unaligned(addr as *mut u32, value);
}

#[inline]
unsafe fn read_u32(addr: u32) -> u32 {
    core::ptr::read_unaligned(addr as *const u32)
}

/// strncmp-style compare for a host-supplied pointer against a Rust slice
/// that ends in NUL.
fn cstr_eq(ptr: u32, expected: &[u8]) -> bool {
    let p = ptr as *const u8;
    for (i, &want) in expected.iter().enumerate() {
        let got = unsafe { *p.add(i) };
        if got != want {
            return false;
        }
        if want == 0 {
            return true;
        }
    }
    false
}

fn cstr_eq_static(ptr: u32, expected: *const u8) -> bool {
    if expected.is_null() {
        return false;
    }
    let p = ptr as *const u8;
    let mut i = 0;
    loop {
        let want = unsafe { *expected.add(i) };
        let got = unsafe { *p.add(i) };
        if got != want {
            return false;
        }
        if want == 0 {
            return true;
        }
        i += 1;
    }
}

/// Read the channel count for the requested port from the raw process
/// struct. Returns 0 if the port index is out of range or the port table
/// pointer is null.
unsafe fn channel_count(process_ptr: u32, is_input: bool, port: usize) -> u32 {
    let (ports_ptr_off, count_off) = if is_input {
        (offsets::process_::AUDIO_INPUTS, offsets::process_::AUDIO_INPUTS_COUNT)
    } else {
        (offsets::process_::AUDIO_OUTPUTS, offsets::process_::AUDIO_OUTPUTS_COUNT)
    };
    let port_count = read_u32(process_ptr + count_off as u32) as usize;
    if port >= port_count {
        return 0;
    }
    let ports_ptr = read_u32(process_ptr + ports_ptr_off as u32);
    if ports_ptr == 0 {
        return 0;
    }
    let buf_ptr = ports_ptr + (port as u32) * (offsets::audio_buffer::SIZE as u32);
    read_u32(buf_ptr + offsets::audio_buffer::CHANNEL_COUNT as u32)
}

/// Resolve `(channel_pointer, frame_count)` for a given port/channel from
/// the raw process struct. `is_input` selects the input vs output side.
unsafe fn channel_slice(
    process_ptr: u32,
    is_input: bool,
    port: usize,
    ch: usize,
    _writable: bool,
) -> Option<(*mut f32, usize)> {
    let frames = read_u32(process_ptr + offsets::process_::FRAMES_COUNT as u32) as usize;
    let (ports_ptr_off, count_off) = if is_input {
        (offsets::process_::AUDIO_INPUTS, offsets::process_::AUDIO_INPUTS_COUNT)
    } else {
        (offsets::process_::AUDIO_OUTPUTS, offsets::process_::AUDIO_OUTPUTS_COUNT)
    };
    let count = read_u32(process_ptr + count_off as u32) as usize;
    if port >= count {
        return None;
    }
    let ports_ptr = read_u32(process_ptr + ports_ptr_off as u32);
    if ports_ptr == 0 {
        return None;
    }
    let buf_ptr = ports_ptr + (port as u32) * (offsets::audio_buffer::SIZE as u32);
    let channel_array = read_u32(buf_ptr + offsets::audio_buffer::DATA32 as u32);
    let channel_count = read_u32(buf_ptr + offsets::audio_buffer::CHANNEL_COUNT as u32) as usize;
    if channel_array == 0 || ch >= channel_count {
        return None;
    }
    let ch_ptr = read_u32(channel_array + (ch as u32) * 4);
    if ch_ptr == 0 {
        return None;
    }
    Some((ch_ptr as *mut f32, frames))
}

// ---------------------------------------------------------------------------
// clap_entry — init / get_factory
// ---------------------------------------------------------------------------

extern "C" fn entry_init(plugin_path_ptr: u32) -> u32 {
    // The host passes us the per-instance modulePath (e.g.
    // `/plugin/<hash>`) here. We stash it so `webview.get_uri` can build
    // the absolute URI `file:<modulePath><ui_path>` that resolves through
    // the SW proxy to the tarball's file map.
    unsafe {
        DEF.module_path_ptr = plugin_path_ptr;
    }
    1
}

extern "C" fn entry_get_factory(id_ptr: u32) -> u32 {
    if cstr_eq(id_ptr, FACTORY_ID) {
        return addr_of!(FACTORY) as u32;
    }
    0
}

// ---------------------------------------------------------------------------
// Factory — exactly one plugin per wasm module for now.
// ---------------------------------------------------------------------------

extern "C" fn factory_get_plugin_count(_factory_ptr: u32) -> u32 {
    1
}

extern "C" fn factory_get_plugin_descriptor(_factory_ptr: u32, index: u32) -> u32 {
    if index != 0 {
        return 0;
    }
    addr_of!(DESCRIPTOR) as u32
}

extern "C" fn factory_create_plugin(_factory: u32, host: u32, plugin_id_ptr: u32) -> u32 {
    let id_ptr = unsafe { DEF.id_ptr };
    if !cstr_eq_static(plugin_id_ptr, id_ptr) {
        return 0;
    }
    // Stash the host pointer so plugin.init can resolve host extensions
    // and process() can later call host_webview.send for plugin→UI pushes.
    unsafe {
        DEF.host_ptr = host;
    }

    let plugin_ptr = malloc(offsets::plugin::SIZE as u32);
    if plugin_ptr == 0 {
        return 0;
    }

    // Allocate the plugin's own state on the heap and stash its pointer in
    // plugin_data. Every shim recovers it via `read_plugin_data`.
    let state_ptr = match unsafe { VTABLE.new } {
        Some(f) => f(),
        None => core::ptr::null_mut(),
    } as u32;

    unsafe {
        write_u32(plugin_ptr + offsets::plugin::DESC as u32, addr_of!(DESCRIPTOR) as u32);
        write_u32(plugin_ptr + offsets::plugin::PLUGIN_DATA as u32, state_ptr);
        write_u32(plugin_ptr + offsets::plugin::INIT as u32, plugin_init as usize as u32);
        write_u32(plugin_ptr + offsets::plugin::DESTROY as u32, plugin_destroy as usize as u32);
        write_u32(plugin_ptr + offsets::plugin::ACTIVATE as u32, plugin_activate as usize as u32);
        write_u32(plugin_ptr + offsets::plugin::DEACTIVATE as u32, plugin_deactivate as usize as u32);
        write_u32(plugin_ptr + offsets::plugin::START_PROCESSING as u32, plugin_start_processing as usize as u32);
        write_u32(plugin_ptr + offsets::plugin::STOP_PROCESSING as u32, plugin_stop_processing as usize as u32);
        write_u32(plugin_ptr + offsets::plugin::RESET as u32, plugin_reset as usize as u32);
        write_u32(plugin_ptr + offsets::plugin::PROCESS as u32, plugin_process as usize as u32);
        write_u32(plugin_ptr + offsets::plugin::GET_EXTENSION as u32, plugin_get_extension as usize as u32);
        write_u32(plugin_ptr + offsets::plugin::ON_MAIN_THREAD as u32, plugin_on_main_thread as usize as u32);
    }
    plugin_ptr
}

#[inline]
unsafe fn read_plugin_data(plugin_ptr: u32) -> *mut () {
    read_u32(plugin_ptr + offsets::plugin::PLUGIN_DATA as u32) as *mut ()
}

// ---------------------------------------------------------------------------
// Plugin lifecycle shims — all dispatch via VTABLE.
// ---------------------------------------------------------------------------

extern "C" fn plugin_init(_plugin_ptr: u32) -> u32 {
    // Resolve host_webview.send now (host.get_extension is guaranteed to
    // work from plugin.init onward, per the CLAP spec). Failure is silent
    // — the plugin just won't be able to push messages to the UI.
    unsafe {
        if DEF.host_ptr != 0 {
            let get_ext_idx =
                read_u32(DEF.host_ptr + offsets::host::GET_EXTENSION as u32);
            if get_ext_idx != 0 {
                static EXT: &[u8] = b"clap.webview/3\0";
                type GetExt = extern "C" fn(host: u32, ext_id: u32) -> u32;
                let f: GetExt = core::mem::transmute(get_ext_idx as usize);
                let webview_struct = f(DEF.host_ptr, EXT.as_ptr() as u32);
                if webview_struct != 0 {
                    DEF.host_webview_send = read_u32(
                        webview_struct + offsets::host_webview::SEND as u32,
                    );
                }
            }
        }
    }
    1
}

extern "C" fn plugin_destroy(plugin_ptr: u32) {
    unsafe {
        let data = read_plugin_data(plugin_ptr);
        if let Some(f) = VTABLE.drop {
            f(data);
        }
        write_u32(plugin_ptr + offsets::plugin::PLUGIN_DATA as u32, 0);
    }
    // The clap_plugin struct itself was allocated by `malloc` (which lives
    // in *our* heap); freeing it is the host's responsibility per CLAP.
}

extern "C" fn plugin_activate(
    plugin_ptr: u32,
    sample_rate: f64,
    _min_frames: u32,
    max_frames: u32,
) -> u32 {
    unsafe {
        if let Some(f) = VTABLE.activate {
            f(read_plugin_data(plugin_ptr), sample_rate, max_frames);
        }
    }
    1
}

extern "C" fn plugin_deactivate(plugin_ptr: u32) {
    unsafe {
        if let Some(f) = VTABLE.deactivate {
            f(read_plugin_data(plugin_ptr));
        }
    }
}

extern "C" fn plugin_start_processing(plugin_ptr: u32) -> u32 {
    unsafe {
        if let Some(f) = VTABLE.start_processing {
            f(read_plugin_data(plugin_ptr));
        }
    }
    1
}

extern "C" fn plugin_stop_processing(plugin_ptr: u32) {
    unsafe {
        if let Some(f) = VTABLE.stop_processing {
            f(read_plugin_data(plugin_ptr));
        }
    }
}

extern "C" fn plugin_reset(plugin_ptr: u32) {
    unsafe {
        if let Some(f) = VTABLE.reset {
            f(read_plugin_data(plugin_ptr));
        }
    }
}

extern "C" fn plugin_process(plugin_ptr: u32, process: u32) -> u32 {
    let mut ctx = ProcessCtx { process_ptr: process };
    let status = unsafe {
        match VTABLE.process {
            Some(f) => f(read_plugin_data(plugin_ptr), &mut ctx),
            None => ProcessStatus::Sleep,
        }
    };
    status as u32
}

extern "C" fn plugin_get_extension(_plugin_ptr: u32, ext_id_ptr: u32) -> u32 {
    if cstr_eq(ext_id_ptr, EXT_AUDIO_PORTS) {
        return addr_of!(AUDIO_PORTS_EXT) as u32;
    }
    if cstr_eq(ext_id_ptr, EXT_NOTE_PORTS) {
        return addr_of!(NOTE_PORTS_EXT) as u32;
    }
    if cstr_eq(ext_id_ptr, EXT_WEBVIEW) {
        // Only expose the extension if the plugin actually has a UI; the
        // host treats a null return as "no UI" and skips the iframe.
        if unsafe { !DEF.ui_path_ptr.is_null() } {
            return addr_of!(WEBVIEW_EXT) as u32;
        }
    }
    if cstr_eq(ext_id_ptr, EXT_PARAMS) {
        // Only expose if the plugin declared at least one param.
        if unsafe { DEF.params_len > 0 } {
            return addr_of!(PARAMS_EXT) as u32;
        }
    }
    if cstr_eq(ext_id_ptr, EXT_STATE) {
        // Param-dump state only makes sense with params.
        if unsafe { DEF.params_len > 0 } {
            return addr_of!(STATE_EXT) as u32;
        }
    }
    if cstr_eq(ext_id_ptr, EXT_LATENCY) {
        // Always exposed — plugins default to 0 samples if they don't
        // override `latency_samples()`, which is the right answer for
        // hosts that don't ask.
        return addr_of!(LATENCY_EXT) as u32;
    }
    0
}

extern "C" fn plugin_on_main_thread(_plugin_ptr: u32) {}

// ---------------------------------------------------------------------------
// clap.audio-ports — channel count is fixed at 2 (stereo) per port.
// ---------------------------------------------------------------------------

extern "C" fn audio_ports_count(_plugin: u32, is_input: u32) -> u32 {
    unsafe {
        if is_input != 0 {
            DEF.audio_inputs as u32
        } else {
            DEF.audio_outputs as u32
        }
    }
}

extern "C" fn audio_ports_get(_plugin: u32, index: u32, is_input: u32, info_ptr: u32) -> u32 {
    let count = audio_ports_count(0, is_input);
    if index >= count {
        return 0;
    }
    unsafe {
        core::ptr::write_bytes(info_ptr as *mut u8, 0, offsets::audio_port_info::SIZE);
        write_u32(info_ptr + offsets::audio_port_info::ID as u32, index);
        let name: &[u8] = if is_input != 0 { b"Input\0" } else { b"Output\0" };
        let dst = info_ptr as usize + offsets::audio_port_info::NAME_OFFSET;
        for (i, &b) in name.iter().enumerate() {
            *((dst + i) as *mut u8) = b;
        }
        write_u32(info_ptr + offsets::audio_port_info::FLAGS as u32, PORT_FLAG_IS_MAIN);
        write_u32(info_ptr + offsets::audio_port_info::CHANNEL_COUNT as u32, 2);
    }
    1
}

// ---------------------------------------------------------------------------
// clap.note-ports — instrument plugins; otherwise empty.
// ---------------------------------------------------------------------------

extern "C" fn note_ports_count(_plugin: u32, is_input: u32) -> u32 {
    unsafe {
        if is_input != 0 {
            DEF.note_inputs as u32
        } else {
            0
        }
    }
}

extern "C" fn note_ports_get(_plugin: u32, index: u32, is_input: u32, info_ptr: u32) -> u32 {
    let count = note_ports_count(0, is_input);
    if index >= count {
        return 0;
    }
    unsafe {
        core::ptr::write_bytes(info_ptr as *mut u8, 0, offsets::note_port_info::SIZE);
        write_u32(info_ptr + offsets::note_port_info::ID as u32, index);
        write_u32(
            info_ptr + offsets::note_port_info::SUPPORTED_DIALECTS as u32,
            NOTE_DIALECT_CLAP | NOTE_DIALECT_MIDI,
        );
        write_u32(
            info_ptr + offsets::note_port_info::PREFERRED_DIALECT as u32,
            NOTE_DIALECT_CLAP,
        );
        let name: &[u8] = b"Notes\0";
        let dst = info_ptr as usize + offsets::note_port_info::NAME_OFFSET;
        for (i, &b) in name.iter().enumerate() {
            *((dst + i) as *mut u8) = b;
        }
    }
    1
}

// ---------------------------------------------------------------------------
// clap.webview/3 — two-call probe for `get_uri`, plus stub `receive` and
// `get_resource`. The host serves the iframe's static assets out of the
// tarball directly (via its `getFile` map), so `get_resource` is a no-op
// that returns false to let the host fall back to its own resolver.
// `receive` accepts but drops messages until clap.params is implemented —
// once it lands, this is where we'll parse `{ set: [id, value] }` CBOR.
// ---------------------------------------------------------------------------

/// Compose `file:<modulePath><ui_path>` into the host-supplied buffer.
/// Two-call probe: `cap == 0` returns required byte length (excluding the
/// trailing NUL); `cap > 0` writes bytes (clamped) and terminates.
///
/// `modulePath` comes from `entry.init`; without it the URI would be
/// `file:<ui_path>` which the SW proxy fails to match against the file
/// map (which is keyed by `/plugin/<hash>/...`).
extern "C" fn webview_get_uri(_plugin_ptr: u32, buf_ptr: u32, cap: u32) -> i32 {
    const PREFIX: &[u8] = b"file:";

    let (ui_ptr, ui_len) = unsafe { (DEF.ui_path_ptr, DEF.ui_path_len) };
    if ui_ptr.is_null() {
        return 0;
    }
    let module_ptr = unsafe { DEF.module_path_ptr };
    let module_len = if module_ptr == 0 {
        0
    } else {
        cstr_len(module_ptr)
    };

    let total = PREFIX.len() as u32 + module_len + ui_len;

    if cap == 0 {
        return total as i32;
    }

    let writable = core::cmp::min(total, cap.saturating_sub(1));
    let dst = buf_ptr as *mut u8;
    unsafe {
        // "file:"
        let p1 = core::cmp::min(writable, PREFIX.len() as u32);
        core::ptr::copy_nonoverlapping(PREFIX.as_ptr(), dst, p1 as usize);

        // module path
        let remaining_after_prefix = writable.saturating_sub(p1);
        let p2 = core::cmp::min(remaining_after_prefix, module_len);
        if p2 > 0 {
            core::ptr::copy_nonoverlapping(
                module_ptr as *const u8,
                dst.add(p1 as usize),
                p2 as usize,
            );
        }

        // ui path
        let remaining_after_module = writable.saturating_sub(p1 + p2);
        let p3 = core::cmp::min(remaining_after_module, ui_len);
        if p3 > 0 {
            core::ptr::copy_nonoverlapping(
                ui_ptr,
                dst.add((p1 + p2) as usize),
                p3 as usize,
            );
        }

        // NUL terminate
        *dst.add(writable as usize) = 0;
    }
    total as i32
}

/// strlen for a u32-addressed NUL-terminated string in our linear memory.
fn cstr_len(ptr: u32) -> u32 {
    let mut n = 0u32;
    loop {
        let b = unsafe { *((ptr + n) as *const u8) };
        if b == 0 {
            return n;
        }
        n += 1;
    }
}

extern "C" fn webview_get_resource(
    _plugin_ptr: u32,
    _path_ptr: u32,
    _mime_buf: u32,
    _mime_cap: u32,
    _ostream_ptr: u32,
) -> u32 {
    // Falsy return → host falls back to its tarball `getFile` resolver,
    // which already has every file we shipped under `widgets/` and `ui/`.
    0
}

/// `webview.receive(plugin, buf, size)` — bytes from the UI iframe.
/// We accept the simple `{set:[<u32 id>, <f64 value>]}` CBOR shape used by
/// the auto-pan / vocal-* UIs (see widgets/cbor.mjs). Anything else is
/// silently dropped — extending the protocol means adding parsers here.
extern "C" fn webview_receive(plugin_ptr: u32, buf_ptr: u32, size: u32) -> u32 {
    let p = buf_ptr as *const u8;
    // Ready: text(5) "ready" — the UI just (re)opened on its hardcoded
    // defaults. Reply with a full param snapshot ({params:{id:value}})
    // so it reflects the plugin's ACTUAL state (possibly restored from
    // the project via clap.state). The widget transport's
    // decodeParamsSnapshot consumes exactly this shape.
    if size == 6 {
        unsafe {
            if *p == 0x65
                && *p.add(1) == b'r' && *p.add(2) == b'e' && *p.add(3) == b'a'
                && *p.add(4) == b'd' && *p.add(5) == b'y'
            {
                push_params_snapshot(plugin_ptr);
                return 1;
            }
        }
    }
    if size < 20 {
        return 1;
    }
    unsafe {
        // 0xa1 = map(1)
        if *p != 0xa1 {
            return 1;
        }
        // 0x63 "set"
        if *p.add(1) != 0x63 || *p.add(2) != 0x73 || *p.add(3) != 0x65 || *p.add(4) != 0x74 {
            return 1;
        }
        // 0x82 = array(2)
        if *p.add(5) != 0x82 {
            return 1;
        }
        // 0x1a = u32; then 4 big-endian bytes
        if *p.add(6) != 0x1a {
            return 1;
        }
        let id = u32::from_be_bytes([*p.add(7), *p.add(8), *p.add(9), *p.add(10)]);
        // 0xfb = f64; then 8 big-endian bytes
        if *p.add(11) != 0xfb {
            return 1;
        }
        let mut vbytes = [0u8; 8];
        for i in 0..8 {
            vbytes[i] = *p.add(12 + i);
        }
        let value = f64::from_be_bytes(vbytes);

        let data = read_plugin_data(plugin_ptr);
        if let Some(f) = VTABLE.set_param {
            f(data, id, value);
        }
    }
    1
}

// ---------------------------------------------------------------------------
// clap.params — count / get_info / get_value, plus no-op flush + text
// conversions. The plugin's params are declared as `&'static [ParamDef]`
// via `Plugin::params()`; the C-ABI shims read them straight out of DEF
// without needing P-typing.
// ---------------------------------------------------------------------------

#[inline]
unsafe fn param_def(idx: u32) -> Option<&'static ParamDef> {
    if idx >= DEF.params_len || DEF.params_ptr.is_null() {
        return None;
    }
    Some(&*DEF.params_ptr.add(idx as usize))
}

extern "C" fn params_count(_plugin_ptr: u32) -> u32 {
    unsafe { DEF.params_len }
}

extern "C" fn params_get_info(_plugin_ptr: u32, index: u32, info_ptr: u32) -> u32 {
    unsafe {
        let Some(def) = param_def(index) else { return 0 };
        // Zero the whole struct, then write fields. The struct is 1320
        // bytes; the host allocated it inside our memory.
        core::ptr::write_bytes(info_ptr as *mut u8, 0, offsets::param_info::SIZE);
        write_u32(info_ptr + offsets::param_info::ID as u32, def.id);
        write_u32(info_ptr + offsets::param_info::FLAGS as u32, def.flags);
        write_str_into(
            info_ptr + offsets::param_info::NAME as u32,
            offsets::param_info::NAME_CAP,
            def.name,
        );
        write_str_into(
            info_ptr + offsets::param_info::MODULE as u32,
            offsets::param_info::MODULE_CAP,
            def.module,
        );
        write_f64(info_ptr + offsets::param_info::MIN_VALUE as u32, def.min);
        write_f64(info_ptr + offsets::param_info::MAX_VALUE as u32, def.max);
        write_f64(
            info_ptr + offsets::param_info::DEFAULT_VALUE as u32,
            def.default,
        );
    }
    1
}

extern "C" fn params_get_value(plugin_ptr: u32, id: u32, out_ptr: u32) -> u32 {
    let v = unsafe {
        let data = read_plugin_data(plugin_ptr);
        match VTABLE.get_param {
            Some(f) => f(data, id),
            None => 0.0,
        }
    };
    unsafe { write_f64(out_ptr, v) };
    1
}

extern "C" fn params_value_to_text(
    _plugin_ptr: u32,
    _id: u32,
    _value: u64,
    _buf_ptr: u32,
    _cap: u32,
) -> u32 {
    // Host falls back to a default decimal formatter if we return false.
    0
}

extern "C" fn params_text_to_value(
    _plugin_ptr: u32,
    _id: u32,
    _text_ptr: u32,
    _out_ptr: u32,
) -> u32 {
    0
}

extern "C" fn params_flush(
    _plugin_ptr: u32,
    _in_events_ptr: u32,
    _out_events_ptr: u32,
) {
    // No-op for Stage A — UI param changes arrive through webview.receive,
    // not the event queue. DAW-driven automation events come in Stage B.
}

extern "C" fn latency_get(plugin_ptr: u32) -> u32 {
    unsafe {
        let data = read_plugin_data(plugin_ptr);
        match VTABLE.latency_samples {
            Some(f) => f(data),
            None => 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers used by the param-info writer.
// ---------------------------------------------------------------------------

#[inline]
unsafe fn write_f64(addr: u32, value: f64) {
    core::ptr::write_unaligned(addr as *mut f64, value);
}

/// Copy a NUL-terminated source slice into a fixed-capacity destination
/// inside the plugin's memory, clamped to `cap - 1` so the host's null
/// terminator survives. Source's trailing NUL is dropped if present.
unsafe fn write_str_into(dst: u32, cap: usize, src: &[u8]) {
    let src_len = src.iter().position(|&b| b == 0).unwrap_or(src.len());
    let n = core::cmp::min(src_len, cap.saturating_sub(1));
    core::ptr::copy_nonoverlapping(src.as_ptr(), dst as *mut u8, n);
    *((dst + n as u32) as *mut u8) = 0;
}

// Silence "may be unused" warnings on non-wasm hosts.
#[allow(dead_code)]
fn _keep_alive() {
    let _ = addr_of_mut!(clap_entry);
    let _ = addr_of_mut!(FACTORY);
    let _ = addr_of_mut!(DESCRIPTOR);
    let _ = addr_of_mut!(AUDIO_PORTS_EXT);
    let _ = addr_of_mut!(NOTE_PORTS_EXT);
    let _ = addr_of_mut!(WEBVIEW_EXT);
    let _ = addr_of_mut!(PARAMS_EXT);
    let _ = addr_of_mut!(LATENCY_EXT);
    let _ = addr_of_mut!(FEATURES_TABLE);
    let _ = addr_of_mut!(DEF);
    let _ = addr_of_mut!(VTABLE);
}

// ---------------------------------------------------------------------------
// clap.state — generic param-dump persistence. Save serializes every
// declared param's (id, value); load replays the pairs through set_param.
// Every plugin built on this crate gets project-persistent knob state for
// free. Blob layout (all little-endian):
//   u32 magic "PLST" · u32 version (1) · u32 count · count × (u32 id, f64 value)
// Unknown ids on load are still forwarded to set_param, which ignores
// them — so older blobs survive param-surface growth.
// ---------------------------------------------------------------------------

const STATE_MAGIC: u32 = 0x504c_5354; // "PLST"

/// `clap_ostream.write` / `clap_istream.read` — host-stub fn ptr at +4,
/// called as `(stream*, buf*, u64 size) -> i64`.
type StreamIoFn = extern "C" fn(stream: u32, buf: u32, size: u64) -> i64;

unsafe fn stream_io_fn(stream_ptr: u32) -> Option<StreamIoFn> {
    let idx = read_u32(stream_ptr + 4);
    if idx == 0 {
        return None;
    }
    Some(core::mem::transmute::<usize, StreamIoFn>(idx as usize))
}

extern "C" fn state_save(plugin_ptr: u32, ostream_ptr: u32) -> u32 {
    unsafe {
        let Some(write) = stream_io_fn(ostream_ptr) else { return 0 };
        let Some(get) = VTABLE.get_param else { return 0 };
        let data = read_plugin_data(plugin_ptr);
        let n = DEF.params_len;
        let mut buf: alloc::vec::Vec<u8> =
            alloc::vec::Vec::with_capacity(12 + n as usize * 12);
        buf.extend_from_slice(&STATE_MAGIC.to_le_bytes());
        buf.extend_from_slice(&1u32.to_le_bytes());
        buf.extend_from_slice(&n.to_le_bytes());
        for i in 0..n {
            let Some(def) = param_def(i) else { continue };
            buf.extend_from_slice(&def.id.to_le_bytes());
            buf.extend_from_slice(&get(data, def.id).to_le_bytes());
        }
        let mut off = 0usize;
        while off < buf.len() {
            let w = write(
                ostream_ptr,
                buf.as_ptr().add(off) as u32,
                (buf.len() - off) as u64,
            );
            if w <= 0 {
                return 0;
            }
            off += w as usize;
        }
    }
    1
}

unsafe fn stream_read_exact(read: StreamIoFn, istream_ptr: u32, out: &mut [u8]) -> bool {
    let mut off = 0usize;
    while off < out.len() {
        let r = read(
            istream_ptr,
            out.as_mut_ptr().add(off) as u32,
            (out.len() - off) as u64,
        );
        if r <= 0 {
            return false;
        }
        off += r as usize;
    }
    true
}

extern "C" fn state_load(plugin_ptr: u32, istream_ptr: u32) -> u32 {
    unsafe {
        let Some(read) = stream_io_fn(istream_ptr) else { return 0 };
        let Some(set) = VTABLE.set_param else { return 0 };
        let data = read_plugin_data(plugin_ptr);
        let mut hdr = [0u8; 12];
        if !stream_read_exact(read, istream_ptr, &mut hdr) {
            return 0;
        }
        let magic = u32::from_le_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]);
        let version = u32::from_le_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]);
        let count = u32::from_le_bytes([hdr[8], hdr[9], hdr[10], hdr[11]]);
        if magic != STATE_MAGIC || version != 1 {
            return 0;
        }
        // 4096-param ceiling — a corrupt count can't spin us forever.
        let mut pair = [0u8; 12];
        for _ in 0..count.min(4096) {
            if !stream_read_exact(read, istream_ptr, &mut pair) {
                return 0;
            }
            let id = u32::from_le_bytes([pair[0], pair[1], pair[2], pair[3]]);
            let value = f64::from_le_bytes([
                pair[4], pair[5], pair[6], pair[7], pair[8], pair[9], pair[10], pair[11],
            ]);
            set(data, id, value);
        }
    }
    1
}

/// Push the full param surface to the plugin's UI as a CBOR
/// `{params:{<id>:<f64>, …}}` snapshot via `clap_host_webview.send`.
/// No-op when the host didn't expose the webview ext or there are no
/// params. Map header uses the short form (≤23 entries) or the 1-byte
/// length form (≤255).
fn push_params_snapshot(plugin_ptr: u32) {
    unsafe {
        let host = DEF.host_ptr;
        let send_idx = DEF.host_webview_send;
        let n = DEF.params_len;
        let Some(get) = VTABLE.get_param else { return };
        if host == 0 || send_idx == 0 || n == 0 || n > 255 {
            return;
        }
        let data = read_plugin_data(plugin_ptr);
        let mut buf: alloc::vec::Vec<u8> =
            alloc::vec::Vec::with_capacity(10 + n as usize * 14);
        buf.push(0xa1);
        buf.push(0x66);
        buf.extend_from_slice(b"params");
        if n <= 23 {
            buf.push(0xa0 | (n as u8));
        } else {
            buf.push(0xb8);
            buf.push(n as u8);
        }
        for i in 0..n {
            let Some(def) = param_def(i) else { continue };
            buf.push(0x1a);
            buf.extend_from_slice(&def.id.to_be_bytes());
            buf.push(0xfb);
            buf.extend_from_slice(&get(data, def.id).to_be_bytes());
        }
        type Send = extern "C" fn(host: u32, buf: u32, size: u32) -> u32;
        let f: Send = core::mem::transmute(send_idx as usize);
        f(host, buf.as_ptr() as u32, buf.len() as u32);
    }
}
