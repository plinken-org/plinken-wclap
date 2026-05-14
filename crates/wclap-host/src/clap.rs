#![allow(dead_code)] // constants land in use at step 4b (`createPlugin`).

//! Field offsets and struct sizes for the CLAP-as-WCLAP ABI as the plugin's
//! wasm module lays them out.
//!
//! Source of truth: `vendor/wclap-host-js/cpp/wclap-cpp/include/wclap/_impl/wclap-generic.hpp`
//! (auto-generated CLAP header where `Pointer<T>` and `Function<R, Args...>`
//! are 4-byte WASM-pointer/function-index values).
//!
//! Everything in the plugin's address space — including the `clap_host_t` we
//! build for the plugin in step 4 — uses these offsets.

/// `wclap_version` — three little-endian u32s (major, minor, revision).
pub const VERSION_SIZE: usize = 12;

pub mod entry {
    //! `wclap_plugin_entry` — handed back by `_wclapInstance.init32`.
    pub const VERSION: usize = 0;
    pub const INIT: usize = 12; // Function<bool, Pointer<const char>>
    pub const DEINIT: usize = 16; // Function<void>
    pub const GET_FACTORY: usize = 20; // Function<Pointer<const void>, Pointer<const char>>
    pub const SIZE: usize = 24;
}

pub mod descriptor {
    //! `wclap_plugin_descriptor` — what the factory hands out per slot index.
    pub const VERSION: usize = 0;
    pub const ID: usize = 12;
    pub const NAME: usize = 16;
    pub const VENDOR: usize = 20;
    pub const URL: usize = 24;
    pub const MANUAL_URL: usize = 28;
    pub const SUPPORT_URL: usize = 32;
    pub const VERSION_STR: usize = 36;
    pub const DESCRIPTION: usize = 40;
    pub const FEATURES: usize = 44; // Pointer<Pointer<const char> const>
    pub const SIZE: usize = 48;
}

pub mod plugin {
    //! `wclap_plugin` — the per-plugin instance struct the factory returns.
    pub const DESC: usize = 0; // Pointer<const wclap_plugin_descriptor>
    pub const PLUGIN_DATA: usize = 4; // Pointer<void>
    pub const INIT: usize = 8;
    pub const DESTROY: usize = 12;
    pub const ACTIVATE: usize = 16; // Function<bool, plugin*, double sr, u32 min, u32 max>
    pub const DEACTIVATE: usize = 20;
    pub const START_PROCESSING: usize = 24;
    pub const STOP_PROCESSING: usize = 28;
    pub const RESET: usize = 32;
    pub const PROCESS: usize = 36;
    pub const GET_EXTENSION: usize = 40;
    pub const ON_MAIN_THREAD: usize = 44;
    pub const SIZE: usize = 48;
}

pub mod factory {
    //! `wclap_plugin_factory` — what `clap_entry.get_factory("clap.plugin-factory")` returns.
    pub const GET_PLUGIN_COUNT: usize = 0;
    pub const GET_PLUGIN_DESCRIPTOR: usize = 4;
    pub const CREATE_PLUGIN: usize = 8;
    pub const SIZE: usize = 12;
}

pub mod host {
    //! `wclap_host` — we build one of these in the plugin's memory at step 4
    //! and pass it to `create_plugin`. Function fields are wasm-table indices
    //! that `_wclapInstance.registerHost32` returned, pointing at host stubs.
    pub const VERSION: usize = 0;
    pub const HOST_DATA: usize = 12;
    pub const NAME: usize = 16;
    pub const VENDOR: usize = 20;
    pub const URL: usize = 24;
    pub const VERSION_STR: usize = 28;
    pub const GET_EXTENSION: usize = 32; // Function<ptr, host*, char*>
    pub const REQUEST_RESTART: usize = 36;
    pub const REQUEST_PROCESS: usize = 40;
    pub const REQUEST_CALLBACK: usize = 44;
    pub const SIZE: usize = 48;
}

pub const FACTORY_ID: &[u8] = b"clap.plugin-factory\0";

pub const EXT_AUDIO_PORTS: &[u8] = b"clap.audio-ports\0";
pub const EXT_WEBVIEW: &[u8] = b"clap.webview/3\0";

pub mod webview {
    //! `wclap_plugin_webview` — draft `clap.webview/3` extension.
    //! `get_uri` follows a two-call probe: first call with `cap=0` returns
    //! the required byte length; second call with a sized buffer writes the
    //! UTF-8 URI (NUL-terminated within the buffer).
    pub const GET_URI: usize = 0; // Function<i32, plugin*, char* buf, u32 cap>
    pub const GET_RESOURCE: usize = 4; // Function<bool, plugin*, char* path, char* mime_buf, u32 mime_cap, ostream*>
    pub const RECEIVE: usize = 8; // Function<bool, plugin*, void* buf, u32 size>
    pub const SIZE: usize = 12;
}

