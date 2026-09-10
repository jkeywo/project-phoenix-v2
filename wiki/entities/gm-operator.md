---
title: GM Operator
type: entity
tags: [gm, operator, identity, reconnect, roster, readiness, force-start, action, pause, puppeting, backfill, host-mesh, map, activity, damage, destruction, objectives, triggers, red-alert, connections, regions, asteroids]
sources: [src/native_host/native_gm/mod.rs, gui/gm-workspace.js, gui/gm-workspace-shell.js, gui/gm-workspace.css, gui/native-gm-workspace.js, pasm/spec/design/native-bridge-operation.yaml, tests/smoke/gm-m2.spec.js, tests/smoke/gm-m2-evidence.js, docs/acceptance/1316-m2-combat-test.md, assets/worlds/combat_test.toml, gui/gm-confirmation.js, gui/gm-confirmation-settings.js, gui/gm-confirmation.css, src/gm_objective.rs, gui/gm-objective-panel.js, src/gm_event.rs, src/gm_effect.rs, src/gm_spawn.rs, src/world/config.rs, src/world/content.rs, src/world/script/, gui/gm-mission-panel.js, gui/gm-direct-effect-panel.js, gui/gm-effect-scope.js, gui/gm-spawn-panel.js, gui/gm-knowledge-compare.js, gui/gm-role-presets.js, pasm/spec/design/gm-console-t2.yaml, src/gm_roster.rs, src/gm_action.rs, src/gm_join.rs, src/gm_projection.rs, src/gm_activity.rs, src/gm_puppet.rs, src/objectives.rs, src/world/server.rs, src/ship/helm_ai/mod.rs, src/entities/config.rs, src/entities/tags.rs, src/asteroids/lifecycle.rs, src/boot/mod.rs, src/lobby/start_policy.rs, src/core/balance.rs, src/core/messages.rs, src/core/codec.rs, src/command_admission/log.rs, src/lobby/server.rs, src/lockstep/frame.rs, src/lockstep/host_loss.rs, src/lockstep/mod.rs, src/lockstep/snapshot_relay.rs, src/server/bridge.rs, src/server_app/broadcast.rs, src/server_app/world_setup.rs, src/snapshot.rs, src/sim_digest.rs, src/headless/replay.rs, gui/host-channel.js, gui/gm-local-projection.js, gui/gm-activity-feed.js, gui/gm-station-puppet.js, gui/entity-inspector.js, gui/components/ph-navigation-map.js, gui/gm-session-actions.js, gui/gm-session-controls.js, gui/console-state.js, gui/console-core.js, gui/sim-state.js, gui/host-mesh.js, gui/fleet-session.js, gui/lobby-state.js, server.html, client.html]
updated: 2026-09-08
---

# GM Operator

A GM Operator is a privileged operator admitted through the fleet-host path or
the native host's private local GM surface. It is not a [Player](./player.md),
Spectator, Station holder, or player ship. Several GMs may be present and all
have the same role; the host mesh's technical star centre and private mesh slot
do not create a public leader or permission tier.

The native local operator shares the ship's one authoritative simulation. It
has no separate fleet peer or browser GM-only boot profile. The host assigns a
dedicated screen before launch; loss preserves identity and pauses the mission,
and recovery requires explicit Resume. Its map, inspection, activity, session
and Station puppeting panels use the same presentation and typed action
reducers as browser GM operators. See [Native Host](../concepts/native-host.md).

## Shared desktop workspace

`gui/gm-workspace-shell.js` composes the existing browser/native presenters
into the same six-panel desk, styled by `gui/gm-workspace.css`: roster, map,
Inspector, mission events, Comms Studio and activity. It moves existing nodes,
retaining their ids, listeners and live-region wiring. Roster rows select through
`gm-local-projection`; action shortcuts bring the existing forms into view.
The role and lethal-confirmation segments drive the existing private selectors
and confirmation profile. Scenario-authored role presets still filter panels.

