//! Exhaustive unit coverage for the pure chunk/reassemble/integrity protocol
//! (issue #1117). Fabricated byte buffers only — no simulation, no snapshot, no
//! Bevy. Every fault the mesh receiver has to name is provoked here on a string
//! nobody had to capture.

use super::*;

const FROM: HostSlot = HostSlot(2);
const TRANSFER: u64 = 0x1117_1117;
const TICK: u64 = 412;

/// A body large enough to force several chunks, deterministic so a corruption is
/// a chosen edit rather than luck.
fn body(len: usize) -> String {
    (0..len)
        .map(|i| char::from(b'a' + (i % 26) as u8))
        .collect()
}

/// Deliver a slice of chunks to a fresh receiver in the given order, returning
/// the completion outcome of the LAST accept.
fn deliver(order: &[usize], chunks: &[SnapshotChunk]) -> Result<Accepted, TransferError> {
    let mut rx = SnapshotReceiver::new();
    let mut last = Ok(Accepted::More {
        received: 0,
        total: chunks.len() as u32,
    });
    for &i in order {
        last = rx.accept(&chunks[i]);
    }
    last
}

// ── Round trips ──────────────────────────────────────────────────────────────

/// The headline: a body chunked and reassembled is the body, byte for byte.
#[test]
fn a_chunked_body_reassembles_to_itself() {
    let text = body(SNAPSHOT_CHUNK_BYTES * 3 + 17);
    let chunks = chunk(&text, FROM, TRANSFER, TICK);
    assert!(
        chunks.len() > 3,
        "the body must have split: {}",
        chunks.len()
    );

    let mut rx = SnapshotReceiver::new();
    let mut result = None;
    for c in &chunks {
        if let Accepted::Complete(t) = rx.accept(c).expect("each chunk accepts") {
            result = Some(t);
        }
    }
    assert_eq!(result.as_deref(), Some(text.as_str()));
}

/// Order does not matter: the chunks reassemble by sequence, not by arrival.
#[test]
fn chunks_reassemble_in_sequence_order_whatever_order_they_arrive_in() {
    let text = body(SNAPSHOT_CHUNK_BYTES * 4);
    let chunks = chunk(&text, FROM, TRANSFER, TICK);
    let n = chunks.len();
    // A deliberately jumbled order that still covers every sequence exactly once.
    let mut order: Vec<usize> = (0..n).rev().collect();
    order.swap(0, n / 2);

    match deliver(&order, &chunks).expect("reassembles") {
        Accepted::Complete(t) => assert_eq!(t, text),
        other => panic!("a full jumbled delivery must complete, got {other:?}"),
    }
}

/// An empty record is a transfer that arrives and completes, not one that hangs.
#[test]
fn an_empty_body_is_one_chunk_that_completes() {
    let chunks = chunk("", FROM, TRANSFER, TICK);
    assert_eq!(chunks.len(), 1, "empty still frames one chunk");
    let mut rx = SnapshotReceiver::new();
    assert_eq!(
        rx.accept(&chunks[0]).expect("accepts"),
        Accepted::Complete(String::new())
    );
}

/// A body under one chunk is one chunk, and completes on that chunk.
#[test]
fn a_small_body_is_a_single_chunk() {
    let text = body(64);
    let chunks = chunk(&text, FROM, TRANSFER, TICK);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].total, 1);
    let mut rx = SnapshotReceiver::new();
    assert_eq!(
        rx.accept(&chunks[0]).expect("accepts"),
        Accepted::Complete(text)
    );
}

/// No chunk ever exceeds the byte budget, and every chunk repeats the whole
/// transfer's facts so a receiver can validate one it sees before chunk zero.
#[test]
fn every_chunk_is_bounded_and_carries_the_whole_transfer_facts() {
    let text = body(SNAPSHOT_CHUNK_BYTES * 5 + 3);
    let chunks = chunk(&text, FROM, TRANSFER, TICK);
    let whole = chunks[0].whole_hash;
    for (i, c) in chunks.iter().enumerate() {
        assert!(
            c.text.len() <= SNAPSHOT_CHUNK_BYTES,
            "chunk {i} is {} bytes, over budget",
            c.text.len()
        );
        assert_eq!(c.seq as usize, i);
        assert_eq!(c.total as usize, chunks.len());
        assert_eq!(
            c.whole_hash, whole,
            "the whole hash is repeated on every chunk"
        );
        assert_eq!(c.from, FROM);
        assert_eq!(c.transfer_id, TRANSFER);
        assert_eq!(c.tick, TICK);
    }
}

