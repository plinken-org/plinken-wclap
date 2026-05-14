//! Render a 440 Hz sine through `clack-gain.wasm` (or any wclap-cpp-style
//! CLAP plugin compiled to wasm32) and write the output as a WAV.
//!
//! Usage:
//!   cargo run --example tiny_host -- [plugin.wasm] [out.wav]
//!
//! Walks the CLAP entry table by hand — no `clack-host` adapter yet (that's
//! M3). The flow mirrors the CLAP spec:
//!
//!   1. read `clap_entry` global, call `init` and `get_factory` indirectly.
//!   2. walk `clap_plugin_factory_t` (count / descriptor / create).
//!   3. allocate a `clap_host_t` in wasm memory and point its callbacks at
//!      host stubs installed into `__indirect_function_table`.
//!   4. create the plugin, drive activate → start_processing → process.
//!   5. copy audio in/out through wasm memory each block, write to WAV.
//!
//! For a self-contained plugin like `clack-gain` we don't need a WASI ctx
//! or a `Linker`: the module has zero imports.

use std::env;
use std::f32::consts::TAU;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use wasmtime::{AsContext, Func, Instance, Memory, Ref, Store, Table, TypedFunc, Val};
use wclap_host::{Bundle, Engine};

const SAMPLE_RATE: u32 = 48_000;
const BLOCK: u32 = 128;
const SECONDS: u32 = 1;
const FREQ: f32 = 440.0;

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let plugin_path = env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from("../../apps/wclap-host/public/samples/clack-gain.wasm")
    });
    let out_wav = env::args()
        .nth(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("out.wav"));

    let bytes = fs::read(&plugin_path)
        .with_context(|| format!("reading {}", plugin_path.display()))?;
    println!("loaded {} bytes from {}", bytes.len(), plugin_path.display());

    let engine = Engine::new()?;
    let bundle = Bundle::load(&engine, &bytes).map_err(|e| anyhow::anyhow!("Bundle::load: {e}"))?;

    let mut store: Store<()> = Store::new(engine.inner_ref(), ());
    let instance = Instance::new(&mut store, bundle.module_ref(), &[])
        .context("instantiate wasm module")?;

    // ── core exports ──────────────────────────────────────────────────
    let memory = instance
        .get_memory(&mut store, "memory")
        .context("missing `memory` export")?;
    let table = instance
        .get_table(&mut store, "__indirect_function_table")
        .context("missing `__indirect_function_table` export")?;
    let malloc: TypedFunc<u32, u32> = instance
        .get_typed_func(&mut store, "malloc")
        .context("missing `malloc` export")?;

    // ── parse clap_entry ──────────────────────────────────────────────
    let entry_ptr = match instance
        .get_global(&mut store, "clap_entry")
        .context("missing `clap_entry` export")?
        .get(&mut store)
    {
        Val::I32(v) => v as u32,
        v => bail!("clap_entry global isn't i32 (got {v:?})"),
    };
    let entry_major = read_u32(&store, memory, entry_ptr)?;
    let entry_minor = read_u32(&store, memory, entry_ptr + 4)?;
    let entry_rev = read_u32(&store, memory, entry_ptr + 8)?;
    let entry_init = read_u32(&store, memory, entry_ptr + 12)?;
    let entry_deinit = read_u32(&store, memory, entry_ptr + 16)?;
    let entry_get_factory = read_u32(&store, memory, entry_ptr + 20)?;
    println!(
        "clap_entry v{entry_major}.{entry_minor}.{entry_rev}  init=#{entry_init} deinit=#{entry_deinit} get_factory=#{entry_get_factory}"
    );

    // ── install host & event stubs into the indirect function table ───
    // clack-gain probably never invokes these, but installing real
    // callables means any future plugin that does won't trap on a null
    // funcref.
    let s_get_ext = Func::wrap(&mut store, |_h: i32, _id: i32| -> i32 { 0 });
    let s_req_restart = Func::wrap(&mut store, |_h: i32| {});
    let s_req_process = Func::wrap(&mut store, |_h: i32| {});
    let s_req_callback = Func::wrap(&mut store, |_h: i32| {});
    let s_ev_size = Func::wrap(&mut store, |_l: i32| -> i32 { 0 });
    let s_ev_get = Func::wrap(&mut store, |_l: i32, _i: i32| -> i32 { 0 });
    let s_ev_try_push = Func::wrap(&mut store, |_l: i32, _e: i32| -> i32 { 0 });

    let idx_get_ext = table.grow(&mut store, 1, Ref::Func(Some(s_get_ext)))? as u32;
    let idx_req_restart = table.grow(&mut store, 1, Ref::Func(Some(s_req_restart)))? as u32;
    let idx_req_process = table.grow(&mut store, 1, Ref::Func(Some(s_req_process)))? as u32;
    let idx_req_callback = table.grow(&mut store, 1, Ref::Func(Some(s_req_callback)))? as u32;
    let idx_ev_size = table.grow(&mut store, 1, Ref::Func(Some(s_ev_size)))? as u32;
    let idx_ev_get = table.grow(&mut store, 1, Ref::Func(Some(s_ev_get)))? as u32;
    let idx_ev_try_push = table.grow(&mut store, 1, Ref::Func(Some(s_ev_try_push)))? as u32;

    // ── clap_entry.init("") ───────────────────────────────────────────
    let path_ptr = alloc_cstr(&mut store, memory, &malloc, "")?;
    let init_ok: i32 = call::<i32, i32>(&mut store, table, entry_init, path_ptr as i32)?;
    if init_ok == 0 {
        bail!("clap_entry.init returned false");
    }

    // ── clap_entry.get_factory("clap.plugin-factory") ─────────────────
    let factory_id_ptr = alloc_cstr(&mut store, memory, &malloc, "clap.plugin-factory")?;
    let factory_ptr =
        call::<i32, i32>(&mut store, table, entry_get_factory, factory_id_ptr as i32)? as u32;
    if factory_ptr == 0 {
        bail!("get_factory returned null — plugin doesn't expose clap.plugin-factory");
    }
    let fac_count = read_u32(&store, memory, factory_ptr)?;
    let fac_descriptor = read_u32(&store, memory, factory_ptr + 4)?;
    let fac_create = read_u32(&store, memory, factory_ptr + 8)?;

    let n_plugins =
        call::<i32, i32>(&mut store, table, fac_count, factory_ptr as i32)? as u32;
    println!("plugin_count = {n_plugins}");
    if n_plugins == 0 {
        bail!("factory has no plugins");
    }
    let desc_ptr = call::<(i32, i32), i32>(
        &mut store,
        table,
        fac_descriptor,
        (factory_ptr as i32, 0),
    )? as u32;
    if desc_ptr == 0 {
        bail!("descriptor[0] is null");
    }
    // clap_plugin_descriptor: version(12) then id ptr at +12.
    let desc_id_ptr = read_u32(&store, memory, desc_ptr + 12)?;
    let plugin_id = read_cstr(&store, memory, desc_id_ptr)?;
    println!("plugin id: \"{plugin_id}\"");

    // ── build a clap_host_t in wasm memory ────────────────────────────
    let host_ptr = alloc(&mut store, &malloc, 48)?;
    write_zero(&mut store, memory, host_ptr, 48)?;
    write_u32_mut(&mut store, memory, host_ptr, 1)?; // clap_version.major
    write_u32_mut(&mut store, memory, host_ptr + 4, entry_minor)?;
    write_u32_mut(&mut store, memory, host_ptr + 8, entry_rev)?;
    // host_data = null (offset 12) — already zeroed
    let s_name = alloc_cstr(&mut store, memory, &malloc, "tiny_host")?;
    let s_vendor = alloc_cstr(&mut store, memory, &malloc, "plinken")?;
    let s_url = alloc_cstr(&mut store, memory, &malloc, "https://plinken.org")?;
    let s_ver = alloc_cstr(&mut store, memory, &malloc, "0.0.1")?;
    write_u32_mut(&mut store, memory, host_ptr + 16, s_name)?;
    write_u32_mut(&mut store, memory, host_ptr + 20, s_vendor)?;
    write_u32_mut(&mut store, memory, host_ptr + 24, s_url)?;
    write_u32_mut(&mut store, memory, host_ptr + 28, s_ver)?;
    write_u32_mut(&mut store, memory, host_ptr + 32, idx_get_ext)?;
    write_u32_mut(&mut store, memory, host_ptr + 36, idx_req_restart)?;
    write_u32_mut(&mut store, memory, host_ptr + 40, idx_req_process)?;
    write_u32_mut(&mut store, memory, host_ptr + 44, idx_req_callback)?;

    // ── factory.create_plugin(factory, host, plugin_id) ───────────────
    let plugin_id_ptr = alloc_cstr(&mut store, memory, &malloc, &plugin_id)?;
    let plugin_ptr = call::<(i32, i32, i32), i32>(
        &mut store,
        table,
        fac_create,
        (factory_ptr as i32, host_ptr as i32, plugin_id_ptr as i32),
    )? as u32;
    if plugin_ptr == 0 {
        bail!("create_plugin returned null");
    }
    println!("plugin_ptr = 0x{plugin_ptr:x}");

    // clap_plugin layout (12 ptrs, 4 bytes each = 48 bytes):
    //   0:desc 4:plugin_data 8:init 12:destroy 16:activate 20:deactivate
    //   24:start_processing 28:stop_processing 32:reset 36:process
    //   40:get_extension 44:on_main_thread
    let pl_init = read_u32(&store, memory, plugin_ptr + 8)?;
    let pl_destroy = read_u32(&store, memory, plugin_ptr + 12)?;
    let pl_activate = read_u32(&store, memory, plugin_ptr + 16)?;
    let pl_deactivate = read_u32(&store, memory, plugin_ptr + 20)?;
    let pl_start = read_u32(&store, memory, plugin_ptr + 24)?;
    let pl_stop = read_u32(&store, memory, plugin_ptr + 28)?;
    let pl_process = read_u32(&store, memory, plugin_ptr + 36)?;

    if call::<i32, i32>(&mut store, table, pl_init, plugin_ptr as i32)? == 0 {
        bail!("plugin.init returned false");
    }
    if call::<(i32, f64, i32, i32), i32>(
        &mut store,
        table,
        pl_activate,
        (plugin_ptr as i32, SAMPLE_RATE as f64, BLOCK as i32, BLOCK as i32),
    )? == 0
    {
        bail!("plugin.activate returned false");
    }
    if call::<i32, i32>(&mut store, table, pl_start, plugin_ptr as i32)? == 0 {
        bail!("plugin.start_processing returned false");
    }
    println!("activated @ {SAMPLE_RATE} Hz, block={BLOCK}");

    // ── audio buffers + clap_process struct in wasm memory ────────────
    let block_bytes = BLOCK * 4;
    let in_l = alloc(&mut store, &malloc, block_bytes)?;
    let in_r = alloc(&mut store, &malloc, block_bytes)?;
    let out_l = alloc(&mut store, &malloc, block_bytes)?;
    let out_r = alloc(&mut store, &malloc, block_bytes)?;

    let in_data32 = alloc(&mut store, &malloc, 8)?;
    let out_data32 = alloc(&mut store, &malloc, 8)?;
    write_u32_mut(&mut store, memory, in_data32, in_l)?;
    write_u32_mut(&mut store, memory, in_data32 + 4, in_r)?;
    write_u32_mut(&mut store, memory, out_data32, out_l)?;
    write_u32_mut(&mut store, memory, out_data32 + 4, out_r)?;

    // clap_audio_buffer_t (24 bytes): data32, data64, ch_count, latency, constant_mask(u64)
    let in_buf = alloc(&mut store, &malloc, 24)?;
    let out_buf = alloc(&mut store, &malloc, 24)?;
    write_zero(&mut store, memory, in_buf, 24)?;
    write_zero(&mut store, memory, out_buf, 24)?;
    write_u32_mut(&mut store, memory, in_buf, in_data32)?;
    write_u32_mut(&mut store, memory, in_buf + 8, 2)?; // channel_count = 2
    write_u32_mut(&mut store, memory, out_buf, out_data32)?;
    write_u32_mut(&mut store, memory, out_buf + 8, 2)?;

    // clap_input_events (12): ctx, size_fn, get_fn
    let in_events = alloc(&mut store, &malloc, 12)?;
    write_u32_mut(&mut store, memory, in_events, 0)?;
    write_u32_mut(&mut store, memory, in_events + 4, idx_ev_size)?;
    write_u32_mut(&mut store, memory, in_events + 8, idx_ev_get)?;
    // clap_output_events (8): ctx, try_push_fn
    let out_events = alloc(&mut store, &malloc, 8)?;
    write_u32_mut(&mut store, memory, out_events, 0)?;
    write_u32_mut(&mut store, memory, out_events + 4, idx_ev_try_push)?;

    // clap_process_t (40 bytes):
    //   0: i64 steady_time
    //   8: u32 frames_count
    //   12: ptr transport
    //   16: ptr audio_inputs
    //   20: ptr audio_outputs
    //   24: u32 audio_inputs_count
    //   28: u32 audio_outputs_count
    //   32: ptr in_events
    //   36: ptr out_events
    let proc_ptr = alloc(&mut store, &malloc, 40)?;
    write_zero(&mut store, memory, proc_ptr, 40)?;
    write_u32_mut(&mut store, memory, proc_ptr + 8, BLOCK)?;
    write_u32_mut(&mut store, memory, proc_ptr + 16, in_buf)?;
    write_u32_mut(&mut store, memory, proc_ptr + 20, out_buf)?;
    write_u32_mut(&mut store, memory, proc_ptr + 24, 1)?;
    write_u32_mut(&mut store, memory, proc_ptr + 28, 1)?;
    write_u32_mut(&mut store, memory, proc_ptr + 32, in_events)?;
    write_u32_mut(&mut store, memory, proc_ptr + 36, out_events)?;

    // ── process loop ──────────────────────────────────────────────────
    let total_frames = SAMPLE_RATE * SECONDS;
    let blocks = total_frames / BLOCK;
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(&out_wav, spec)
        .with_context(|| format!("creating {}", out_wav.display()))?;

    let dphase = TAU * FREQ / SAMPLE_RATE as f32;
    let mut phase = 0f32;
    let mut steady: i64 = 0;
    let mut in_bytes = vec![0u8; (block_bytes) as usize];
    let mut out_bytes = vec![0u8; (block_bytes) as usize];

    for _ in 0..blocks {
        // synthesize a stereo sine, write into both input channels
        for i in 0..BLOCK as usize {
            let s = phase.sin() * 0.5;
            in_bytes[i * 4..i * 4 + 4].copy_from_slice(&s.to_le_bytes());
            phase += dphase;
            if phase > TAU {
                phase -= TAU;
            }
        }
        memory.write(&mut store, in_l as usize, &in_bytes)?;
        memory.write(&mut store, in_r as usize, &in_bytes)?;
        // zero outputs so we can tell if the plugin actually wrote
        memory.write(&mut store, out_l as usize, &vec![0u8; block_bytes as usize])?;
        memory.write(&mut store, out_r as usize, &vec![0u8; block_bytes as usize])?;

        memory.write(&mut store, proc_ptr as usize, &steady.to_le_bytes())?;

        let status = call::<(i32, i32), i32>(
            &mut store,
            table,
            pl_process,
            (plugin_ptr as i32, proc_ptr as i32),
        )?;
        if status == 0 {
            bail!("plugin.process returned CLAP_PROCESS_ERROR");
        }

        memory.read(&store, out_l as usize, &mut out_bytes)?;
        let mut left = vec![0f32; BLOCK as usize];
        for i in 0..BLOCK as usize {
            left[i] = f32::from_le_bytes(out_bytes[i * 4..i * 4 + 4].try_into().unwrap());
        }
        memory.read(&store, out_r as usize, &mut out_bytes)?;
        for i in 0..BLOCK as usize {
            let r = f32::from_le_bytes(out_bytes[i * 4..i * 4 + 4].try_into().unwrap());
            writer.write_sample(left[i])?;
            writer.write_sample(r)?;
        }

        steady += BLOCK as i64;
    }
    writer.finalize()?;
    println!("wrote {} ({} frames)", out_wav.display(), total_frames);

    // ── teardown ──────────────────────────────────────────────────────
    call::<i32, ()>(&mut store, table, pl_stop, plugin_ptr as i32)?;
    call::<i32, ()>(&mut store, table, pl_deactivate, plugin_ptr as i32)?;
    call::<i32, ()>(&mut store, table, pl_destroy, plugin_ptr as i32)?;
    call::<(), ()>(&mut store, table, entry_deinit, ())?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────