The browser lobby's Ready, Force Start, GM admission prompt and manual save
controls move into the roster on the GM route, with readiness above the roster
and spawn palette. Native GM startup removes the host landing, scenario and
WASM-loading overlays because their browser dismissal code does not run on that
surface. This keeps Ready visible and clickable without covering the Station
Bar. The authentic Station iframe remains outside the desk
at 1280×720, with its existing console URL and command adapter. Inspector tabs
present the existing knowledge comparison, without adding a new projection.

After an action shortcut jumps the Inspector down to a control, a sticky
`#gm-inspector-back` button offers the way back to the selection card; it
hides again once the card is in view. The Objectives list narrows to the ship
the map has selected (`gm-objective-panel.js` `select`): rows whose recipients
are empty address every ship and stay listed, any non-ship selection lists
everything. Desk panels show a thin, always-visible scrollbar in both Chrome
and the native Ultralight pane, and the desk floors its type at 14px body /
12px labels with dim headings lifted to `--ink`. On the native pane a wheel
notch arrives in line units and is converted to `WHEEL_LINE_PIXELS` (60px) by
`PaneInput::scroll_from_wheel` in `src/native_host/panes/pane_thread.rs`; the
raw notch count was previously sent as pixels, scrolling one pixel per notch.

`gm_projection.rs` classifies a `planet`, `moon` or `star` tagged world entity
as `GmEntityKind::Celestial`, a blip drawn from its `[radar_appearance]`;
before that a planet fell through `world_kind` and never reached the GM map.

## Identity and roster

The fleet owner mints two separate identities for a GM:

- a stable public operator id such as `gm-1`, projected with only `name`,
  `connected`, and `ready`; and
- a private opaque reconnect capability retained by that operator's server
  page and presented on a later privileged reconnect.

Rendezvous peer ids and technical mesh slots are private transport details.
Neither they nor the reconnect capability enter the public roster. While the
Lobby topology is still mutable, a known disconnected GM can bind a new
connection to the same operator row without consuming a ship slot or displacing
a connected operator. Once frozen, that technical slot is part of Rust's
deterministic wait-set. A known frozen-session reconnect is admitted only when
the private capability names the exact disconnected public operator and the
frozen roster maps that operator to the exact departed technical slot. A live,
mismatched, or competing duplicate is refused without changing the roster.

Scenario-authored role presets are personal presentation choices, managed by
`gui/gm-role-presets.js` and read through `wasm_get_gm_role_presets`. An operator
can switch presets live; the built-in All is the default and fallback. The
selection persists with that operator's private reconnectable identity. The
current controller filters the map, inspector and activity panels and the
Pause/Resume quick actions. Presets never change action authority or enter
`GmOperator`, a public roster, a snapshot, or a digest.

## Private confirmation choices

`gui/gm-confirmation.js` owns the category/default catalogue, portable profile
adapter and one confirmation dialog. `gui/gm-confirmation-settings.js` renders
the matrix and profile import/export in Settings' Controls tab. These choices
remain in the operator profile, outside the roster, snapshot and digest.

Panels capture their intended target before opening the dialog; acceptance
enters their existing submission and result lifecycle. The damage panel's
preview reads the latest projected hull for that captured entity. It can change
while the dialog is open, but only normal admission determines the result.
Continuous Station control releases go through normal admission immediately
and retire any unsent held value for the same control, including while another
action's dialog is open.

## M2 recording automation

`tests/smoke/gm-m2.spec.js` drives Combat Test through the real GM controls and
keeps its normal initial export and GameOver autosave. The companion collector
observes submissions, confirmation decisions and crew traffic. The recording
is limited to GM actions with fixed initial crew seats and ratings, because
ordinary manual exports omit human-command history. The invocation and
artifact contract and local milestone evidence are in
`docs/acceptance/1316-m2-combat-test.md`. The native verifier checks the restored
origin before replaying requests, then compares the actual final results and
digest; collector evidence separately covers the absence of omitted crew input.

## First-time mid-session admission

An unknown GM joining a running fleet first receives only a private candidate
connection and reserved identity. Every existing host-class page displays the
request, and any admitted host or GM may accept or reject it; the star centre
delivers that decision but has no product-level veto. Rejection before
acceptance changes neither the public roster nor authoritative pause state.