/// A multibyte codepoint is never split through the middle: the reassembly is
/// still valid UTF-8 and equal to the source.
#[test]
fn multibyte_codepoints_are_never_split() {
    // A body of 3-byte and 4-byte codepoints, sized so boundaries land inside
    // characters if the splitter is naive.
    let unit = "日本語🚀"; // 3+3+3+4 = 13 bytes
    let repeats = (SNAPSHOT_CHUNK_BYTES * 2 / unit.len()) + 5;
    let text: String = unit.repeat(repeats);
    let chunks = chunk(&text, FROM, TRANSFER, TICK);
    assert!(chunks.len() >= 2, "the body must have split");
    for c in &chunks {
        // If a split fell inside a codepoint, this slice would not be valid UTF-8
        // — but it is a `String`, so instead assert the crc matches, proving the
        // bytes are exactly what was hashed.
        assert_eq!(vellum_digest::crc32_ieee(c.text.as_bytes()), c.crc);
    }
    let joined: String = chunks.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(
        joined, text,
        "the concatenation is the source, char boundaries intact"
    );
}

// ── Integrity refusals ───────────────────────────────────────────────────────

/// A chunk damaged in flight fails its own checksum on arrival — distinct from a
/// missing one and named the instant it lands.
#[test]
fn a_corrupt_chunk_is_refused_on_arrival() {
    let text = body(SNAPSHOT_CHUNK_BYTES * 2);
    let mut chunks = chunk(&text, FROM, TRANSFER, TICK);
    // Change a byte in chunk 1's text WITHOUT updating its crc — the wire changed
    // the bytes, not the sender. A `wrapping_add` on a lowercase letter stays
    // ASCII, so the chunk is still a valid `String` but no longer matches its crc.
    let mut bytes = chunks[1].text.clone().into_bytes();
    bytes[10] = bytes[10].wrapping_add(1);
    chunks[1].text = String::from_utf8(bytes).expect("still ascii");

    let mut rx = SnapshotReceiver::new();
    rx.accept(&chunks[0]).expect("chunk 0 is intact");
    assert_eq!(
        rx.accept(&chunks[1]),
        Err(TransferError::ChunkCorrupt { seq: 1 }),
        "a chunk whose text no longer matches its checksum is corrupt, not missing"
    );
}

/// A whole payload whose pieces individually checksum but do not belong together
/// is caught at completion by the whole-payload hash.
#[test]
fn a_reassembly_that_does_not_hash_is_refused_at_completion() {
    let text = body(SNAPSHOT_CHUNK_BYTES * 2);
    let mut chunks = chunk(&text, FROM, TRANSFER, TICK);
    // Rewrite chunk 1's text AND its crc so the chunk is internally consistent,
    // but the whole no longer matches `whole_hash`. This is the class a per-chunk
    // crc cannot catch and the whole hash exists for.
    chunks[1].text = body(chunks[1].text.len());
    chunks[1].crc = vellum_digest::crc32_ieee(chunks[1].text.as_bytes());

    let mut rx = SnapshotReceiver::new();
    rx.accept(&chunks[0]).expect("chunk 0");
    match rx.accept(&chunks[1]) {
        Err(TransferError::WholeHashMismatch { expected, actual }) => {
            assert_eq!(expected, chunks[0].whole_hash);
            assert_ne!(actual, expected);
        }
        other => panic!("expected a whole-hash mismatch, got {other:?}"),
    }
}

/// A dropped chunk is a gap, named as `Incomplete` rather than silently awaited
/// forever — the distinct answer a receiver gives to "a piece never came".
#[test]
fn a_dropped_chunk_is_reported_as_a_gap() {
    let text = body(SNAPSHOT_CHUNK_BYTES * 3);
    let chunks = chunk(&text, FROM, TRANSFER, TICK);
    assert!(chunks.len() >= 3);

    let mut rx = SnapshotReceiver::new();
    // Deliver everything except chunk 1.
    for c in chunks.iter().filter(|c| c.seq != 1) {
        assert!(matches!(rx.accept(c), Ok(Accepted::More { .. })));
    }
    assert_eq!(
        rx.missing(),
        vec![1],
        "the receiver names the missing sequence"
    );
    match rx.finish() {
        Err(TransferError::Incomplete { received, total }) => {
            assert_eq!(received, chunks.len() as u32 - 1);
            assert_eq!(total, chunks.len() as u32);
        }
        other => panic!("a gap must finish as Incomplete, got {other:?}"),
    }
    // …and the transfer is still recoverable: the missing chunk completes it.
    let one = chunks.iter().find(|c| c.seq == 1).unwrap();
    assert!(matches!(rx.accept(one), Ok(Accepted::Complete(_))));
}

// ── Bounded reassembly ───────────────────────────────────────────────────────

