//! WAL record types and frame (de)serialization.
//!
//! Frame layout: `[u32 LE len][u32 LE crc32(len_bytes ++ payload)][payload = serde_json(WalRecord)]`
//! CRC + length give torn-write and bit-rot detection independent of JSON, and
//! detect interior corruption, not just the tail. [FIX-INTERIOR-CORRUPT][FIX-TORN]
//!
//! IMPORTANT: the CRC covers the 4-byte length prefix as well as the payload
//! ([FIX-LEN-CRC]). If it covered only the payload, a torn write or bit-rot in
//! an *interior* frame's length prefix would be indistinguishable from a torn
//! tail: a corrupted length either points past EOF (read_frame → Incomplete) or
//! is contrived to land at EOF (read_frame → BadCrc over the wrong window), and
//! in both cases recovery would silently truncate every committed record after
//! it. By folding the length bytes into the CRC, any single-bit flip in the
//! length prefix is detected as a CRC mismatch and classified by the same
//! interior-vs-tail forward scan as any other corruption.

use serde::{Deserialize, Serialize};

use crate::crc::crc32;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum FailedOutcomeRec {
    Retry { not_before_ms: u64 },
    Dead,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum WalRecord {
    Enqueued {
        id: u64,
        payload: Vec<u8>,
        max_attempts: u32,
        not_before_ms: u64,
        idempotency_key: Option<String>,
    },
    Claimed {
        id: u64,
        epoch: u64,
        wall_deadline_ms: u64,
        deliveries: u32,
    },
    Reclaimed {
        id: u64,
        epoch: u64,
    },
    Completed {
        id: u64,
    },
    Failed {
        id: u64,
        attempts: u32,
        outcome: FailedOutcomeRec,
    },
    Snapshotted {
        up_to_offset: u64,
    },
}

pub const HEADER_LEN: usize = 8; // 4 bytes len + 4 bytes crc

/// CRC32 over the 4-byte length prefix followed by the payload. Keeping this in
/// one place guarantees `encode_frame` and `read_frame` agree on what bytes the
/// CRC protects. [FIX-LEN-CRC]
fn crc32_header_payload(len_bytes: &[u8; 4], payload: &[u8]) -> u32 {
    let mut buf = Vec::with_capacity(4 + payload.len());
    buf.extend_from_slice(len_bytes);
    buf.extend_from_slice(payload);
    crc32(&buf)
}

/// Serialize a record into a complete contiguous frame buffer.
pub fn encode_frame(rec: &WalRecord) -> Result<Vec<u8>, serde_json::Error> {
    let payload = serde_json::to_vec(rec)?;
    let len = payload.len() as u32;
    let len_bytes = len.to_le_bytes();
    // [FIX-LEN-CRC]: CRC covers the length prefix AND the payload so that any
    // corruption of the length field is detected as a CRC mismatch rather than
    // being silently trusted by recovery's torn-vs-interior decision.
    let crc = crc32_header_payload(&len_bytes, &payload);
    let mut buf = Vec::with_capacity(HEADER_LEN + payload.len());
    buf.extend_from_slice(&len_bytes);
    buf.extend_from_slice(&crc.to_le_bytes());
    buf.extend_from_slice(&payload);
    Ok(buf)
}

/// Result of attempting to decode one frame from a byte slice starting at `offset`.
pub enum FrameRead {
    /// A valid record and the number of bytes consumed.
    Ok { rec: WalRecord, consumed: usize },
    /// The frame is incomplete (short read) — treat as torn tail if it's the
    /// last frame in the file.
    Incomplete,
    /// The CRC did not match (corruption). If this is the last frame it's a torn
    /// tail; if interior, recovery fails loud.
    BadCrc,
}

/// Try to read one frame from `data` starting at byte 0.
pub fn read_frame(data: &[u8]) -> FrameRead {
    if data.len() < HEADER_LEN {
        return FrameRead::Incomplete;
    }
    let len_bytes = [data[0], data[1], data[2], data[3]];
    let len = u32::from_le_bytes(len_bytes) as usize;
    let crc_expected = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    let frame_end = HEADER_LEN + len;
    if data.len() < frame_end {
        return FrameRead::Incomplete;
    }
    let payload = &data[HEADER_LEN..frame_end];
    // [FIX-LEN-CRC]: verify the CRC over the length prefix AND the payload.
    let crc_actual = crc32_header_payload(&len_bytes, payload);
    if crc_actual != crc_expected {
        return FrameRead::BadCrc;
    }
    match serde_json::from_slice::<WalRecord>(payload) {
        Ok(rec) => FrameRead::Ok {
            rec,
            consumed: frame_end,
        },
        // CRC matched but JSON failed: structurally this is corruption too.
        Err(_) => FrameRead::BadCrc,
    }
}
