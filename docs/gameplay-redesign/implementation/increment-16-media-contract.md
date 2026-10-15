# Selected human media implementation contract

Adopted before runtime changes on 2026-10-15, coordinated with the Comms owner.
This extends the existing Phoenix transport; it adds no peer, broker or delivery
class. Device and real cross-host acceptance remain unclaimed until exercised.

## Canonical game owners

Model adds `MediaMode::{Audio, AudioVideo}` and
`GenericCommsRequest::MediaCall { mode }`. Calls are Contact-only requests through
the existing paid, retained request path. The recipient explicitly reads and
accepts or declines. Ordinary Comms AI declines after its existing reading work
and normal response admission; it never creates a voice or device stream.
The source must actually receive the matching returned Accept. Recipient-local
accepted lineage and sender-local returned lineage remain distinct evidence.

`SetCommsMediaCall { origin_id: Option<String> }` is an ordinary admitted Comms
System command selecting or clearing one genuinely eligible own call. The new
canonical `ShipCommsMediaIntent` retains this selection in snapshot and digest.
Selecting a conversation confers no other control authority. Explicit radio
equipment media capability bounds the supported mode; capability is not codec
negotiation or permission to access a local device.

Both endpoints must select the same actual call and have valid accepted/heard
lineage. The existing fixed `tick_radio` derives current paid carrier eligibility
after its ordinary once-per-tick listening work. It uses existing physical
route, equipment, held-key, interference and protection rules. No device callback
can mutate canonical Comms or power, or charge per-packet nondeterministic work.
The derived carrier cache retains its actual sample tick, expires when no longer
current and clears on restore. It is not a second authoritative radio store.

## Ephemeral session and packet owners

A fresh explicit local start binds a non-restored lease to the current human
System/Console/Station operator, actual selected call/mode, and current local or
frozen host-slot owner. Stop, operator/call change, disconnect, reconnect and
restore invalidate leases. Accepted history never reopens capture.

The closed packet carries actual call origin, lease generation, monotonic u32
sequence, original local capture offset, supported audio/video kind and bounded
bytes. Source and destination validate current eligibility independently. A
cross-host sender derives its authenticated frozen slot from the existing link;
payload tokens or claimed ships cannot grant route authority. The destination
resolves its own current local Session. Raw packets and leases never enter
command history, lockstep continuation streams, snapshots or digests.

## Frozen initial browser and transport limits

- PCM16 little-endian, mono 16000 Hz: exactly 80 ms / 1280 samples / 2560 bytes.
- Independent JPEG stills: at most 12288 bytes and two frames per second.
  Check encoded JPEG dimensions before decoding: width at most 320, height at
  most 180, and at most 57600 pixels. Capture uses the same bounded canvas.
- Full encoded JSON: at most 20480 bytes, measured before general ingress staging.
- Playback: at most 200 ms audio and one pending video frame; drop stale work.
- Optional generic `sendSnapshotOnly(encoded, {maxBytes, maxBufferedBytes})`
  returns success/failure, checks actual backlog, and never falls back to
  Reliable. Existing generic sends retain their current compatibility behaviour.
- Game adapters own media rate/age/replacement rules. Reusable transport knows
  only bounded opaque strings, current physical readiness and backlog.

Capture requires explicit local activation and supported secure browser APIs.
Permission denial, insecure-origin clients and embedded native panes report
unavailable. Native panes currently lack a browser media backend. Independent
packets are required: lossy arbitrary MediaRecorder container chunks are excluded.

## Consumer proof and integration handoff

Targeted JavaScript tests prove real helper use on direct and fallback channel
adapters, with ordinary command delivery unchanged; absent/overloaded/oversize
media never reaches Reliable. Browser tests cover actual production capture and
playback owners through injected devices, packet validation, bounded queues,
permission races and cleanup. Native owner tests must establish ordinary call
receipt/acceptance, intent admission, paid route loss, former-operator refusal
and no byte/lease restoration. They do not substitute for device acceptance.

Root owns central Client/Server/System message declarations, registration,
snapshot/digest and host-mesh joins. This lane supplies explicit typed leaves and
hook requirements. Actual two-host/two-human direct and native/fallback calls
with simultaneous commands and gameplay snapshots are required before claiming
the full media exit. Shared Snapshot pressure, pane semantics and authenticated
mesh routing must be measured rather than inferred from a same-host demo.


## Negotiated drop-only Snapshot queue policy

The existing relay advertises `limits.snapshot_drop_only = true` only when its
ordinary Snapshot mailbox implements optional drop-only work. The same existing
relay frame can then carry `drop_only: true` with class `snapshot`. A full mailbox
drops an incoming optional frame. An ordinary snapshot displaces optional work
first and is drained before all queued optional work, across senders. There is
no third delivery class, retained stream or media-aware service parser. Older or
unadvertised policies refuse optional sends; ordinary delivery is unchanged.
Deploy the service capability before enabling relayed media clients.

Native sockets expose an opt-in bounded send against the actual queue/open
state. Native bridge callers with no synchronous backlog must use an explicit
bounded staging/ready/receipt seam, rather than claiming a zero backlog. The
same physical links remain authoritative for source connection incarnation.
Generic scoped ingress keeps that incarnation for selected ephemeral inputs;
unknown adapters refuse them. Full encoded media bounds apply before general
browser/native staging, including reordered, escaped and malformed envelopes.

Root delegated media-only central integration to this lane on 2026-10-15;
root retains final compatibility versions, state census and integrated native
validation. The native fleet bridge is an independent coordinated donor.

## Live route continuation clarification

An authenticated heartbeat for the current paid accepted call may report the
receiver's retained generation floor when it has no active matching peer lease.
The sender raises its ephemeral generation high-water and invalidates a matching
start at or below that floor. Capture then requires a fresh explicit human start.
This lets a replacement host process obtain a safe generation without clearing
the surviving receiver's stale-frame protection. A frame packet cannot carry a
floor. Wrong call, slot or physical incarnation cannot update it; exhausting the
bounded generation range refuses further starts instead of wrapping. None of
these receipt-time actions changes the canonical call or machinery.