Acceptance asks Rust's technical owner to assign exactly one synchronized pause
boundary. Its provisional topology is a private `GmJoinBootstrap`, which gives
world setup the topology it needs without installing `FleetRoster` or
`FleetLockstep`. The candidate receives the canonical #1117 run privately: its
world snapshot, complete admitted-command history, and snapshotted
`GmActionJournal`. It reports the restored digest through the typed
`GmJoinFrame` lane. Only a matching digest commits the transaction, installs
the public GM roster and lockstep wait-set, and makes the connection a live
fleet peer. An authenticated owner-carried peer loss received after snapshot
capture remains in private, non-authoritative `GmJoinPendingHostLoss`; it cannot
alter digest proof or admit a wait-set. Commit drains it once into
`PendingHostLoss` and marks that peer departed in the newly installed wait-set
before Resume. Successful restore engages the candidate's technical join hold
even when its private world began behind the pause boundary. An accepted
disconnect or authenticated transfer/restore failure is terminally refused to
every peer; if the disconnect callback races an existing Commit, Rust preserves
and re-emits the same committed proof instead of losing the transaction.
A stalled not-ready restore exhausts an authenticated, owner-sequenced
`RestoreBoundary` protocol clock; duplicate, stale, and render-only updates are
inert, while a candidate lost after commit follows the ordinary host-loss path.

Once acceptance has scheduled it, `GmJoinPauseHold` keeps the session paused
after either terminal Rust outcome. It is a protocol hold, not a fabricated GM
action, and is released only when a newly applied, attributed
`SetSessionPaused { active: false }` follows that terminal result. Consequently
admission can never resume as a side effect of roster or transport completion.

## Known-GM reconnect

A correct frozen-session capability automatically starts one bounded
`GmJoinKind::Reconnect` transaction; existing peers do not receive Accept or
Reject controls. This is the same owner-sequenced paused transfer as first-time
admission, not a second recovery protocol. The public GM row remains the
original disconnected row while the returning connection is private. Exact
retries replay the retained transaction, while a competing connection loses
deterministically.

The returning process is never a canonical snapshot source or an election
candidate, including the two-peer case where no majority can exist. The sole
survivor supplies the canonical #1118 record: world snapshot, complete admitted
command history, snapshotted GM journal, and owner boundary. Only the matching
restored digest commits. Commit preserves the operator id, name, technical slot,
capability, and fleet counters, then calls `LockstepSession::rejoin` at the
proven watermark instead of appending or admitting a peer. Corrupt or incomplete
records follow the existing rollback-safe restore seam and never publish the
returning connection.

The candidate Pause is applied before its fresh GameStart topology can spend a
private fixed tick. Pause arms the existing restore transaction but does not
enter `InProgress`: only after the canonical record passes its format, content,
and boot-identity gate does the private candidate stage that record's exact
authored-row/UUID map through `stage_resume_game_start_entity_uuids` and request
GameStart. Thus the returning process never mints a competing player-ship
identity before restore. On asteroid worlds, restore readiness compares the
standing authored field composition directly with the canonical saved window;
an exact match lets restore install that initialized window without advancing
the candidate, while a different composition still fails closed. The bounded
restore clock starts only after `GameStartEntityUuids` proves the deferred spawn
walked every authored row. If normal initialization has then still not produced
rebuildable rows, only the final authenticated owner-granted restore boundary
may enable the existing rebuild branch of the same rollback-safe restore seam.
A world that remains non-rebuildable reaches the deterministic timeout instead.

`GmJoinPauseHold` is technical protocol state, so snapshot capture and restore
preserve an empty `GmActionJournal` byte-for-byte instead of rewriting its
initial pause. When the first real typed grant is replicated, every peer adopts
its current paused state as the journal baseline before inserting the grant;
the owner and returning peer therefore digest the same durable GM history.

Commit keeps `GmJoinPauseHold` active. The returning GM becomes command-eligible
only after its canonical state and histories are installed, and the fleet still
requires a later attributed `SetSessionPaused { active: false }` to Resume.

## Authoritative projection

