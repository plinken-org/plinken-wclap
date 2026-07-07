//! Plinken sample core — shared sample-playback building blocks.
//!
//! Used two ways:
//! - **Pulze** (drum machine): one [`SampleVoice`] pool, one shot per pad.
//! - **Synome** (polysynth): a per-synth-voice [`SampleVoice`] driven as an
//!   oscillator via [`SampleVoice::tick_pitched`].
//!
//! Playback math ported from the private monorepo's SFZ Sampler
//! (`plugins/Sampler/src/{voice.rs, sample_cache.rs}`); the std-bound
//! loading layer (WAV/FLAC decode, SFZ parse, download cache) is
//! deliberately absent — hosts decode audio and deliver PCM through the
//! PLSP chunk protocol parsed by [`SampleAssembler`].

pub mod assembler;
pub mod sample;
pub mod voice;

pub use assembler::{AssembleResult, SampleAssembler};
pub use sample::SampleData;
pub use voice::{pitch_ratio, LoopMode, SampleVoice, VoiceParams, VoicePool};
