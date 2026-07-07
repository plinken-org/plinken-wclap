//! PLSP sample-transfer reassembly — the plugin-side half of the chunked
//! sample-delivery contract (the app-side encoder lives in plinken-app's
//! `plugin-samples` service; both sides must match this layout exactly).
//!
//! Wire shape: one CBOR map `{"smp": <bytes>}` — sniffable from the 10-byte
//! prefix `A1 63 's' 'm' 'p' 5A <u32 BE payload len>` — whose byte-string
//! payload is a 32-byte little-endian header followed by planar f32le PCM:
//!
//! | off | field             |                                              |
//! |-----|-------------------|----------------------------------------------|
//! | 0   | magic u32         | 0x504C5350 "PLSP"                            |
//! | 4   | version u32       | 1                                            |
//! | 8   | slot u32          | Pulze pad 0–63 (bank*16+pad); Synome 0       |
//! | 12  | sample_rate u32   | rate of the PCM as sent (playback resamples) |
//! | 16  | channels u32      | 1 or 2                                       |
//! | 20  | total_frames u32  | 0 = clear the slot                           |
//! | 24  | chunk_frame_start | u32                                          |
//! | 28  | chunk_frames u32  |                                              |
//! | 32… | PCM               | chunk_frames left f32le, then right if stereo |
//!
//! `chunk_frame_start == 0` (re)allocates the slot's buffers; the transfer
//! completes when `start + frames == total_frames`. Re-sending from
//! `start == 0` resets a mid-flight slot safely (idempotent per slot).

use crate::sample::SampleData;

/// "PLSP", little-endian.
pub const PLSP_MAGIC: u32 = 0x504C_5350;
pub const PLSP_VERSION: u32 = 1;
/// CBOR prefix `{"smp": bytes(u32-BE length)}`.
const CBOR_PREFIX: [u8; 6] = [0xa1, 0x63, b's', b'm', b'p', 0x5a];
const HEADER_LEN: usize = 32;
/// Refuse absurd allocations (frames per channel). 2^26 frames of stereo
/// f32 is 512 MiB — far beyond any sane sample; wasm memory would die
/// long before. Real kit samples are a few seconds.
const MAX_FRAMES: u32 = 1 << 26;
const MAX_SLOTS: usize = 64;

/// Result of feeding one `webview.receive` payload to the assembler.
#[derive(Debug)]
pub enum AssembleResult {
    /// Not a PLSP chunk message — hand it to the next parser.
    NotMine,
    /// Chunk accepted; transfer still in flight.
    Progress { slot: u32, received_frames: u32, total_frames: u32 },
    /// Slot fully received. Install the sample.
    Complete { slot: u32, sample: SampleData },
    /// The app cleared this slot (`total_frames == 0`).
    Cleared { slot: u32 },
    /// Malformed / out-of-contract message; the slot's in-flight state
    /// (if any) was dropped.
    Error,
}

struct InFlight {
    slot: u32,
    sample_rate: u32,
    channels: u32,
    total_frames: u32,
    received_frames: u32,
    left: Vec<f32>,
    right: Vec<f32>,
}

/// Reassembles per-slot chunked sample transfers. One per plugin instance;
/// wire it in `Plugin::on_message`.
#[derive(Default)]
pub struct SampleAssembler {
    in_flight: Vec<InFlight>,
}