`GmRoster` is the Rust-side bounded, canonical full-replacement projection. The
browser fleet layer sends only public rows through `wasm_set_gm_roster`; Rust
validates unique ids, bounds names and row count, and broadcasts changes through
`Welcome`, `GmRosterChanged`, and the host lobby channel. Host and crew lobby
views render GMs in their own labelled presence group, outside Station,
Spectator, and crew-capacity calculations. Disconnect and reconnect retain the
operator id but clear readiness.

## Lobby readiness and force-start

Connected GM Operators participate in the same collective start policy as
connected non-Spectator crew. Every connected participant must be ready for an
automatic start, including a GM-only roster. Any connected GM may request a
force-start that bypasses readiness but never content, compatibility, preload,
or peer validation. The mesh authenticates the requesting connection and emits
one attributed, idempotent proposal. The proposal carries no apply tick. Rust on
the technical owner assigns the safe exact tick and puts the grant in its
authenticated `TickFrame`; every member adopts that canonical value. The
star-centre host does not gain a leader permission.

The explicit `?gm=1` page selects the production
`BootProfile::BrowserGameMaster` before `wasm_init`, independently of
WebDriver. It keeps the browser window, world ingest, fixed-tick simulation,
and fleet participant while installing no renderer and no `SelectedShipResource`
or local ship. The local `gm_entity` Host Channel carries an absolute,
UUID-sorted projection of facilitation-relevant local truth. Ships remain
player or NPC contacts, while `StaticPointDefence` is a structure. The typed
canonical `structure` tag also projects fixed authored structures that carry
neither Station nor infrastructure components. Structures, hazards, inert
Regions, asteroid fields, and selectable authored
asteroids use semantic kinds rather than exposing ECS components. Region,
hazard, and field footprints reuse the authored `RegionShape`; an asteroid
field is one aggregate Region, while lifecycle-streamed `Asteroid` rocks never
become individual contacts. A named/selectable asteroid authored in world
content is the only asteroid point contact. Each row contains stable
`EntityUuid`, position, String Table display ids, broad hull/infrastructure
condition, current Tactical target where applicable, and narrowed authored
radar appearance. It carries no Bevy entity id, component inventory or layer
identity. Hull-bearing rows also carry current and
maximum milli-HP and a per-System hull breakdown with authored Station
ownership, which the direct-effect panel uses for scope selection and preview. The Host
Channel dispatcher exempts this strict DTO from recursive localisation, keeping
those identities and display ids raw until the map or inspector presents them.

`gui/gm-local-projection.js` validates and narrows that DTO, partitions point
contacts from geometry Regions, then adapts both to the shared
`ph-navigation-map` in local inspect mode. Selection is keyed only by stable
UUID. An absolute refresh updates the linked inspector without dropping
selection; removal clears both map selection and stale inspector detail, and a
later same-UUID reappearance does not resurrect selection. At overlap a point
wins over a Region, then stable UUID breaks ties. Touch and keyboard traverse
selectable Regions as well as contacts only in inspect mode; Navigation's
interaction contract is unchanged. Semantic marker/outline patterns, a
selected outline, a destroyed cross, text legend, and textual inspector status
make kind and condition independent of colour. The inspector hides its hull
meter for kinds without hull applicability. The projection is not a
`ServerMessage`, `MeshFrame`, or `SimOutbox`; peer state transfer remains the
snapshot-recovery path.

The sibling local `gm_activity` Host Channel is the one absolute oldest-first
view of one bounded presentation ring. Every row uses the common
`{ tick, category, ships, links, detail }` contract. Its complete M1 category
order is Damage, Destruction, Objective, Trigger, Red Alert, Connection, then
GM Action; category-specific stable keys order simultaneous facts and equal
facts remain separate occurrences. The authored
`global.gm_activity_history_depth` sets the capacity and defaults to 128.
Capacity changes, new rows, and Lobby reset publish a replacement; an unchanged
frame does not.

Fixed facts come from unconditional source seams before `SimTick` advances.
Damage and destruction retain their existing balance facts; Red Alert uses
actual `RedAlertChanged` edges. Objective add/complete/fail mutations emit only
when `ObjectiveManager` really transitions, including the shared dispatcher and
independent Helm-AI completion path. An actual trigger evaluation emits
its authored id or the stable `script_path::function` fallback, never a runtime
vector index. Objective removal during layer unload and repeated idempotent
mutations make no feed noise. Fictional consequences caused by a GM remain
ordinary world-category rows.