// helpers
// ─────────────────────────────────────────────────────────────────────────

fn alloc(store: &mut Store<()>, malloc: &TypedFunc<u32, u32>, n: u32) -> Result<u32> {
    let p = malloc.call(&mut *store, n)?;
    if p == 0 {
        bail!("malloc({n}) returned 0");
    }
    Ok(p)
}

fn alloc_cstr(
    store: &mut Store<()>,
    memory: Memory,
    malloc: &TypedFunc<u32, u32>,
    s: &str,
) -> Result<u32> {
    let n = (s.len() + 1) as u32;
    let p = alloc(store, malloc, n)?;
    memory.write(&mut *store, p as usize, s.as_bytes())?;
    memory.write(&mut *store, (p as usize) + s.len(), &[0u8])?;
    Ok(p)
}

fn read_u32(store: impl AsContext, memory: Memory, ptr: u32) -> Result<u32> {
    let mut buf = [0u8; 4];
    memory.read(store, ptr as usize, &mut buf)?;
    Ok(u32::from_le_bytes(buf))
}

fn write_u32_mut(store: &mut Store<()>, memory: Memory, ptr: u32, val: u32) -> Result<()> {
    memory.write(&mut *store, ptr as usize, &val.to_le_bytes())?;
    Ok(())
}

