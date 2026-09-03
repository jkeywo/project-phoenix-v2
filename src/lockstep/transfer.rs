//! Framing, chunking and integrity for the portable authoritative record
//! (issue #1117).
//!
//! Pure and Bevy-free. This module never captures or restores anything and never
//! decides what a snapshot *means* — it only turns the one canonical record
//! [`crate::snapshot`] already produces (the RON text a `LocalStorage` slot or a
//! `phoenix-save.ron` export holds, byte for byte) into a sequence of bounded
//! frames a host mesh can carry, and turns a received sequence back into that
//! same text or a distinct refusal.
//!
//! # Why this exists at all
//!
//! `p2p-delta-snapshot-is-whole-payload-ron` settles the shape: the delivered
//! record is one whole-payload RON string with a `Versions` gate, and #854 (now
//! this issue) "owns the transport half only: chunking, reassembly and integrity
//! checking of an existing whole-payload record". The mesh channel carries
//! bounded frames — a browser DataChannel refuses a message over the
//! SDP-negotiated `max-message-size` — so a snapshot large enough to matter has
//! to travel in pieces, and a receiver reassembling those pieces has to be able
//! to say "a piece is missing", "a piece is damaged" and "a sender is trying to
//! make me buffer forever" as three different answers.
//!
//! # What "no second serializer" means here
//!
//! Nothing in this module reads or writes a single field of simulation state.
//! The bytes it chunks are produced by [`crate::snapshot::export_artifact`] and
//! consumed by [`crate::snapshot::import_artifact`], verbatim — the same two
//! calls a local file export and import already use. This module frames the
//! string those calls hand it and nothing more, which is why the acceptance
//! criterion "no second capture/restore walk or simulation serializer is
//! introduced" holds by construction rather than by review: there is no state
//! walk in here to be a second one.
//!
//! # Bounded reassembly
//!
//! A receiver refuses, up front, a transfer that declares more than
//! [`SNAPSHOT_MAX_CHUNKS`] chunks, and refuses any single chunk whose text
//! exceeds [`SNAPSHOT_CHUNK_BYTES`]. Together those cap what one transfer can ever
//! make a receiver hold at [`SNAPSHOT_MAX_TRANSFER_BYTES`], so a broken or
//! hostile sender cannot grow the buffer without bound by promising a transfer it
//! never completes or by sending oversized pieces. A chunk for a *different*
//! transfer than the one in progress is refused rather than blindly adopted, so a
//! second sender cannot displace the first either.
//!
//! That [`SNAPSHOT_MAX_TRANSFER_BYTES`] ceiling is per-transfer and belongs to
//! this pure [`SnapshotReceiver`], which holds exactly one transfer's chunks and
//! nothing more. The Bevy adapter that wraps it,
//! [`crate::lockstep::snapshot_relay::MeshSnapshotReceiver`], can transiently hold
//! more — a completed record staged for restore PLUS a fresh transfer already
//! arriving — so its peak is up to ~2x this bound; that is documented on the
//! adapter, not here.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::command_admission::log::HostSlot;

/// The largest a single chunk's text may be, in UTF-8 bytes.
///
/// [ai] 32 KiB, chosen against the transport rather than by taste. The host mesh
/// rides the same DataChannel the crew snapshot class does, whose SDP-negotiated
/// `max-message-size` is 262144 bytes between two Chromiums and whose conservative
/// framing bound (`gui/rendezvous-relay.js`, `src/core/rendezvous.rs`) is 65536.
/// A chunk is JSON-escaped into the `{ m, t, tick, d }` envelope before it is a
/// message, and RON text is very nearly all ASCII, so 32 KiB of chunk text leaves
/// comfortable headroom under the 65536-byte frame bound for the envelope and any
/// escaping. Larger chunks mean fewer frames but risk a rejected message; this is
/// the knob, and it is documented rather than inlined so a transport change moves
/// one number.
pub const SNAPSHOT_CHUNK_BYTES: usize = 32 * 1024;

/// The largest whole payload a receiver will reassemble, in bytes.
///
/// [ai] 64 MiB — a generous ceiling, not a target. A real capture of the flagship
/// scenario is single-digit megabytes (dozens of chunks); this exists only so a
/// sender that declares an enormous `total`, or that never stops sending, meets a
/// hard wall instead of an unbounded buffer. [`SNAPSHOT_MAX_CHUNKS`] is derived
/// from it so the two cannot drift.
pub const SNAPSHOT_MAX_TRANSFER_BYTES: usize = 64 * 1024 * 1024;