FixedLast samples public connection changes at their source tick and PostUpdate
publishes them, with the same PostUpdate projection supplying the paused-frame
fallback. In a deterministic fleet, crew presence comes from the frozen
crewing in the replicated `FleetRoster` and joins its private slot internally
to the actual `FleetSlotOf` Ship UUID/name. Host loss empties that crewing on
the agreed fixed tick, so every peer derives the same ship-scoped disconnected
edge; restoring the cohort derives the matching connected edge. A standalone
App falls back to its local `Sessions`. GM presence reuses
`GmRoster` public identity. No row copies session tokens, rendezvous peer ids,
mesh slots, or reconnect capabilities. Attributed GM action results come from the
terminal `GmActionLog` plus local refusals and deduplicate by
`(operator, correlation)` while retaining the canonical action order. Force
Start observes the existing typed grant result before the browser bridge drains
it. Applied, No-op, Refused, and the exact reason remain explicit; restore or
join rebases the observed cursor so old terminal rows do not replay.

A presentation-only UUID-to-name-and-kind directory is sampled before combat,
then pruned to identities that are live or referenced by bounded history.
Semantic ship scope comes only from an actual `Ship` marker, never from
`VictimKind` or the presence of an `EntityUuid`. Category and ship filters are a
strict AND; global rows appear only under All ships, Clear resets both filters,
and removal of the selected ship resets that filter to All. Links stay separate
from filtering and call the same stable-UUID map selection seam. A removed or
racing target remains readable but disabled and cannot clear another selection.
String Table ids resolve only at render time while literal mod-authored names
remain literal. M1 carries no attention score or ranking. The feed remains
absent from `ServerMessage`, mesh frames, snapshots, digests, replay, and peer
transport.

## Typed session actions

`SetSessionPaused { active }` controls the session pause. The page submits
an absolute value plus the public operator id and a bounded durable
`GmActionId`; it never calls the local/debug toggle. Rust binds that identity to
the authenticated GM slot in the frozen private `FleetRoster`. Any GM may send
a proposal, but only the frozen technical owner assigns its exact logical
boundary and canonical `(apply_tick, GmActionOrder { sequence, origin })` key.
The proposal's `from` remains the requester and a decision's `sequenced_by`
names the technical owner; neither field grants operator precedence. Exact
retransmission is inert and a new id that requests the current value records an
explicit No-op. JavaScript relays the opaque Proposal, Granted, and Refused
frames without interpreting the Rust-owned action protocol.

The complete bounded `GmActionJournal` is snapshotted as authoritative input
and the whole journal remains the operator-scoped idempotency record. Current
state and digest fold `initial_paused`, the exact `applied_prefix()`, its
persisted apply-boundary outcomes, and `SimulationPaused`; future grants
received early are not current state.
`apply_due_actions` advances the durable `applied_grants` reducer frontier in
`PreUpdate`, so Resume remains consumable while Pause has starved `FixedUpdate`.
It revalidates live authority in canonical action order and stores the actual
Applied/No-op/Refused result beside each applied grant. A correlated Station
command alone records an internal Pending boundary after admission, then
replaces it exactly once with the ordinary System consumer's terminal Applied
or Refused feedback; Pending is neither projected nor a terminal idempotency
fact. Sequencing therefore
must preserve a takeover when a human holder reconnects, while refusing a
same-tick command after an earlier release removed its authority.
The existing lockstep gate still owns the combined virtual-time decision, so a
GM resume cannot release a peer, recovery, or model-readiness hold.

Replay installs a one-shot `ReplayGmSeed` after the ordinary run-start reset.
It loads only the source run's applied prefix into an empty live journal with a
zero frontier, then re-applies those recorded boundaries through the production
reducer; an unapplied suffix never extends replay beyond the artifact's final
tick. The GM page receives only an absolute local `gm_session` Host Channel
projection and does not optimistically change the displayed pause state.