fn write_zero(store: &mut Store<()>, memory: Memory, ptr: u32, len: u32) -> Result<()> {
    let zeros = vec![0u8; len as usize];
    memory.write(&mut *store, ptr as usize, &zeros)?;
    Ok(())
}

fn read_cstr(store: impl AsContext, memory: Memory, ptr: u32) -> Result<String> {
    // Read 256 bytes at a time and stop at NUL. Plenty for descriptor strings.
    let mut out = Vec::new();
    let mut chunk = [0u8; 64];
    let mut off = ptr as usize;
    let store = store.as_context();
    loop {
        memory.read(&store, off, &mut chunk)?;
        for &b in &chunk {
            if b == 0 {
                return Ok(String::from_utf8_lossy(&out).into_owned());
            }
            out.push(b);
            if out.len() > 4096 {
                bail!("read_cstr: no NUL within 4096 bytes");
            }
        }
        off += chunk.len();
    }
}

/// Indirect call through `__indirect_function_table` at `idx`,
/// typed as `Params -> Results`. Errors if the slot is null or has a
/// different signature than declared.
fn call<P, R>(
    store: &mut Store<()>,
    table: Table,
    idx: u32,
    params: P,
) -> Result<R>
where
    P: wasmtime::WasmParams,
    R: wasmtime::WasmResults,
{
    let r = table
        .get(&mut *store, idx)
        .with_context(|| format!("table index {idx} out of bounds"))?;
    let f = match r {
        Ref::Func(Some(f)) => f,
        Ref::Func(None) => bail!("table[{idx}] is a null funcref"),
        _ => bail!("table[{idx}] is not a funcref"),
    };
    let tf: TypedFunc<P, R> = f
        .typed(store.as_context())
        .with_context(|| format!("table[{idx}] signature mismatch"))?;
    Ok(tf.call(&mut *store, params)?)
}