pub mod host_webview {
    //! `wclap_host_webview` — host side of `clap.webview/3`. Single
    //! function pointer; plugin calls it to push bytes back to the iframe.
    pub const SEND: usize = 0; // Function<bool, host*, void* buf, u32 size>
    pub const SIZE: usize = 4;
}

pub mod audio_ports {
    //! `wclap_plugin_audio_ports` — extension returned by
    //! `plugin.get_extension(plugin, "clap.audio-ports")`.
    pub const COUNT: usize = 0; // Function<u32, plugin*, bool is_input>
    pub const GET: usize = 4; // Function<bool, plugin*, u32 index, bool is_input, audio_port_info*>
    pub const SIZE: usize = 8;
}

pub mod audio_port_info {
    //! `wclap_audio_port_info` — filled by `audio_ports.get`.
    pub const ID: usize = 0;
    pub const NAME: usize = 4; // char[256]
    pub const FLAGS: usize = 260;
    pub const CHANNEL_COUNT: usize = 264;
    pub const PORT_TYPE: usize = 268; // Pointer<const char>
    pub const IN_PLACE_PAIR: usize = 272;
    pub const SIZE: usize = 276;
}

pub mod audio_buffer {
    //! `wclap_audio_buffer` — one per port. M1 host writes only `data32`
    //! and `channel_count`; `data64` is 0 (we host wasm32 plugins only),
    //! `latency` is 0, `constant_mask` is 0.
    pub const DATA32: usize = 0; // Pointer<Pointer<f32>>
    pub const DATA64: usize = 4; // Pointer<Pointer<f64>>
    pub const CHANNEL_COUNT: usize = 8;
    pub const LATENCY: usize = 12;
    pub const CONSTANT_MASK: usize = 16; // u64
    pub const SIZE: usize = 24;
}

pub mod process {
    //! `wclap_process` — passed once into `clap_plugin.process`. M1 host
    //! reuses the same struct every block, mutating only `frames_count`.
    //! Events come in/out via empty `clap_input_events` / `clap_output_events`
    //! stubs that are no-ops at M1.
    pub const STEADY_TIME: usize = 0; // i64
    pub const FRAMES_COUNT: usize = 8;
    pub const TRANSPORT: usize = 12; // Pointer<const event_transport>
    pub const AUDIO_INPUTS: usize = 16;
    pub const AUDIO_OUTPUTS: usize = 20;
    pub const AUDIO_INPUTS_COUNT: usize = 24;
    pub const AUDIO_OUTPUTS_COUNT: usize = 28;
    pub const IN_EVENTS: usize = 32;
    pub const OUT_EVENTS: usize = 36;
    pub const SIZE: usize = 40;
}

pub mod input_events {
    //! `wclap_input_events` — host-supplied event source the plugin polls
    //! during `process`. M1 emits an empty stream: `size` returns 0,
    //! `get` is never called.
    pub const CTX: usize = 0; // Pointer<void>
    pub const SIZE_FN: usize = 4; // Function<u32, list*>
    pub const GET: usize = 8; // Function<event*, list*, u32 index>
    pub const SIZE: usize = 12;
}

pub mod output_events {
    //! `wclap_output_events` — host-supplied sink. M1 drops everything
    //! the plugin tries to push (`try_push` returns false).
    pub const CTX: usize = 0;
    pub const TRY_PUSH: usize = 4; // Function<bool, list*, event*>
    pub const SIZE: usize = 8;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struct_sizes_match_field_layout() {
        assert_eq!(entry::SIZE, 24);
        assert_eq!(descriptor::SIZE, 48);
        assert_eq!(plugin::SIZE, 48);
        assert_eq!(factory::SIZE, 12);
        assert_eq!(host::SIZE, 48);
        assert_eq!(VERSION_SIZE, 12);
        assert_eq!(audio_ports::SIZE, 8);
        assert_eq!(audio_port_info::SIZE, 276);
        assert_eq!(audio_buffer::SIZE, 24);
        assert_eq!(process::SIZE, 40);
        assert_eq!(input_events::SIZE, 12);
        assert_eq!(output_events::SIZE, 8);
        assert_eq!(webview::SIZE, 12);
        assert_eq!(host_webview::SIZE, 4);
    }

    #[test]
    fn factory_id_is_nul_terminated() {
        assert_eq!(FACTORY_ID.last(), Some(&0));
        assert_eq!(&FACTORY_ID[..FACTORY_ID.len() - 1], b"clap.plugin-factory");
    }
}