## Authored events and directed world actions

`src/gm_event.rs` resolves authored event controls and publishes the absolute,
page-local `gm_mission` projection for `gui/gm-mission-panel.js`. Scripted
`gm_event(id, label, handler)` and automatic registrations with
`.gm_controls(id, label)` share one control set and ordinary trigger lifecycle.
`.pauseable()` adds persistent per-event Pause/Resume; `.skip()` arms the next
qualifying automatic occurrence. Fire substitutes for the automatic event
match and bypasses per-event Pause, but still obeys the authored `.when(...)`
predicate, once/repeat latch and cooldown. A false predicate or unelapsed
cooldown retains the Fire arm until it can run the ordinary handler; a spent
once-only event cannot fire again. Neither the manual Fire pass nor Pause
consumes an armed Skip: only a qualifying automatic occurrence consumes it
while the event remains present. `FireGmEvent`, `SetEventPaused`, and `ArmGmEventSkip` use the same
attributed canonical action journal as session controls.

`src/gm_attention.rs` lists the beats whose Fire would land right now in the
peer-local attention queue (`gui/gm-attention-panel.js`), reading that same
control state plus `world::content::manual_fire_would_land` — the predicate
`fire_manual_trigger` itself applies — so nothing is evaluated twice and no
handler runs. `.attention_band(...)` on the control set chooses the row's band.
Activating a beat row focuses that beat's existing mission-panel levers
(`focusEvent`); it never fires anything.

`src/gm_quiet.rs` adds the quiet-time advisory to that same queue: one
`Background` row, never escalated by age, after an authored interval of
SIMULATION seconds with no meaningful crew activity. `GmCrewActivity` is the
peer-local clock and `observe_crew_activity` the adapter, reading three sources
the simulation already keeps — an `ActionFeedback` the owning System settled
`Applied` (which is how a completed Comms response counts), a control a seat
worked that has no terminal result to settle (read from `AdmittedCommands`, with
AI decisions excluded by their `ai:` token), and
`BalanceEvent::ObjectiveChanged`. `[gm_attention] quiet_time_secs` and
`quiet_time_disabled` are independent authored fields on the same table the
idle-NPC advisory reads; a non-positive or non-finite interval fails the world
load. The row names only the interval — there is no
keystroke telemetry anywhere in the path — and carries no target, so the desk
draws it with Snooze and no Open.

`src/gm_objective.rs` resolves `[[gm_objective_palette]]` entries and handles
`ObjectiveAction` activation, completion and failure through the ordinary
Objective lifecycle. Authored recipient ships are separate from subject
`targets`; each Objective retains one status for its whole recipient scope.
Crew and AI consumers filter by their ship UUID. The mission projection carries
the palette, current records and attributed results to
`gui/gm-objective-panel.js`, whose preview names the intended ships and closes
when the selected record or scope changes. Snapshot and digest include the
complete ordered records, including terminal statuses and recipient scope.

`ApplyDirectEffect` carries an Entity UUID, whole-Entity/Station/System scope,
damage or healing, and an amount in milli-HP. `src/gm_effect.rs` resolves and
arms the effect at the canonical action boundary; the ordinary Damage schedule
applies it through the hull path and normal destruction lifecycle. It bypasses
shields, clamps to the selected scope and records discarded overflow.
`gui/gm-direct-effect-panel.js` reads the selected inspector entity's live
hull totals and authored ownership; `gui/gm-effect-scope.js` supplies the
shared browser scope vocabulary.

`src/gm_spawn.rs` publishes the local `gm_spawn` palette projection for
`gui/gm-spawn-panel.js`. Scenario/mod `[[gm_palette]]` entries bind preloaded
templates and closed authored variants. `SpawnPaletteEntity` carries only
palette/variant ids and resolved position/heading. The shared map placement
gesture and keyboard controls supply that placement; canonical pending spawns
enter ordinary scripted `SpawnEntity` dispatch and deterministic UUID minting.
The activity feed presents event levers, Objective changes, scoped effects,
palette placements and Comms transmissions as attributed GM Action details;
their fictional consequences remain ordinary world-category rows. Comms action
results and refusals retain their canonical recipients, so filtering by ship
still finds a transmission after its recipient leaves or its grant is pruned.

