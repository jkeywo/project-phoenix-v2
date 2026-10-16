---
title: Sensing and Spatial Fields
type: concept
tags: [science, observations, regions, simulation]
sources:
  - crates/phoenix-model/src/shield_enquiry.rs
  - crates/phoenix-simulation/src/science/shield_enquiry.rs
  - crates/phoenix-simulation/src/ship/shield_enquiry.rs
  - crates/phoenix-model/src/focused_enquiry.rs
  - crates/phoenix-model/src/sensing.rs
  - crates/phoenix-model/src/sensing_forecast.rs
  - crates/phoenix-sim-gameplay/src/science/forecast.rs
  - crates/phoenix-sim-gameplay/src/science/tracks.rs
  - crates/phoenix-sim-gameplay/src/science/sharing.rs
  - crates/phoenix-simulation/src/science/sharing.rs
  - crates/phoenix-simulation/src/console/navigation/sharing.rs
  - crates/phoenix-simulation/src/sim_sets/order.rs
  - crates/phoenix-sim-gameplay/src/science/sensing.rs
  - crates/phoenix-sim-gameplay/src/science/signatures.rs
  - crates/phoenix-simulation/src/science/broad.rs
  - crates/phoenix-simulation/src/science/contact_routing.rs
  - crates/phoenix-simulation/src/science/focused_ai.rs
  - crates/phoenix-simulation/src/console_delivery.rs
  - crates/phoenix-simulation/src/console_delivery_references.rs
  - gui/components/ph-scan-readout.js
  - gui/console-state.js
  - gui/sensor-tracks.js
  - gui/components/ph-radar.js
  - gui/workshop-sensing-panel.js
  - crates/phoenix-sim-world/src/world/script/triggers_enquiry.rs
  - crates/phoenix-simulation/src/science/enquiry.rs
  - crates/phoenix-simulation/src/science/server.rs
  - crates/phoenix-simulation/src/science/focused.rs
  - crates/phoenix-simulation/src/science/passive.rs
  - crates/phoenix-sim-gameplay/src/regions/field.rs
  - crates/phoenix-sim-gameplay/src/regions/signal_path.rs
  - crates/phoenix-simulation/src/snapshot/spatial_fields.rs
  - crates/phoenix-simulation/src/snapshot/sensing_publications.rs
---

# Sensing and Spatial Fields

`SensingConfig.focused`, `broad` and `passive` are arrays of fitted instruments.
`ShipScanRecord` maps actual System IDs to active paid focused enquiries, bounded unfinished work,
retained readings and the latest-screen projection. `tick_scans` admits no free
completion: current geometry, shared Sensors path, operational condition and
actual local energy constrain progress. General completion samples published
condition and emits the existing scenario scan flag. Historical readings do not track
later live condition changes. The Console publication carries the actual owner
for controls. Each qualified `scan/{UTF8-byte-length}:{owner}` channel follows its
actual System’s Console; an exact typed feed grants information only. Completed
readings retain the public contact alias captured by the enquiry.

`FocusedAim` closes supported questions to General, Environment, Resources and
Anomaly. Equipment declares optional question support and relative duration/cost;
all use the same active work slot. Subject `enquiry_observation` rules independently
permit normal or intrusive measurement of actual local field exposure, labelled
finite capacities or supported effect signatures. `EnquiryScene` samples the
receiver or an already-observed subject point, withholds ambiguous public quantity
labels and never publishes internal capacity IDs. Non-General readings carry typed
quantities, with absent structural condition and bulk mass rather than fabricated
values. Protected and absent answers share the ordinary refusal.

The first actual paid work and its completion enqueue observer-aware `Enquiry`
events on the existing World event path. `on_enquiry` scripts filter subject,
observer, System, question, mode and phase. This supports authored contextual
reactions; it does not guarantee that every subject detects every scan. Ordinary
System AI preserves an active human question/mode for the same observed mission
handle, including after handoff. Only a genuinely different request can retarget it.

An optional focused `forecast` policy supplies a finite positive horizon. On an
actual completed enquiry, the forecast consumer uses two retained readings of
the same public contact, question and mode at different sample ticks. It projects condition ranges
with quantisation uncertainty and common observed resource levels under an
explicit unchanged-trend assumption. The issued forecast stays on the existing
canonical `ScanReading`, including its original samples and horizon; later live
changes and pruning earlier input samples cannot rewrite it. Snapshot validation
checks interval shape, observed quantities, accepted policy and fixed logical time.
The scan readout displays this payload without running its own prediction model.