/// The largest number of chunks a transfer may declare.
///
/// Derived from [`SNAPSHOT_MAX_TRANSFER_BYTES`] and [`SNAPSHOT_CHUNK_BYTES`] so a
/// transfer that fits the byte ceiling fits the chunk ceiling and vice versa. A
/// `total` above this is refused before a single chunk is buffered.
pub const SNAPSHOT_MAX_CHUNKS: u32 = (SNAPSHOT_MAX_TRANSFER_BYTES / SNAPSHOT_CHUNK_BYTES) as u32;

/// One framed piece of a portable snapshot, as it crosses the host mesh.
///
/// Every chunk of one transfer repeats the whole-transfer facts — `transfer_id`,
/// `total` and `whole_hash` — so a receiver can validate any chunk against the
/// transfer in progress without having seen chunk zero first, and reject one that
/// belongs to a different transfer. `crc` is this chunk's own checksum, so a
/// damaged piece is caught the instant it arrives rather than only when the whole
/// payload fails to hash at the end.
///
/// The payload is `text` — a slice of the RON export — rather than raw bytes,
/// because the record is UTF-8 text and carrying it as text keeps the JSON and
/// RON encodings trivial (a string field, no base64) and keeps a chunk split on a
/// UTF-8 character boundary, never through the middle of a codepoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotChunk {
    /// The fleet slot sending this transfer, mirroring every other mesh frame's
    /// `from`.
    pub from: HostSlot,
    /// Identifies one capture. A re-capture is a new transfer with a new id, so a
    /// stale chunk from an abandoned transfer cannot be mistaken for a live one.
    pub transfer_id: u64,
    /// The `SimTick` this snapshot was captured at. Diagnostic, and the tick the
    /// envelope stamps.
    pub tick: u64,
    /// This chunk's 0-based index within the transfer.
    pub seq: u32,
    /// How many chunks the whole transfer has.
    pub total: u32,
    /// `fnv1a` over the WHOLE reassembled text. The integrity of the join: a
    /// receiver that reassembles to a different hash refuses rather than restores.
    pub whole_hash: u64,
    /// `crc32_ieee` over THIS chunk's text. Catches a damaged chunk at arrival.
    pub crc: u32,
    /// This chunk's slice of the RON export.
    pub text: String,
}

/// Why a received snapshot transfer cannot be trusted.
///
/// Each variant is a distinct answer with a distinct remedy, which is the whole
/// point of naming them apart: "a chunk is missing" (wait, or ask again), "a
/// chunk is damaged" (the wire corrupted it), "the sender is oversized" (refuse
/// the sender), and "the reassembly does not hash" (the pieces do not belong
/// together) are four different failures, and a receiver told the wrong one acts
/// on the wrong thing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransferError {
    /// The transfer declared `total` chunks, above [`SNAPSHOT_MAX_CHUNKS`]. A
    /// sender cannot make a receiver promise an unbounded buffer.
    TooManyChunks { total: u32 },
    /// A single chunk's text exceeds [`SNAPSHOT_CHUNK_BYTES`].
    ChunkTooLarge { seq: u32, len: usize },
    /// A chunk names a `seq` at or beyond the declared `total`.
    SeqOutOfRange { seq: u32, total: u32 },
    /// A transfer declared zero chunks, or a chunk claimed `total == 0`.
    Empty,
    /// A chunk arrived for a different transfer than the one in progress — a
    /// different id, total or whole-hash. Refused rather than allowed to displace
    /// the transfer already underway.
    TransferMismatch,
    /// The same `seq` arrived twice carrying different bytes. One of them is
    /// corrupt; taking either silently would be a guess.
    DuplicateConflict { seq: u32 },
    /// A chunk's own checksum did not match its text — the wire damaged it.
    ChunkCorrupt { seq: u32 },
    /// Every chunk arrived and each checksummed, but the reassembled whole did
    /// not match the declared whole-payload hash. The pieces do not belong
    /// together.
    WholeHashMismatch { expected: u64, actual: u64 },
    /// [`SnapshotReceiver::finish`] was called before every chunk had arrived.
    Incomplete { received: u32, total: u32 },
}