For intended design, see
[`gm-console-t2.yaml`](../../pasm/spec/design/gm-console-t2.yaml).

## Truth and crew knowledge

`gui/gm-knowledge-compare.js` compares a separately selected ship's knowledge
with the GM view. Truth uses `gm_entity`; the selected ship's `gm_station`
replica passes through the same fold and Sensors/Comms builders used by
authentic Station consoles. Contacts can differ because of sensor range and
tag filtering. Both Objective comparison columns use the selected ship's
recipient-filtered list and therefore show equality. Comms likewise compares
the selected ship's recipient-filtered blackboard with the ordinary console
builder's view of that same blackboard. The comparison excludes Station-private hull and
blackboard detail, which remains accessible through Station puppeting.

## Authentic Station puppeting

`SetStationPuppet` and `IssueStationCommand` extend the same attributed,
idempotent `GmActionJournal`. A takeover may begin only on an authored Station
on a compatible ship, including a human-held player Station on any live rating. `StationPuppets` stores
the canonical `(ship, station, sorted equal-operator set)` membership; it is
captured in snapshots and folded into the simulation digest alongside the
applied GM journal. A Station command is decoded and passes the shared
payload-aware System authority/availability policy at the action boundary;
only then is its source-stripped payload queued. That already-admitted pending
queue is captured and folded, so recovery between PreUpdate acceptance and
FixedUpdate delivery cannot lose a command or settle it before its real System
consumer does. The transient response route is reconstructed from that queue
after restore; it carries only canonical order plus the iframe's opaque
correlation, never GM authority. Activity remains a presentation projection
and is cleared on restore.

The rendererless GM reads each compatible ship's exact `StationConfig.console`
URL plus its topology, tagged blackboards, ratings, control sources,
membership, activity, pose, waypoint, objectives, hull and absolute live world
entity lane over the local-only `gm_station` Host Channel. It also receives the
exact complete `ShipClientConfig` built by the ordinary `Welcome` projector,
not a GM-owned subset: authored radar ranges and filters, weapon arcs, hostile
arc colour, tutorials, hull identity and assist gaps therefore reach the same
Helm, Sensors and Navigation builders. Static entity facts come from
`WorldResource`; live position, hull and shield fields share the ordinary
`SimState` producers. The browser folds those raw facts through `ClientSimState`
and the shared radar-region builder before calling the ordinary
`buildConsoleState`, so spatial Stations see the same complete replica as a
player rather than a GM-only map model. The scrollable GM surface mounts the
authored URL in an iframe that remains reachable at the standard 1280×720 host
viewport. Messages from that exact iframe pass through the existing action map;
the iframe's opaque correlation is preserved on the typed GM action and the
canonical terminal result settles that same iframe's ordinary feedback
lifecycle. Admission-time refusals settle immediately; accepted correlated
commands remain Pending until their ordinary consumer reports Applied or
Refused, and duplicate feedback cannot settle them again. A browser ingress
refusal settles the originating iframe immediately as Refused without claiming
a gameplay result. The parent-side pending-feedback map shares the ordinary
bounded feedback capacity and timeout: capacity eviction or a missing result is
presented as TimedOut, removed deterministically, and cannot be mutated by a
late canonical result. Only the resulting `ControlSystem` command is wrapped; no
GM-specific copy of a Captain, Helm, or other Station interface exists.

At the System Admission boundary, a viewscreen command derives authority from
the payload's authored source System (for example Radar from Helm), not from
the viewscreen transport target. Canonical GM commands are ordered after any
ordinary admitted human input for that tick and by their owner-assigned GM
order. The sidecar activity entry retains operator attribution for GM and crew
presentation, while the downstream `AdmittedCommand` has no response token or
actor identity. Simulation systems therefore do not branch on human versus GM.

Takeover makes only the selected Station's systems Human-controlled so its
Backfill AI emitters stop; it never changes `Player.station`. The crew's
`SimState` identifies active operators and the latest admitted activity, and
the shared console runtime renders that truth in the affected authentic
interface. Releasing the last operator reapplies the Station's ordinary live
rating, including a mixed human/AI rating changed during takeover, so a present
holder retains their human Systems and an absent holder returns to Backfill and the original token can
still reconnect to the same Station.

