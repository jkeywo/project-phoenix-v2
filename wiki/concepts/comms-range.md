---
title: Comms Range
type: concept
tags: [comms, range, hail, authority, contacts]
sources: [crates/phoenix-simulation/src/comms/traffic.rs, crates/phoenix-simulation/src/snapshot/comms_traffic.rs, crates/phoenix-sim-gameplay/src/electronic/traffic.rs, gui/comms-traffic.js, crates/phoenix-simulation/src/comms/request_returns.rs, crates/phoenix-simulation/src/comms/requests.rs, crates/phoenix-simulation/src/snapshot/comms_requests.rs, crates/phoenix-model/src/comms_channel.rs, crates/phoenix-simulation/src/comms/radio.rs, crates/phoenix-sim-gameplay/src/entities/config/comms.rs, crates/phoenix-model/src/comms_history.rs, crates/phoenix-simulation/src/comms/mod.rs, crates/phoenix-simulation/src/comms/range.rs, crates/phoenix-simulation/src/comms/component.rs, crates/phoenix-simulation/src/comms/roster.rs, crates/phoenix-simulation/src/comms/server.rs, crates/phoenix-simulation/src/console/comms/server.rs, crates/phoenix-model/src/messages.rs, crates/phoenix-sim-gameplay/src/entities/config.rs, crates/phoenix-simulation/src/entities/spawner.rs, gui/comms-state.js, assets/entities/alliance_destroyer.toml]
updated: 2026-10-16
---

# Comms Range

Comms endpoints opt in with an authored `[comms]` block. A contact is reachable when both endpoints support range and their distance is within the effective link range. Hailability and physical range are separate authored facts.

`crates/phoenix-simulation/src/comms/range.rs` contains the pure distance/range calculation. `crates/phoenix-simulation/src/comms/server.rs` maintains each ship's authoritative contact roster and range flags from live transforms. `crates/phoenix-simulation/src/console/comms/server.rs` enforces the same flags when a hail or reply command is applied, so stale or malicious client state cannot bypass range.

`CommsContact.in_range` describes the current roster entry. Legacy plot/text
message range presentation follows its existing live reply gate. Structured
request copies retain their receipt fact instead: current remote reach does not
prevent the receiver from reading or making a local Report/Decline decision,
and does not disclose whether a return has been heard.

Entities with no `[comms]` endpoint are not contacts. Missing range state is denied while range enforcement is active; fixtures/worlds that explicitly run without range use the documented inactive mode rather than fabricated flags.

`CommsRuntime.open_hails` uses Model `comms_history::HailHistory` to retain the
issuing ship and target together. Fleet Human/ordinary AI hail consumers use the
issuing slot's physical gate; clearing and AI candidacy retirement affect only
that source. Snapshot capture/restore and the ordered digest retain the same
source-scoped history. The local roster remains a presentation projection.

Authored `[comms.radio]` equipment is installed by `comms/radio.rs`. It spends
ordinary continuous work for listening and discrete work for each emission.
Receiver copies require matching channel/protection configuration, actual
operation, finite endpoint reach and the existing field/terrain path. Source
feedback records local emission only; received copies retain the original
identity and tick. Both remain in the existing recipient-scoped inbox. The
source's next message identity is independent of conditional receiver count.
Model `comms_channel::validate_transmissions` preflights the entire saved inbox
before restore writes, and `sim_digest.rs` folds transmission metadata.

Optional Radio `traffic.listening` equipment bounds simultaneous receive
channels; the retained own priority order selects them. `comms/traffic.rs`
applies admitted order changes and derives coverage from the same actual paid
endpoint interval. Receivers, relay hops, original-event capture and media
carriers use that coverage; transmission can use any supported source channel.
Absent this equipment, the existing all-channel behavior remains. Derived paid
coverage is neither saved nor folded and is cleared on restore.

Optional alert traffic consumes the exact explicit own Captain feed and its
original dated Alert observation. The ordinary Comms controller can request the
same `ReportCommsAlert` verb as a Human; Radio rechecks freshness, changed semantic
intent, spacing and actual payment before emitting and recording its last
successful tuple. The established publication order remains unchanged. The
Console reports capability, not a guarantee of readiness or remote delivery.
`snapshot/comms_traffic.rs` validates the accepted recipe, canonical intent and
retained publication clocks before restore writes. `gui/comms-traffic.js` and
the existing Workshop Radio editor expose the same closed choices.
Retained dated alert knowledge is allowed to differ from the current private
Alert. Its original value/timestamp survive restore; age determines whether
Radio can still consume it. Validation checks source/owner/shape/clocks rather
than authenticating an erased historical value or rewriting it from machinery.

Equipped non-roster actors build their own contact rows in
`comms/radio_contacts.rs` from the same current paid endpoint pass. Only actual
public Hailable equipment with a compatible unkeyed route is discovered. Loss
of either endpoint's payment, operation or physical path withdraws that row.
The regular Comms publisher preserves these own measurements under the actual
Radio System identity; it still withholds crew-only legacy plot and objectives.
An NPC's planned request therefore requires an independently permitted recipient
before its ordinary controller can send through the normal paid radio service.

Structured requests and returned outcomes use the same radio and inbox. Original
requests are deliberate one-shot transmissions. A local returned Outcome pays
its discrete admission cost once independently of reverse reach; its retained
sent copy can subsequently deliver through actual continuously paid endpoints,
matching protection and the physical path. Reception is idempotent. A broken
return link neither cancels the local decision nor becomes a public failure
receipt; the sender learns later stages only from actually received outcomes.
Ordinary text retains its one-shot delivery semantics.

Saved request lineage and partial ordinary AI reading are validated against the
accepted actual actor recipe, owner, source/recipient and original clock before
restore writes. The same inbox/pending entries participate in the digest; there
is no separate request inbox or retry queue.

`RegionSignalQuery` also applies the current paid equipment receiver factor
through the same physical link gate. Each endpoint's own reach is scaled once;
natural conversation-path attenuation stays separate. See [Electronic
Equipment](./electronic-equipment.md) for actual source work, self/friendly
exposure, restore timing and own-receiver notice boundaries.

## Related

- [Comms Panel](./comms-panel.md)
- [World Data](../entities/world-data.md)