`PassiveProfile` and `PassiveState` map actual System IDs to independent powered
receivers, ordinary closed AI frames, enabled intent and actual measured samples.
`science::signatures::observed_signature` measures their strongest actual reactor
or supported machinery cause, preserving reactor-first and authored-order ties.
Actual radio/transponder work uses the Comms cause, fitted active instruments
use ActiveScan, and remaining performed auxiliary work is Unclassified.
Passive supplies current output and work at its existing measurement boundary;
the helper stores no extra physical state or observer knowledge.
Physical producers use actual resource expenditure, mount coverage and signal
paths. Public signals carry bearing, error, signature, strength and dated local
provenance rather than the emitting actor's private identity. Existing screens
show enquiry progress/history and passive receiver controls/readings.

`SensingConfig.broad` is an authored array of independent instruments. Each entry
names a fitted Helm, Tactical or Sensor radar and derives range/filter coverage
from that capability's existing configuration. `BroadProfile` and `BroadTracks`
are keyed by actual System ID. Each `BroadTrackState` advances only from its own
paid grant and physical measurements, retains acquired findings with their sample
ticks and predicts lost positions using observed motion. Public contact IDs include
their instrument owner; current actuator routing stays private and expires with
measured support. Co-hosting instruments does not merge their knowledge.
Installed live owners share the same lifecycle/projection body as the validating
public APIs, avoiding repeated full-history copies and release-time validation.
Construction and restore retain complete validation; physical input checks,
every-tick paid support, routing and publication cadence remain unchanged.
Physical returns also carry quantized elevation with their original sample tick
and error bound. Vertical velocity requires two distinct paid physical samples;
reports without measured height retain `None`. Held and relayed elevation stays
dated evidence, and disjoint height intervals cannot fuse as one planar return.
Generated gameplay rocks participate using their existing `AsteroidUuid`, actual
physical pose and authored roster tags. Visibility follows the scanner's `shows`
filter; selection separately follows the rock template's `target.tags` and the
scanner's `selects`. Their measured returns use the same paid range/path rules.

`SensingConfig.sharing` configures each actual Sensors or Tactical radar's
own-faction sharing and independent reception intent. Received originals live
in that instrument's existing `BroadTrackState`, preserving their public origin,
sample time, measuring pose and first receipt. Delivery uses actual fictional
Comms reach; a lost link retains evidence without renewing its age or actuator
route. The public projection may narrow compatible, reciprocally unambiguous
independent error discs; exports retain the original samples. Copies and relays
never add independent support. `radar-sharing-controls.js` presents the exact
owned radar's controls, while foreign feeds remain informational.

`navigation_console.sharing` binds Navigation's distinct waypoint/chart feed to
its actual System. The adjacent `NavigationSharing` frontier holds dated fixed
points from the source's own course and explicitly charted landmarks within its
configured chart coverage. A mobile anchor contributes only its permitted held
measurement, with the original sample clock; a private live anchor cache grants
no geometry. Received points preserve original provenance across relays and
never replace the receiving course or Helm clearance. Applying a received point
uses the ordinary owned, paid `SetNavigationWaypoint` path.
The Navigation plugin installs the accepted owner and derived chart profile;
snapshot preflight binds the saved intent/history to that owner and logical
clock before writes. Capture, restore and the digest carry the same frontier.
The sharing owner runs late in Modifiers after scans, Broad and actual radar-feed
delivery, before objective cursor advancement. Its current-route guard sees that
completed receipt frontier; physical modifier translation and Tractor keep their
earlier order.
Custom Navigation IDs also own ordinary waypoint and traffic-order payment;
accepted composition never falls back to an absent canonical ID.

An observer's substituted Sensors reports enter that same instrument-owned track
lifecycle. They retain the reported position, permitted label and original sample
metadata; repeated receipt does not renew their age or acquire current physical
facts. Public metadata names the instrument's contact handle, while the actual
actor remains only a private current actuator route. Other radar instruments do
not inherit the Sensors report. Explicit synthetic points use those same retained
tracks and generic reported provenance, with no physical actuator route. Their
private no-actor distinction is never copied into a public decoy-specific list or
flag; the measured GUI shows ordinary nonselectable reported evidence.

`contact_routing` resolves a current selectable public return through the consuming
System's permitted Console view before consulting its private actuator association.
Helm, Tactical and Science publish each actual instrument's measured records;
an empty measured picture does not fall back to world entities. Ordinary target
selectors use those permitted records and the configured System's closed tree.
Additional focused controllers skip the designated Science owner. Routine reference
matching withholds ambiguous display labels, and selecting an anonymous return
does not supply a name from the machinery's live entity lookup.

Marked held findings remain in World's existing `EvidenceLog`. Gameplay
`science/references` derives routine or deliberately attached views; Simulation
`science/references` admits attachment through the radar owner's permitted typed
feeds. `science/reading` advances private semantic-text progress inside the
existing System tree runtime. This work has its own logical-tick cadence and
does not install another controller. Snapshot `spatial_fields` validates the
candidate's evidence and reading clocks before restore writes any state.