At an agreed GM host-loss boundary, that operator is removed from every
takeover and its activity projection. Equal surviving operators retain their
membership; when the departed GM was last, the Station's ordinary rating is
reapplied at that same boundary. The frozen GM binding remains available to an
authenticated recovered peer, which may take a compatible Station again. Each
recovery advances a canonical slot generation recorded in the journal: grants
retain the generation in which they were sequenced, so pre-loss work remains
durably Refused even after `rejoin` clears the transient departed flag, while
genuinely post-recovery work applies. Both adjacent generations are eligible
exactly on the recovery boundary to preserve established deterministic
ordering. These generation facts are snapshotted, folded and replayed. A grant
exactly on the still-unapplied loss boundary keeps the normal action-before-
loss schedule order; equal surviving GMs remain members when cleanup follows.

## NPC capability and lifecycle

NPC capability is checked by `src/gm_puppet/capability.rs` at offer, ingress and
canonical application. The initial audited NPC interface is battleship Helm;
Captain, Power and mixed interfaces with incomplete per-ship producers remain
excluded. The common NPC spawn stores the normal `ShipClientConfig` projection
from its resolved instance config, including overrides, and restore rebuilds it.
Active membership promotes the NPC through the existing full-fidelity bundle
before Admission and holds it against LOD demotion. Last release restores the
live rating and ordinary LOD policy. Post-Damage cleanup removes memberships
whose Ship or Station disappeared. The browser replaces the full iframe context
when its Ship/Station key changes, so stale messages cannot control a replacement.

## Safe entity removal

`src/gm_despawn.rs` owns the shared apply-boundary policy and normal scripted-removal cleanup. Authored `gm_removable` tags opt eligible NPCs, structures and runtime hazards in; fleet hulls and foundational world geometry remain protected. `DespawnEntity` queues a stable UUID on `WorldContentRuntime` for the ordinary DestroyEntity trigger cascade. Snapshot format 27 preserves the pending queue, and `gm_entity` carries permission previews and attributed removal results to `gui/gm-despawn-panel.js`. Confirmation is tied to the selected UUID and invalidated by selection or permission changes. Live references clear while historical destruction predicates and Comms messages remain readable.

## Faction relations and undo

`src/gm_faction.rs` is the narrowly typed, attributed adapter over faction hostility: `SetFactionHostility` names two authored faction reference names, resolves them against the live `FactionRegistry` at the apply tick and runs the same `add_enemy`/`remove_enemy` calls the `add_faction_enemy` trigger action uses. `GmFactionOverrides` records which ordered pairs a GM moved and what each held before any GM touched it, so a snapshot restores those decisions and the digest folds them once — and only once — a GM has actually moved a relation. Withdrawing a hostility arms `revalidate_gm_faction_locks`, which drops AI tactical locks immediately before `ai_target_selection`. `gui/gm-faction-panel.js` reads the authored roster from `GmSessionProjection::factions`; there is no free-text or UUID entry.

`GmAction::UndoGmAction` reverses one earlier action of a reversible family. An `Applied` action of such a family records the exact affected field with its before and after values on `LoggedGmAction::affected`; the inverse carries the original's public identity and those recorded facts, and `undo_precheck` plus a live per-field check at the apply tick refuse an unknown original, an unreversible one, a stale reading, an already-applied inverse and an intervening change to that one field — while allowing every unrelated change. Both operators sit in the one journal: the undoing GM as the inverse row's operator, the original on `undo_of`. `gui/gm-journal-panel.js` offers the Undo control only where the canonical journal says one can work, and `gui/gm-inverse-preview.js` renders the before/after, technical and already-witnessed sentences under every confirmation policy. Snapshot format 34 carries the faction overrides.

## Related

- [Session](./session.md)
- [Player](./player.md)
- [Networking](../concepts/networking.md)
- [Server Lobby UI](../concepts/server-lobby-ui.md)
- [Stations](../concepts/stations.md)
