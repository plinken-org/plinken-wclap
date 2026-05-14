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
    }

    #[test]
    fn factory_id_is_nul_terminated() {
        assert_eq!(FACTORY_ID.last(), Some(&0));
        assert_eq!(&FACTORY_ID[..FACTORY_ID.len() - 1], b"clap.plugin-factory");
    }
}