Broad also samples actual `TorpedoSystemResource` in-flight positions, without
copying launcher/guidance orders into measurements. Gameplay `science/approaches`
derives conditional closest approaches and conservative corridors from public
track DTOs. Simulation's corresponding publisher reads the Helm owner's permitted
feeds after ordinary publication. `helm_console.projection` enables the display;
it does not modify motion. The browser Helm radar paints the delivered corridors
and states their unchanged-motion assumption.

The existing browser and native outbound sinks use `ConsoleDeliveryProjection`
to preserve own-ship data and explicitly charted landmarks while withholding
foreign live snapshots and unseen lifecycle events. Typed target references are
projected through their producing Console's permitted instrument observations.
The accepted client capability roster keeps this boundary active after loss of
the local hull. The scan readout selects actual owners from `scans[]`; its controls
always address that selected System.

Workshop edits the whole sensing profile through accepted source/history, with
separate add/edit/remove rows for each role, supported questions, forecast policy
and subject observation permissions. Explicit `sensing.control` bindings
identify the singular Combat Lock and Science Designation machinery owners,
without pooling the independent instruments’ observations. Authored NPC Tactical radars
now have finite explicit reach; the old absent-table unbounded acquisition is not
an alternate equipment source.

`RegionFieldState` owns bounded analytic parameters, origin and causal tick anchor.
`RegionSignalQuery` shares deterministic segment attenuation/occlusion between
actual sensing and fictional Comms reach. Explicit `SignalMaterialConfig` and
physical shape bounds describe substantial terrain; rendered mesh detail does
not define obstruction. `replace_region_field` replaces accepted field parameters
through the ordinary typed World effect boundary.

Region membership, exposure and sampled pose are derived. Restore reconstructs
these views, Region-owned modifiers and actual Comms reach before the next Input,
preserving captured hail intent and avoiding synthetic boundary events. Field and
recorded-observation continuations participate in the existing snapshot/digest;
the snapshot artifact and shared protocol constants declare the compatibility boundary.

Configured Navigation builds its candidates and moving-anchor refresh from its
own permitted measured picture and explicit filtered chart. Lost observations
hold the last legitimate waypoint; foreign Transform and current itinerary are
not alternate knowledge sources. Identified civilian contacts expose fixed
authored interaction choices. Live route/compliance requires an explicit received
Navigation claim. Its existing waypoint owner retains the latest successful own
request per public contact to avoid repeated AI payment, without treating that
receipt as foreign compliance. These receipts save and digest with that owner.

Optional `shields` enquiries use the same paid, System-owned focused slot.
Subjects explicitly permit `shield_frequency` acquisition. A typed finding in
`ScanReading` records quantized frequency, response and the observed configuration
signature; its history is never upgraded from current enemy state. The existing
frequency advisories read only qualified acquired Scan publications through the
consumer's actual Console scope. Broad assessment supplies no such finding. NPC automatic phaser matching uses
that same acquired evidence through each AI weapon bank's Console; current
observation loss, feed revocation or Conceal pauses its advisory delay. Target
frequency rotation never upgrades an old finding.
`science/shield_enquiry.rs` owns that read seam and `science/enquiry.rs` owns
physical acquisition. Optional authored shield cycles update the existing
`ShipShields` frequency from a saved logical anchor in `ship/shield_enquiry.rs`.
Workshop extends its existing sensing/subject and shield forms; no new scanner
controller, inventory or observation store is involved.

The broader redesign contract and incomplete work belong in PASM and the
implementation/acceptance records, rather than this current-owner index.

Restore rebuilds missing derived publications, then `sensing_publications::restore_held`
retains only a saved radar's `tracks` when they exactly match its canonical paid
instrument state at the saved counter or preceding fixed-step publication.
Actual System ownership and publication kind must agree. Forged or missing views
keep the rebuilt projection; controls, selections, physical routing and acquisition
clocks are never copied from that held view. Ordinary publication advances its age
on the next tick.

The Alliance base hulls declare an actual `Target` capability with ship selection
tags. Display tags alone do not make an entity selectable: the paid radar record
and the receiving Console's permitted feed still govern target admission.
Shield-frequency automation correlates different radar handles only when the weapon Console explicitly receives both current selectable observations and the acquired Scan feed. The ordinary contact resolver may establish that both handles route to the same physical actuator target; acquired reading subject, time and configuration remain unchanged. The curated Tactical/Gunnery SensorRadar feed is an explicit information grant, and losing either current route withholds fresh automatic tuning while history remains visible.

Helm combines only its delivered Helm and Tactical tracks. A permitted return
beyond the local display scale remains visible as an outward rim chevron; its
measured position is unchanged. Predicted returns stay dashed and cannot be
selected. Off-scale returns omit the position-error circle because its centre
would otherwise imply a location on the rim. This changes presentation only,
not detection range or Console feed permissions.
