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
}

const FACTORY_ID: &[u8] = b"clap.plugin-factory\0";
const EXT_AUDIO_PORTS: &[u8] = b"clap.audio-ports\0";
const EXT_NOTE_PORTS: &[u8] = b"clap.note-ports\0";

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
}

/// Four-slice view of one stereo block.
pub struct StereoIo<'a> {
    pub input_l: &'a [f32],
    pub input_r: &'a [f32],
    pub output_l: &'a mut [f32],
    pub output_r: &'a mut [f32],
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
}
static mut DEF: DefSnapshot = DefSnapshot {
    audio_inputs: 0,
    audio_outputs: 0,
    note_inputs: 0,
    id_ptr: core::ptr::null(),
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

        VTABLE.new = Some(thunk_new::<P>);
        VTABLE.drop = Some(thunk_drop::<P>);
        VTABLE.activate = Some(thunk_activate::<P>);
        VTABLE.deactivate = Some(thunk_deactivate::<P>);
        VTABLE.start_processing = Some(thunk_start::<P>);
        VTABLE.stop_processing = Some(thunk_stop::<P>);
        VTABLE.reset = Some(thunk_reset::<P>);
        VTABLE.process = Some(thunk_process::<P>);
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

extern "C" fn entry_init(_plugin_path_ptr: u32) -> u32 {
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

extern "C" fn factory_create_plugin(_factory: u32, _host: u32, plugin_id_ptr: u32) -> u32 {
    let id_ptr = unsafe { DEF.id_ptr };
    if !cstr_eq_static(plugin_id_ptr, id_ptr) {
        return 0;
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

// Silence "may be unused" warnings on non-wasm hosts.
#[allow(dead_code)]
fn _keep_alive() {
    let _ = addr_of_mut!(clap_entry);
    let _ = addr_of_mut!(FACTORY);
    let _ = addr_of_mut!(DESCRIPTOR);
    let _ = addr_of_mut!(AUDIO_PORTS_EXT);
    let _ = addr_of_mut!(NOTE_PORTS_EXT);
    let _ = addr_of_mut!(FEATURES_TABLE);
    let _ = addr_of_mut!(DEF);
    let _ = addr_of_mut!(VTABLE);
}