impl std::fmt::Display for TransferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransferError::TooManyChunks { total } => write!(
                f,
                "the transfer declares {total} chunks, above the {SNAPSHOT_MAX_CHUNKS} \
                 a receiver will buffer"
            ),
            TransferError::ChunkTooLarge { seq, len } => write!(
                f,
                "chunk {seq} is {len} bytes, above the {SNAPSHOT_CHUNK_BYTES}-byte \
                 limit a single frame may carry"
            ),
            TransferError::SeqOutOfRange { seq, total } => {
                write!(
                    f,
                    "chunk {seq} names a sequence at or beyond the {total} declared"
                )
            }
            TransferError::Empty => f.write_str("the transfer declares no chunks"),
            TransferError::TransferMismatch => {
                f.write_str("this chunk belongs to a different transfer than the one in progress")
            }
            TransferError::DuplicateConflict { seq } => {
                write!(f, "chunk {seq} arrived twice with different contents")
            }
            TransferError::ChunkCorrupt { seq } => {
                write!(
                    f,
                    "chunk {seq} failed its own checksum — the wire damaged it"
                )
            }
            TransferError::WholeHashMismatch { expected, actual } => write!(
                f,
                "the reassembled record hashes to {actual:#018x}, not the {expected:#018x} \
                 the transfer declared — the pieces do not belong together"
            ),
            TransferError::Incomplete { received, total } => {
                write!(f, "only {received} of {total} chunks have arrived")
            }
        }
    }
}

/// Split a portable snapshot's text into framed chunks (issue #1117).
///
/// The text is the RON export a local save already produces; this never looks
/// inside it. Splits fall on UTF-8 character boundaries at or before
/// [`SNAPSHOT_CHUNK_BYTES`], so no chunk ever cuts a codepoint and the
/// concatenation of the chunks is the original text byte for byte.
///
/// An empty text still produces exactly one (empty) chunk, so "restore nothing"
/// is a transfer that arrives and completes rather than one a receiver waits on
/// forever.
pub fn chunk(text: &str, from: HostSlot, transfer_id: u64, tick: u64) -> Vec<SnapshotChunk> {
    let whole_hash = vellum_digest::fnv1a(text.as_bytes());

    // Slice on character boundaries so a multibyte codepoint is never split.
    let mut slices: Vec<&str> = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let hard_end = (start + SNAPSHOT_CHUNK_BYTES).min(text.len());
        // Walk back to the nearest char boundary at or below the hard end. The
        // loop terminates: a boundary always exists at `start` itself.
        let mut end = hard_end;
        while end > start && !text.is_char_boundary(end) {
            end -= 1;
        }
        // A single codepoint larger than the chunk budget cannot be split at all;
        // take the whole codepoint. This cannot exceed 4 bytes over budget, and a
        // budget below 4 bytes is not a configuration this ships.
        if end == start {
            end = hard_end;
            while end < text.len() && !text.is_char_boundary(end) {
                end += 1;
            }
        }
        slices.push(&text[start..end]);
        start = end;
    }
    if slices.is_empty() {
        slices.push("");
    }

    let total = slices.len() as u32;
    slices
        .into_iter()
        .enumerate()
        .map(|(seq, slice)| SnapshotChunk {
            from,
            transfer_id,
            tick,
            seq: seq as u32,
            total,
            whole_hash,
            crc: vellum_digest::crc32_ieee(slice.as_bytes()),
            text: slice.to_string(),
        })
        .collect()
}

/// Accumulates the chunks of one transfer, bounded, and hands back the whole text
/// once every chunk has arrived and verified (issue #1117).
///
/// Absent (`None` active) until the first chunk of a transfer arrives. A receiver
/// holds one of these and feeds every snapshot chunk it receives to
/// [`SnapshotReceiver::accept`]; the first chunk that completes the transfer
/// returns the reassembled text, and any fault returns a [`TransferError`] naming
/// what went wrong.
#[derive(Debug, Default)]
pub struct SnapshotReceiver {
    active: Option<Reassembly>,
}

#[derive(Debug)]
struct Reassembly {
    transfer_id: u64,
    total: u32,
    whole_hash: u64,
    chunks: BTreeMap<u32, String>,
}

/// Whether accepting a chunk completed the transfer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Accepted {
    /// The transfer is not complete yet; more chunks are expected.
    More { received: u32, total: u32 },
    /// Every chunk has arrived and the whole payload verified. The text is the
    /// reassembled record, ready for the version gate.
    Complete(String),
}

