#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;

#[cfg(target_arch = "wasm32")]
#[global_allocator]
static ALLOC: dlmalloc::GlobalDlmalloc = dlmalloc::GlobalDlmalloc;

#[cfg(target_arch = "wasm32")]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

#[cfg(target_arch = "wasm32")]
mod instance;

mod bytes;
mod call;
mod clap;
#[cfg(target_arch = "wasm32")]
mod host_stubs;
mod host;
mod plugin;
mod wclap_instance;

// The wclap-host-js loader calls `_initialize()` when the host doesn't import
// its memory (see `wclap.mjs` startHost: `if (needsInit) this.hostInstance.exports._initialize()`).
// Rust's cdylib for wasm32-unknown-unknown doesn't auto-export one (reactor
// behaviour is wasi-only), so we provide a no-op. Drop this if we ever switch
// to imported memory.
#[no_mangle]
pub extern "C" fn _initialize() {}