/// A sender declaring more chunks than the ceiling is refused before a single
/// chunk is buffered — the bound that stops an unbounded-buffer attack.
#[test]
fn a_transfer_declaring_too_many_chunks_is_refused_up_front() {
    let good = &chunk(&body(10), FROM, TRANSFER, TICK)[0];
    let evil = SnapshotChunk {
        total: SNAPSHOT_MAX_CHUNKS + 1,
        seq: 0,
        ..good.clone()
    };
    let mut rx = SnapshotReceiver::new();
    assert_eq!(
        rx.accept(&evil),
        Err(TransferError::TooManyChunks {
            total: SNAPSHOT_MAX_CHUNKS + 1
        })
    );
    assert!(!rx.is_receiving(), "nothing was buffered");
}

/// An oversized chunk is refused, so a sender cannot exceed the per-frame bound
/// that keeps the total buffer bounded.
#[test]
fn an_oversized_chunk_is_refused() {
    let base = &chunk(&body(10), FROM, TRANSFER, TICK)[0];
    let big = body(SNAPSHOT_CHUNK_BYTES + 1);
    let evil = SnapshotChunk {
        crc: vellum_digest::crc32_ieee(big.as_bytes()),
        text: big,
        total: 4,
        ..base.clone()
    };
    let mut rx = SnapshotReceiver::new();
    assert_eq!(
        rx.accept(&evil),
        Err(TransferError::ChunkTooLarge {
            seq: 0,
            len: SNAPSHOT_CHUNK_BYTES + 1
        })
    );
}

/// A sequence at or beyond the declared total is refused, and a zero-total
/// transfer is empty.
#[test]
fn out_of_range_and_empty_transfers_are_refused() {
    let base = &chunk(&body(10), FROM, TRANSFER, TICK)[0];
    let mut rx = SnapshotReceiver::new();
    assert_eq!(
        rx.accept(&SnapshotChunk {
            seq: 4,
            total: 4,
            ..base.clone()
        }),
        Err(TransferError::SeqOutOfRange { seq: 4, total: 4 })
    );
    assert_eq!(
        rx.accept(&SnapshotChunk {
            seq: 0,
            total: 0,
            ..base.clone()
        }),
        Err(TransferError::Empty)
    );
}

/// A chunk from a different transfer cannot displace the one underway.
#[test]
fn a_chunk_from_another_transfer_is_refused() {
    let a = chunk(&body(SNAPSHOT_CHUNK_BYTES * 2), FROM, TRANSFER, TICK);
    let b = chunk(&body(SNAPSHOT_CHUNK_BYTES * 2), FROM, TRANSFER + 1, TICK);
    let mut rx = SnapshotReceiver::new();
    rx.accept(&a[0]).expect("transfer A begins");
    assert_eq!(
        rx.accept(&b[1]),
        Err(TransferError::TransferMismatch),
        "a second transfer's chunk must not join the first"
    );
    // The original transfer is intact and still completes.
    rx.accept(&a[1]).expect("A continues");
    assert!(matches!(rx.accept(&a[0]), Ok(_)), "A still live");
}

/// The same sequence resent identically is inert; resent with different bytes is
/// a conflict, because one copy is corrupt and choosing is a guess.
#[test]
fn a_conflicting_duplicate_is_refused_but_an_identical_one_is_inert() {
    let chunks = chunk(&body(SNAPSHOT_CHUNK_BYTES * 3), FROM, TRANSFER, TICK);
    let mut rx = SnapshotReceiver::new();
    rx.accept(&chunks[0]).expect("first copy");

    // Identical resend: inert, no step backwards.
    assert!(matches!(
        rx.accept(&chunks[0]),
        Ok(Accepted::More { received: 1, .. })
    ));

    // Different bytes under the same seq (crc kept consistent so it is a genuine
    // content conflict, not a corrupt chunk). "Z" is outside `body`'s lowercase
    // cycle, so the clash text genuinely differs from the original chunk 0.
    let mut clash = chunks[0].clone();
    clash.text = "Z".repeat(chunks[0].text.len());
    clash.crc = vellum_digest::crc32_ieee(clash.text.as_bytes());
    assert_ne!(clash.text, chunks[0].text, "the clash must actually differ");
    assert_eq!(
        rx.accept(&clash),
        Err(TransferError::DuplicateConflict { seq: 0 })
    );
}

/// The declared ceilings are consistent: the derived chunk cap matches the byte
/// cap, so the two documented bounds cannot drift.
#[test]
fn the_bounds_are_internally_consistent() {
    assert_eq!(
        SNAPSHOT_MAX_CHUNKS as usize,
        SNAPSHOT_MAX_TRANSFER_BYTES / SNAPSHOT_CHUNK_BYTES
    );
    assert!(
        SNAPSHOT_CHUNK_BYTES >= 4,
        "a chunk must fit any single codepoint"
    );
}