impl SnapshotReceiver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a transfer is currently in progress.
    pub fn is_receiving(&self) -> bool {
        self.active.is_some()
    }

    /// The sequences still missing from the transfer in progress, in order.
    ///
    /// Empty when no transfer is active or every chunk has arrived. This is what
    /// turns "the join stalled" into "chunks 4, 5 and 9 never arrived".
    pub fn missing(&self) -> Vec<u32> {
        match &self.active {
            None => Vec::new(),
            Some(r) => (0..r.total)
                .filter(|seq| !r.chunks.contains_key(seq))
                .collect(),
        }
    }

    /// How far the transfer in progress has got, as `(received, total)`.
    pub fn progress(&self) -> Option<(u32, u32)> {
        self.active
            .as_ref()
            .map(|r| (r.chunks.len() as u32, r.total))
    }

    /// Take one received chunk.
    ///
    /// Validates the chunk against the transfer's declared bounds and against the
    /// transfer already in progress, checksums it, buffers it, and — once the
    /// last outstanding chunk arrives — reassembles, verifies the whole-payload
    /// hash, and returns the text. Any refusal leaves the receiver's in-progress
    /// state untouched, so a single bad chunk does not discard a good transfer.
    pub fn accept(&mut self, chunk: &SnapshotChunk) -> Result<Accepted, TransferError> {
        if chunk.total == 0 {
            return Err(TransferError::Empty);
        }
        if chunk.total > SNAPSHOT_MAX_CHUNKS {
            return Err(TransferError::TooManyChunks { total: chunk.total });
        }
        if chunk.seq >= chunk.total {
            return Err(TransferError::SeqOutOfRange {
                seq: chunk.seq,
                total: chunk.total,
            });
        }
        if chunk.text.len() > SNAPSHOT_CHUNK_BYTES {
            return Err(TransferError::ChunkTooLarge {
                seq: chunk.seq,
                len: chunk.text.len(),
            });
        }
        if vellum_digest::crc32_ieee(chunk.text.as_bytes()) != chunk.crc {
            return Err(TransferError::ChunkCorrupt { seq: chunk.seq });
        }

        // Begin a transfer, or check this chunk belongs to the one underway.
        match &self.active {
            None => {
                let mut chunks = BTreeMap::new();
                chunks.insert(chunk.seq, chunk.text.clone());
                self.active = Some(Reassembly {
                    transfer_id: chunk.transfer_id,
                    total: chunk.total,
                    whole_hash: chunk.whole_hash,
                    chunks,
                });
            }
            Some(active) => {
                if active.transfer_id != chunk.transfer_id
                    || active.total != chunk.total
                    || active.whole_hash != chunk.whole_hash
                {
                    return Err(TransferError::TransferMismatch);
                }
                // Borrow mutably now that the transfer is confirmed to match.
                let active = self.active.as_mut().expect("just matched Some");
                if let Some(existing) = active.chunks.get(&chunk.seq) {
                    if existing != &chunk.text {
                        return Err(TransferError::DuplicateConflict { seq: chunk.seq });
                    }
                    // An identical re-send is inert, exactly as a repeated
                    // watermark is: reordered and duplicate delivery must converge.
                    return Ok(Accepted::More {
                        received: active.chunks.len() as u32,
                        total: active.total,
                    });
                }
                active.chunks.insert(chunk.seq, chunk.text.clone());
            }
        }

        let active = self.active.as_ref().expect("set above");
        let received = active.chunks.len() as u32;
        if received < active.total {
            return Ok(Accepted::More {
                received,
                total: active.total,
            });
        }

        // Complete: reassemble in sequence order and verify the whole hash.
        let reassembly = self.active.take().expect("complete transfer is active");
        let text = reassembly.reassemble()?;
        Ok(Accepted::Complete(text))
    }

    /// Force completion of the transfer in progress, or say why it cannot.
    ///
    /// [`Self::accept`] already completes a transfer the instant its last chunk
    /// arrives, so this is for a caller that wants to turn a stalled transfer into
    /// a definite refusal — a dropped chunk names itself here as
    /// [`TransferError::Incomplete`] rather than leaving the receiver waiting.
    pub fn finish(&mut self) -> Result<String, TransferError> {
        let Some(reassembly) = self.active.take() else {
            return Err(TransferError::Incomplete {
                received: 0,
                total: 0,
            });
        };
        let received = reassembly.chunks.len() as u32;
        if received < reassembly.total {
            let total = reassembly.total;
            // Put it back so a caller can keep waiting if it chooses.
            self.active = Some(reassembly);
            return Err(TransferError::Incomplete { received, total });
        }
        reassembly.reassemble()
    }
}

impl Reassembly {
    /// Concatenate the buffered chunks in sequence order and verify the whole
    /// payload hashes to the declared value.
    ///
    /// The caller guarantees every sequence `0..total` is present before calling
    /// this.
    fn reassemble(self) -> Result<String, TransferError> {
        let mut text = String::new();
        for seq in 0..self.total {
            match self.chunks.get(&seq) {
                Some(part) => text.push_str(part),
                None => {
                    return Err(TransferError::Incomplete {
                        received: self.chunks.len() as u32,
                        total: self.total,
                    })
                }
            }
        }
        let actual = vellum_digest::fnv1a(text.as_bytes());
        if actual != self.whole_hash {
            return Err(TransferError::WholeHashMismatch {
                expected: self.whole_hash,
                actual,
            });
        }
        Ok(text)
    }
}

#[cfg(test)]
#[path = "transfer_tests.rs"]
mod tests;