impl SampleAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one raw `webview.receive` payload.
    pub fn push(&mut self, bytes: &[u8]) -> AssembleResult {
        // Envelope sniff.
        if bytes.len() < CBOR_PREFIX.len() + 4 + HEADER_LEN {
            if bytes.len() >= CBOR_PREFIX.len() && bytes[..CBOR_PREFIX.len()] == CBOR_PREFIX {
                return AssembleResult::Error; // ours, but truncated
            }
            return AssembleResult::NotMine;
        }
        if bytes[..CBOR_PREFIX.len()] != CBOR_PREFIX {
            return AssembleResult::NotMine;
        }
        let declared = u32::from_be_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]) as usize;
        let payload = &bytes[10..];
        if payload.len() < declared || declared < HEADER_LEN {
            return AssembleResult::Error;
        }
        let payload = &payload[..declared];

        let u32_at = |off: usize| {
            u32::from_le_bytes([
                payload[off],
                payload[off + 1],
                payload[off + 2],
                payload[off + 3],
            ])
        };
        if u32_at(0) != PLSP_MAGIC || u32_at(4) != PLSP_VERSION {
            return AssembleResult::Error;
        }
        let slot = u32_at(8);
        let sample_rate = u32_at(12);
        let channels = u32_at(16);
        let total_frames = u32_at(20);
        let chunk_start = u32_at(24);
        let chunk_frames = u32_at(28);

        if slot as usize >= MAX_SLOTS {
            return AssembleResult::Error;
        }

        // Clear message.
        if total_frames == 0 {
            self.in_flight.retain(|f| f.slot != slot);
            return AssembleResult::Cleared { slot };
        }

        if !(1..=2).contains(&channels)
            || sample_rate == 0
            || total_frames > MAX_FRAMES
            || chunk_frames == 0
            || chunk_start.checked_add(chunk_frames).map_or(true, |end| end > total_frames)
        {
            self.drop_slot(slot);
            return AssembleResult::Error;
        }

        // PCM payload size must match the header exactly.
        let expected_pcm = (chunk_frames as usize) * (channels as usize) * 4;
        if payload.len() != HEADER_LEN + expected_pcm {
            self.drop_slot(slot);
            return AssembleResult::Error;
        }

        // start == 0 (re)initializes the slot.
        if chunk_start == 0 {
            self.drop_slot(slot);
            self.in_flight.push(InFlight {
                slot,
                sample_rate,
                channels,
                total_frames,
                received_frames: 0,
                left: vec![0.0; total_frames as usize],
                right: if channels == 2 {
                    vec![0.0; total_frames as usize]
                } else {
                    Vec::new()
                },
            });
        }

        let Some(fl) = self.in_flight.iter_mut().find(|f| f.slot == slot) else {
            // Mid-transfer chunk for a slot we never started.
            return AssembleResult::Error;
        };
        // Chunks must arrive in order and agree on the transfer shape.
        if fl.sample_rate != sample_rate
            || fl.channels != channels
            || fl.total_frames != total_frames
            || fl.received_frames != chunk_start
        {
            self.drop_slot(slot);
            return AssembleResult::Error;
        }

        let n = chunk_frames as usize;
        let start = chunk_start as usize;
        let pcm = &payload[HEADER_LEN..];
        for i in 0..n {
            let off = i * 4;
            fl.left[start + i] =
                f32::from_le_bytes([pcm[off], pcm[off + 1], pcm[off + 2], pcm[off + 3]]);
        }
        if channels == 2 {
            let roff = n * 4;
            for i in 0..n {
                let off = roff + i * 4;
                fl.right[start + i] =
                    f32::from_le_bytes([pcm[off], pcm[off + 1], pcm[off + 2], pcm[off + 3]]);
            }
        }
        fl.received_frames = chunk_start + chunk_frames;

        if fl.received_frames == fl.total_frames {
            let pos = self.in_flight.iter().position(|f| f.slot == slot).unwrap();
            let fl = self.in_flight.swap_remove(pos);
            return AssembleResult::Complete {
                slot,
                sample: SampleData {
                    sample_rate: fl.sample_rate,
                    channels: fl.channels,
                    frame_count: fl.total_frames as usize,
                    left: fl.left,
                    right: fl.right,
                },
            };
        }
        AssembleResult::Progress {
            slot,
            received_frames: fl.received_frames,
            total_frames: fl.total_frames,
        }
    }

    fn drop_slot(&mut self, slot: u32) {
        self.in_flight.retain(|f| f.slot != slot);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test-side encoder — mirrors what the app's plugin-samples service
    /// sends. Kept in tests so the wire contract is exercised end-to-end.
    fn encode_chunk(
        slot: u32,
        sample_rate: u32,
        channels: u32,
        total_frames: u32,
        chunk_start: u32,
        left: &[f32],
        right: Option<&[f32]>,
    ) -> Vec<u8> {
        let chunk_frames = left.len() as u32;
        let pcm_len = left.len() * 4 + right.map_or(0, |r| r.len() * 4);
        let payload_len = HEADER_LEN + pcm_len;
        let mut out = Vec::with_capacity(10 + payload_len);
        out.extend_from_slice(&CBOR_PREFIX);
        out.extend_from_slice(&(payload_len as u32).to_be_bytes());
        out.extend_from_slice(&PLSP_MAGIC.to_le_bytes());
        out.extend_from_slice(&PLSP_VERSION.to_le_bytes());
        out.extend_from_slice(&slot.to_le_bytes());
        out.extend_from_slice(&sample_rate.to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&total_frames.to_le_bytes());
        out.extend_from_slice(&chunk_start.to_le_bytes());
        out.extend_from_slice(&chunk_frames.to_le_bytes());
        for v in left {
            out.extend_from_slice(&v.to_le_bytes());
        }
        if let Some(r) = right {
            for v in r {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        out
    }

    fn encode_clear(slot: u32) -> Vec<u8> {
        let mut out = Vec::with_capacity(10 + HEADER_LEN);
        out.extend_from_slice(&CBOR_PREFIX);
        out.extend_from_slice(&(HEADER_LEN as u32).to_be_bytes());
        out.extend_from_slice(&PLSP_MAGIC.to_le_bytes());
        out.extend_from_slice(&PLSP_VERSION.to_le_bytes());
        out.extend_from_slice(&slot.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // sample_rate
        out.extend_from_slice(&0u32.to_le_bytes()); // channels
        out.extend_from_slice(&0u32.to_le_bytes()); // total_frames = 0
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out
    }

    #[test]
    fn round_trip_stereo_in_chunks() {
        let mut asm = SampleAssembler::new();
        let total = 10u32;
        let left: Vec<f32> = (0..total).map(|i| i as f32).collect();
        let right: Vec<f32> = (0..total).map(|i| -(i as f32)).collect();

        let r1 = asm.push(&encode_chunk(3, 44100, 2, total, 0, &left[..4], Some(&right[..4])));
        assert!(matches!(r1, AssembleResult::Progress { received_frames: 4, .. }));
        let r2 = asm.push(&encode_chunk(3, 44100, 2, total, 4, &left[4..], Some(&right[4..])));
        match r2 {
            AssembleResult::Complete { slot, sample } => {
                assert_eq!(slot, 3);
                assert_eq!(sample.sample_rate, 44100);
                assert_eq!(sample.channels, 2);
                assert_eq!(sample.frame_count, 10);
                assert_eq!(sample.left, left);
                assert_eq!(sample.right, right);
            }
            other => panic!("expected Complete, got {other:?}"),
        }
    }

    #[test]
    fn mono_single_chunk_and_clear() {
        let mut asm = SampleAssembler::new();
        let pcm: Vec<f32> = vec![0.5; 8];
        match asm.push(&encode_chunk(0, 48000, 1, 8, 0, &pcm, None)) {
            AssembleResult::Complete { slot: 0, sample } => {
                assert_eq!(sample.left, pcm);
                assert!(sample.right.is_empty());
            }
            other => panic!("expected Complete, got {other:?}"),
        }
        assert!(matches!(asm.push(&encode_clear(0)), AssembleResult::Cleared { slot: 0 }));
    }

    #[test]
    fn restart_mid_transfer_resets_slot() {
        let mut asm = SampleAssembler::new();
        let l: Vec<f32> = (0..6).map(|i| i as f32).collect();
        asm.push(&encode_chunk(1, 48000, 1, 6, 0, &l[..3], None));
        // New transfer from 0 replaces the old in-flight state.
        asm.push(&encode_chunk(1, 48000, 1, 6, 0, &l[..3], None));
        match asm.push(&encode_chunk(1, 48000, 1, 6, 3, &l[3..], None)) {
            AssembleResult::Complete { sample, .. } => assert_eq!(sample.left, l),
            other => panic!("expected Complete, got {other:?}"),
        }
    }

    #[test]
    fn out_of_order_chunk_is_an_error() {
        let mut asm = SampleAssembler::new();
        let l = vec![1.0f32; 4];
        asm.push(&encode_chunk(2, 48000, 1, 8, 0, &l, None));
        // Skip ahead: start 6 != received 4.
        assert!(matches!(
            asm.push(&encode_chunk(2, 48000, 1, 8, 6, &l[..2], None)),
            AssembleResult::Error
        ));
        // Slot state dropped: continuing the old transfer now errors too.
        assert!(matches!(
            asm.push(&encode_chunk(2, 48000, 1, 8, 4, &l, None)),
            AssembleResult::Error
        ));
    }

    #[test]
    fn foreign_messages_are_not_mine() {
        let mut asm = SampleAssembler::new();
        assert!(matches!(asm.push(b"ready"), AssembleResult::NotMine));
        assert!(matches!(asm.push(&[0xa1, 0x63, b's', b'e', b't']), AssembleResult::NotMine));
        assert!(matches!(asm.push(b"{\"json\":true}"), AssembleResult::NotMine));
    }

    #[test]
    fn size_mismatch_is_error() {
        let mut asm = SampleAssembler::new();
        let mut msg = encode_chunk(1, 48000, 1, 8, 0, &[0.0; 4], None);
        msg.truncate(msg.len() - 4); // lop off PCM
        assert!(matches!(asm.push(&msg), AssembleResult::Error));
    }
}
