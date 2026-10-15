//! Artifact.
use super::*;

// ── The three version dimensions ─────────────────────────────────────────────

/// The payload's byte layout, bumped by hand whenever [`PhoenixSnapshot`]'s
/// shape changes in a way an older save cannot be read as.
///
/// This is a *phoenix* constant that `vellum_save::Versions` carries, not a
/// phoenix version field: the comparison, the ordering of the three checks, and
/// the refusal all belong to `Versions::check`.
///
/// `2` — issue #864 added [`PhoenixSnapshot::scenario`]. Every new field
/// carries `#[serde(default)]`, so a format-1 save still *parses*; the bump is
/// here because parsing it is exactly the wrong outcome. A format-1 payload
/// carries no trigger fired-state, no mission clock and no pending script
/// callbacks, so restoring one would silently re-arm every scenario trigger the
/// run had already spent and drop every deferred callback it was waiting on —
/// the same class of silent gap [`RestoreGap`] exists to refuse out loud. A
/// save this build cannot honour is refused by `Versions::check`, which names
/// the dimension.
///
/// `3` — issue #984's S8 added [`PhoenixSnapshot::comms`], and the same
/// reasoning applies unchanged. Every new field defaults, so a format-2 save
/// parses, and a format-2 save of a comms-quiet world would even restore
/// *correctly* — which is precisely why the version cannot be left at 2. The
/// payload has no way to distinguish "this world had no conversation open" from
/// "this save predates conversations being recorded", and the second one
/// restores a scenario mid-thread into a world with an empty inbox, no dialogue
/// to answer, and every `open_comms` request the run had queued discarded. That
/// is silently wrong in exactly the way a re-armed trigger latch was, so it is
/// refused on the same dimension.
///
/// `4` - issue #985 (Rhai M7) deleted the declarative `[[comms]]` front-end, and
/// [`CommsState`] lost the three fields that only described it: `template_fired`
/// (a `[[comms]]` template's single-shot latch), `uncarried_follow_ups` (the
/// `PendingFollowUp` queue's placeholder ids) and `uncarried_dialogues` (the
/// count of nodes whose responses carried `TriggerAction`s). [`DialogueState`]
/// also lost `speaker`, and its `script` became required rather than optional -
/// every live dialogue is a scripted one now.
///
/// The bump is the same argument the previous two made, run the other way. A
/// format-3 save still *parses* here: serde ignores the three retired fields and
/// defaults the rest. That is precisely the wrong outcome. A format-3 payload
/// recorded mid-conversation on a declarative thread carries a spent template
/// latch this build has nowhere to put and a `script: None` dialogue this build
/// cannot answer, so restoring it would re-arm a broadcast that had already
/// fired and seat a message no `on_pick` exists for. Nothing in the payload
/// distinguishes that save from one whose world simply had no declarative
/// content, so both are refused by `Versions::check`, which names the dimension.
///
/// `5` — issue #1024 added [`ScenarioState::deadlines`], and this is the format-2
/// argument run again on a new vocabulary. Every new field carries
/// `#[serde(default)]`, so a format-4 save still parses — and a format-4 save of
/// a deadline-free world would restore correctly, which is exactly why the
/// constant cannot be left at 4. A format-4 payload carries no deadline record at
/// all, so restoring one into a world that authors `[[deadline]]` blocks re-arms
/// every deadline the run had cancelled, rewinds every one it had slipped back to
/// its authored due time, and un-fires the ones already spent — the same class of
/// silent re-arming that moved this constant to 2 for trigger latches. Nothing in
/// the payload distinguishes that save from one whose world simply authored no
/// deadlines, so both are refused by `Versions::check`, which names the dimension.
///
/// `6` — issue #1029 added [`ScenarioState::commitments`], and the argument is
/// the same one a third time with one twist that makes it sharper. A promise is
/// a RUNTIME artifact: no world file declares one, so nothing in the content
/// digest changes when a run happens to make one, and a format-5 payload of a
/// run that had given its word is byte-indistinguishable from a format-5 payload
/// of a run that had not. Restoring the first into a #1029 build resumes a
/// captain who has promised nothing — which is not merely missing state, it is a
/// *plausible* state, and the ledger's own "unknown" answer is what a scenario
/// guards a duplicate promise with. So the resumed run would re-offer a word
/// already given, and every campaign flag that promise was going to write would
/// be written twice or not at all. Nothing in the payload distinguishes that
/// save from one whose run simply had not reached the negotiation yet, so both
/// are refused by `Versions::check`, which names the dimension.
///
/// `7` — issue #1031 added [`ScenarioState::evidence`], and it is the #1029
/// argument with the twist turned one notch further. A finding is a runtime
/// artifact for the same reason a promise is — no world file declares one, so
/// the content digest is silent about it — but where a missing promise resumes a
/// captain who has said nothing, a missing FINDING resumes a crew who never
/// learned something, and the whole Thin Margin arc is about what the crew know
/// and how they came to know it. A format-6 payload of a run that had scanned the
/// skyhook is byte-indistinguishable from one that had not; restoring the first
/// into a #1031 build hands the crew back a blank intelligence file, and because
/// the store deduplicates on `(subject, provenance, text)` rather than on a
/// counter, a re-scan would re-stamp the finding at the resumed tick and quietly
/// rewrite when they found out. Nothing in the payload distinguishes that save
/// from one whose run had simply not scanned anything yet, so both are refused by
/// `Versions::check`, which names the dimension.
///
/// `8` — issue #1035 added [`ScenarioState::workforce`], and this is #1024's
/// argument rather than #1029's: a strike IS declared in the world file, so it
/// might look as though the content digest could stand in. It cannot, and the
/// reason is one line in [`crate::world::config`]: `RawWorld` sets no
/// `deny_unknown_fields`. A build that predates this vocabulary therefore loads
/// a world authoring `[[workforce]]` perfectly happily, drops the table on the
/// floor, and writes a format-7 save of the SAME files with the SAME content
/// digest — a save that says nothing about a dispute the world is entirely
/// about. Restoring it here arms an empty register, so every structure the
/// strike was gating reads as worked: the depot that was refusing transfers
/// takes one, and the repair that was running unassisted comes back at full
/// rate. Nothing in the payload distinguishes that save from one whose world
/// simply declared no sides, so both are refused by `Versions::check`, which
/// names the dimension.
///
/// `9` — issue #1041 added `EntityState::weapons_hold` (removed again at 20,
/// see below), and it is the simplest of these arguments: the field is on every
/// ship row, so the payload shape moved, and what it records is an ORDER. A
/// format-8 save of a ship whose captain had called a weapons hold is
/// byte-indistinguishable from one whose captain had not; restoring the first
/// into a #1041 build resumes a crew who had chosen restraint with their guns
/// live, on the very tick the scenario is weighing what they chose. The alert
/// beside it has been persisted from the beginning for exactly this reason, and
/// half a firing posture is not a posture. Nothing in the payload distinguishes
/// that save from one taken with the lever never pulled, so both are refused by
/// `Versions::check`, which names the dimension.
///
/// `10` — issue #863 added [`EntityState::spawn`] and
/// [`ScenarioState::name_to_uuid`], and this is the argument #1035 made turned
/// all the way around: not "the content digest cannot see this", but "the
/// content digest is *identical* and the world is still different".
///
/// A format-9 save of a Combat Test run carries every wave NPC's hull, helm,
/// weapons and blackboards — and nothing at all about where those ships came
/// from. That was survivable while a restore was only ever an overwrite, because
/// a fresh boot of the same scenario re-ran the same script from the same seed
/// and re-minted the same uuids, so the roster the capture named was always
/// standing by the time the restore ran. What it was silently relying on is a
/// *replay*, and a resumed browser session is precisely where the replay does
/// not happen: the fresh app boots with nobody at the consoles, so a wave
/// released by a player's action is a wave the bootstrap never releases, and
/// every ship in it comes back as a [`RestoreGap::MissingEntity`] — or, worse,
/// as a `ready_to_restore` that never becomes true and a resume that simply
/// hangs. Two saves of the same scenario, the same files, the same content
/// digest, and one of them describes a raid this build cannot rebuild.
///
/// So the payload now records what each mid-run spawn was made from, and the
/// restore builds it. Both new fields carry `#[serde(default)]`, so a format-9
/// save still parses — and a format-9 save of a world that never spawns anything
/// at runtime would even restore correctly, which is exactly why the constant
/// cannot be left at 9. Nothing in the payload distinguishes that save from one
/// whose run had a whole raid on the board, so both are refused by
/// `Versions::check`, which names the dimension.
///
/// `11` — issues #1107–#1109 added [`EntityState::station_stances`], and it is
/// #1041's argument on a fresh lever. The per-ship Command stance map
/// (`ShipStationStances`) IS folded into the sim digest by
/// `sim_digest::fold_station_stances_namespace`, but nothing in the payload
/// carried it, so a resume dropped every stance an AI, human or objective order
/// had put in force and folded a different number than the capture: a destroyer
/// that had chosen its `ai_engaged` stance at red alert comes back undirected,
/// on the very tick the fold counts it. The field carries `#[serde(default)]`,
/// so a format-10 save still parses — and a format-10 save of a run in which no
/// stance was ever selected would even restore correctly, which is exactly why
/// the constant cannot be left at 10. Nothing in the payload distinguishes that
/// save from one whose crew had directed a Station, so both are refused by
/// `Versions::check`, which names the dimension.
/// `12` — issue #1242 added [`EntityState::motion_plan`], and it is
/// `pass_surface`'s argument on the sibling that fix left behind.
/// `HelmMotionPlan` is republished wholesale by `helm_motion_planner` every AI
/// tick, which reads like a derivation nothing need store — but the AI cadence is
/// HALF the sim cadence, so a capture taken on one of the ticks in between leaves
/// a restored world holding a default plan while the live one still carries the
/// last one planned. Measured on the duel: the live cruiser wanted
/// `desired_facing_local (0.363, 0, -0.932)` and the resumed one `(0, 0, -1)` —
/// no turn at all — and two frames later its steering had gone to zero while the
/// live ship kept turning. The field carries `#[serde(default)]`, so a format-11
/// save still parses — and a format-11 save captured on an AI tick, or of a run
/// whose ships were all stationary, would even restore correctly, which is
/// exactly why the constant cannot be left at 11. Nothing in the payload
/// distinguishes that save from one captured mid-manoeuvre, so both are refused
/// by `Versions::check`, which names the dimension.
///
/// Format 12 is defined by the COMBINED #1242 + #929 shape, and there are THREE
/// contributors, not two: `ai_world` above; `WeaponState.beams` as a 5-tuple
/// (the #929 drawn-cooldown column); and the #929 helm-doctrine keys in
/// `PolicyState.memory` — `broadside_flip`, `broadside_flip_arc` and
/// `broadside_flip_since`, the weak-broadside latch's state, identity and clock.
///
/// The third contributor needs no version dimension of its own and is called out
/// because a reader counting struct changes would miss it. `PolicyState.memory`
/// is already a `name → value` map that the payload carries wholesale, so the
/// keys arrived in a shape every existing format could parse: an OLDER save
/// simply has no `broadside_flip` key, and a hull restored from one reads the
/// slot absent, which is `0.0`, which is "not flipped". Benign — the ring
/// resumes unmirrored and re-latches within a tick of the arc reading that
/// justified it. A save with the keys restored into a build that does not know
/// them is the same story in reverse: unread keys, unchanged behaviour.
///
/// What it is NOT is digest-covered. Policy memory is not folded into
/// `world_digest`, so a payload that silently dropped these keys would restore a
/// hull circling the wrong way with every digest still matching — the #1242
/// failure mode exactly, and the reason
/// `snapshot_resume::a_mid_flip_broadside_survives_a_save_and_restore` asserts
/// the round trip on the payload directly instead of trusting the digest gates.
///
/// The widenings landed in the same unpushed batch, so no build that wrote a
/// 4-tuple format-12 save ever existed outside that merge window; a stray dev
/// save from the window fails RON parse rather than the format check, which is
/// acceptable for a payload no release wrote.
///
/// `13` — issue #1045 made supporting-world ownership authoritative on pending
/// script callbacks, named deadline rows, queued scripted comms opens and live
/// scripted dialogues. It also widened [`LayerFlags`] into the ordered active
/// layer topology: the loader that owns each layer and the activation order that
/// determines scripted-trigger indices. Each new ownership field has a serde
/// default so a format-12 save still parses; accepting it would silently restore
/// layer-owned operations at base scope and replay the fresh bootstrap's layer
/// composition instead of the captured run's. A callback could mutate the wrong
/// flag store, two layers' same-local-id deadlines could cross-talk, or an index
/// could restore fired-state onto another layer's handler. Nothing in an older
/// payload distinguishes those cases from a genuinely base-owned, startup-loaded
/// layer, so the format dimension refuses it rather than guessing ownership.
///
/// `14` — issue #1258 made the Shields AI continuation seam active in ordinary
/// Backfill combat: [`ShieldsAiState`]'s recent-damage window and one-shot
/// threat-bearing inbox, [`SensorsThreatMemoryState`]'s producer-side debounce
/// memory, [`CoordinationQueueState`]'s attached in-flight deliveries, and the
/// unread [`CoordinationEnqueueState`] suffix ahead of that queue. All are
/// runtime state, so the content digest cannot
/// distinguish a format-13 save taken with empty state from one taken while a
/// value was live. Accepting the older payload would therefore resume a ship
/// with a plausible but wrong focus decision — including a fresh Sensors state
/// re-emitting an already-delivered bearing after the Coordination delay. A
/// bootstrap may also have queued—or left unread—that duplicate before restore
/// applies the producer memory, so both attached and staged destinations are
/// replaced rather than merged. Shields damage records carry their exact
/// authoritative `recorded_tick`, while each queued message carries the first
/// future FIXED STEP on which it becomes due rather than a raw `Time`-relative
/// timestamp. Keeping both boundaries integral avoids moving an expiry or
/// delivery by one step through an `f32` projection.
///
/// The same unshipped 13→14 boundary carries the other fixture-reachable
/// producer memories whose fresh-bootstrap values can manufacture a later edge:
/// intent narration's complete station map and generation, recent combat ages,
/// the high-fidelity frequency-hint timer, NPC frequency-match timers, exact
/// ship phaser frequency, the global phaser firing mode, Tactical's one-tick
/// frequency-hint inbox, exact
/// damage-tier/HP baselines, Power's brownout debounce, and Shields'
/// down/restore notification debounce. Each component or resource's presence
/// is represented as `Some`, including a wholly default value, because omission
/// and an authoritative empty frontier are different statements during restore.
/// Format 14 also carries `Time<Fixed>`'s exact nanosecond overstep. Removing
/// `ThrustInput` from restore readiness lets a Low bootstrap become ready on a
/// different rendered frame, and without the accumulator the first resumed
/// `App::update()` can run zero fixed steps while the live world runs one.
/// The same boundary explicitly carries every ship's High/Low fidelity tier
/// and transition age, the canonical bundle's outside drive runtime, the
/// damage-offline control gate, and the broadcast projection registry. These
/// are one lifecycle repair: changing readiness exposes bootstrap states that
/// used to settle before restore, and each is read before its ordinary repair
/// system can rebuild it on the first continuation tick. Navigation's waypoint
/// and clearance frontier and Weapons' global/default replacement state also
/// landed before any format-14 artifact shipped, so they share this boundary.
///
/// Format 15 carries the boot identity that must exist before a snapshot can be
/// restored: the selected local hull, the frozen fleet roster, and every
/// successfully spawned authored `GameStart` entity's `EntityUuid`. A scenario
/// may offer several hulls whose authored component sets differ while retaining
/// the same scenario content digest. A fresh lobby may also reach `GameStart` on
/// a different tick, so merely rebuilding the same roster can mint different
/// UUIDs for the player ships and for any authored GameStart NPCs every captured
/// row names. Without this field a fresh session can boot one hull (or the right
/// hull and roster under new identities), pass every version check, and then
/// apply a snapshot captured from another. There is no honest migration for a
/// format-14 artifact because the missing choices cannot be inferred from world
/// state, so the format gate refuses it rather than guessing.
///
/// Format 16 carries the typed GM action frontier: the absolute session pause
/// state and the complete bounded action journal that supplies ordering,
/// idempotency, attribution, and future scheduled actions. A format-15 record
/// cannot distinguish an unpaused run with no GM actions from a paused run (or
/// one with an already-received future action), so defaulting those fields would
/// silently resume or forget authoritative work. The format gate refuses that
/// ambiguity rather than inventing a migration.
///
/// Format 17 carries the authoritative Station takeover membership reduced
/// from that action frontier. A format-16 record can contain a takeover grant
/// but no reliable statement that its applied side effect crossed the snapshot
/// boundary, so restore refuses to guess between active Backfill and puppeting.
///
/// Format 18 carries actual apply-boundary GM outcomes and the already-admitted
/// Station command queue. A format-17 capture taken after PreUpdate but before
/// the command's FixedUpdate delivery can otherwise say Applied while losing
/// the only source-stripped payload that continues that effect.
///
/// Format 19 carries WHICH repair team is abroad on a field repair (issue
/// #1386). Until now the external claim was a count and the team it meant was
/// re-derived by truncating the idle list, so a save recorded only the target;
/// a seat can choose the team now, and the claim names it. There is no honest
/// migration for a format-18 record holding a live claim: the truncation rule
/// that used to name the team read the WHOLE idle list, which the record does
/// not carry, and defaulting the index to 0 would resume the wrong slot as
/// abroad while leaving the real one double-booked — a divergence the digest
/// would then report as corruption of a save that is perfectly intact. The
/// format gate refuses it rather than guessing, which is why
/// the format-19 external repair team's stored index was mandatory
/// with no serde default. A format-18 save with NO live claim is refused by the
/// same gate, on the same rule every other bump on this ladder keeps: the
/// version is a statement about the rules the save was written under, not about
/// which of its rows happen to be empty.
///
/// `20` — issue #1398 REMOVED `EntityState::weapons_hold`, and it is the first
/// rung on this ladder that subtracts rather than adds. The captain's
/// `ShipWeaponsHold` is gone: restraint is a POWER order now, a `weapons` group
/// commanded to level 0, and that already rides `EntityState::power` (format 14,
/// reinstated at the group's own authored floor since #1395). So a format-20
/// capture carries the posture it always did — in the one field that now
/// expresses it.
///
/// # The decode stays tolerant, and the record is still refused
///
/// Nothing here denies unknown fields, so a format-19 record still PARSES: its
/// `weapons_hold` entries are ignored by the derived visitor and the reader gets
/// a `LoadRefusal::Moved(Format)` that names the dimension, rather than a RON
/// error about an unexpected field. That is what "tolerant decode" buys, and it
/// is the whole of what it buys.
///
/// It does not make the record loadable, and the reason is #1041's own argument
/// read forwards. A format-19 save records the hold in a field this build cannot
/// honour, and the only faithful translation — taking that ship's `weapons`
/// group cold — is one the hull may refuse: `min_level` is authored per hull and
/// `alliance_battleship`, `alliance_courier` and `alliance_tender` all say `1`.
/// Accepting the record would therefore resume a crew who had chosen restraint
/// with live guns on exactly the hulls that cannot express restraint any other
/// way. Nothing in the payload distinguishes that save from one taken with the
/// lever never pulled, so both are refused by `Versions::check`, which names the
/// dimension — the same answer every rung above gives.
///
/// Format 21 carries the armed GM event Fires and each authored trigger's arm
/// latch (issue #1301). It is the same cross-schedule gap one rung further: a
/// format-20 capture taken after the Fire crossed its PreUpdate apply boundary
/// but before `tick_trigger_pipeline` ran the handler records an Applied Fire
/// with nothing that will ever run it, and the journal's idempotency makes a
/// second Fire of that one-shot a No-op — so the event becomes permanently
/// unreachable rather than merely late. Durable results also gained the
/// action's stable target identity, without which a restored feed cannot say
/// WHICH event a logged Fire fired.
///
/// Format 22 carries the armed GM direct damage/heal effects (issue #1310).
/// The same cross-schedule gap again, and the same consequence: a format-21
/// capture taken after the effect crossed its PreUpdate apply boundary but
/// before `SimSet::Damage` landed it records an Applied hit — with an exact
/// hull amount and a lethal flag on the durable result — that no restored peer
/// will ever apply, and the journal's idempotency makes a second press of the
/// same correlation a duplicate rather than a retry. Durable results also
/// gained the resolved effect itself, without which a restored feed cannot say
/// how much a logged hit landed or discarded.
///
/// Format 23 carries the armed GM PLACEMENTS (issue #1305) — the same
/// cross-schedule gap one rung further along, and the one whose loss is least
/// recoverable. A format-22 capture taken after a placement crossed its
/// PreUpdate apply boundary but before `tick_trigger_pipeline` spawned the hull
/// records an Applied placement with nothing that will ever perform it; and
/// because each press is its own correlation rather than an idempotent latch, a
/// GM trying again would risk TWO hulls rather than recovering the one. The
/// queue's ORDER travels with it because it decides which placement draws which
/// `WorldIdMint` id.
///
/// Format 24 carries which GM-operable events are PAUSED, and the lever each
/// durable GM result pulled (issue #1303). A format-23 record has no field that
/// can say either. Resuming it un-pauses every paused event — the restored
/// world resumes evaluating conditions a GM deliberately stopped, while the
/// journal it carries still says that Pause was Applied, so a second Pause is a
/// No-op and no lever remains that could change either fact. And every logged
/// event-control result in it is a Fire by construction, so defaulting the
/// lever would republish a restored run's Pauses and Resumes as fires of the
/// same events.
///
/// Format 25 carries the armed GM event Skips (issue #1304).
/// Format 21's gap in the other direction: a
/// format-24 capture taken after the Skip crossed its PreUpdate apply boundary
/// records an Applied arm that no restored peer will honour, so the occurrence
/// the GM bought silence for happens in full — the loudest possible way to
/// lose a decision the operator already made, and one no re-press can undo
/// once the moment has passed. Durable results also gained the event-control
/// LEVER, without which a restored feed reports every Skip as a Fire of the
/// same event: the opposite sentence about the same button.
///
/// Format 26 carries the SCOPE of an armed or already-applied GM direct effect
/// (issue #1311). A format-25 record predates Station and System scopes, so
/// every effect it holds is a whole-hull one and defaulting the field would be
/// correct — but the durable result gained `effect_scope` alongside it, and a
/// format-25 journal that quietly restored as "whole hull" would let a resumed
/// feed say a GM emptied a ship when they emptied one Station. The format gate
/// refuses that rather than inventing the difference, which is the same
/// judgement formats 21 and 22 made about the facts they added.
/// `27` — #1306 preserves accepted removals waiting for the fixed pipeline.
/// `28` — #1307 retains Objective records, recipient scope and terminal status.
/// Format 29 persists per-observer contact overrides without granting scan knowledge.
/// `30` — #1312 retains independent GM-disabled System latches.
/// `31` — #1317 retains per-ship Comms routing, scripted dialogue, literal text,
/// and the captured audience of durable GM outcomes and refusals.
/// `32` — #1308 retains scenario-applied NPC doctrine through restoration.
/// `33` — #1449 carries the previous per-ship impulse hull sample so a
/// restored damaged hull does not invent or forget a pending drive cancel.
/// It also carries current token-free mesh crew authority; standalone restores
/// retain their fresh crew policy.
/// `34` — #1442 carries GM-attributed faction hostility overrides. This one
/// DOES bump, unlike the additive fields that argue their way out of it above:
/// the faction registry is rebuilt from `assets/factions/*.toml` on every load,
/// so a pre-#1442 save of a run in which a GM made two factions hostile
/// restores a world where they are not — while the journal that save also
/// carries still reports that action as Applied. That is the "silently wrong,
/// and indistinguishable from correct" shape, and no content-digest argument
/// rescues it: the GM action moved the world without moving a single authored
/// file.
/// `36` — issues #1522/#1523 retain the launch-frozen authored ship roster and
/// independent named Objective instances, including progress, fixed completion
/// credit and per-ship frozen history. A format-35 save cannot distinguish a
/// deliberately absent slot from an old single-ship boot, or an untouched
/// Objective definition from one whose named sibling already completed, so it
/// is refused rather than reconstructed differently.
/// `37` - explicit Console ownership and working-view continuation. Earlier
/// saves cannot describe which Console owns a System or its shared work context.
/// `38` - composed Dock command channels and mechanical undock clearance.
/// Older intent state cannot be restored into authored channel bindings.
/// `39` — individual System AI tree states, memory and call continuation.
/// `40` — common observation metadata and repair report continuation.
/// `41` — canonical repair jobs, team commitments and private workflow continuation.
/// 42: canonical stocks, tube reservations and fractional resource-flow continuation.
/// 43: local energy networks, thermal hysteresis, finite coolant credit and flow restoration.
/// 44: raised shield intent, battery recovery and partial-interception continuation.
/// 45: persistent physical repair-team positions and shared access-boundary state.
/// 46: physical damage, retained impact profiles/samples, typed parts-backed
/// repair stages and reversible actor defeat frontiers replace primary ship HP.
/// `48` adds latched stock requests and finite recipe work-in-progress.
/// `49` adds physical port visits, approved lot orders and paid transfer meters,
/// and carries exact Duration mission age in this same unreleased batch.
/// `50` adds actual portable equipment, physical refitting and intact recovery.
/// `51` retains exact canonical logical combat/LOD timestamps across long runs.
/// `52` adds per-instrument sensing histories and spatial-field continuation.
/// `53` retains weapon assignments, finite mounts, causal EMP episodes, active
/// beam operation and frozen guided-round equipment. Older state is refused.
/// `54` scopes issued hails to the actual issuing ship. Old unscoped history
/// cannot be assigned to a fleet hull without inventing ownership.
/// `55` retains original burst launch identity, dated guidance provenance and
/// source-owned issued/heard order knowledge separately from physical flight.
// 56 adds canonical per-axis Helm intent and dated recipient-scoped radio facts.
// 57 adds finite officer custody joined to the candidate's actual Repair teams.
// 58 includes emitter command channels and frozen receiver illumination opt-in.
// 59 retains typed Comms requests, actual-ship response ownership and partial reading.
// 60 retains real activity outcomes/situations and explicit bootstrap slot provenance.
// 61 retains independent jammer command intent; paid current influence is derived.
// 62 retains actual Transit order phases, accepted-course/fuel and arrival receipts.
// 63 retains immutable HTN method choice and original primitive release ticks.
/// `64` retains original bounded capability-refusal explanations on PlanChoice.
/// `65` retains actual onboard signal-protection command intent.
/// `66` retains canonical crew presence, dated temporal decisions, actual probe
/// custody/Bay work and complete shared Security jobs, including consumed IDs.
/// `67` distinguishes physical JobIds from actual transmission execution identity.
/// `68` retains actual planned radio emission/receipt bindings and local Security
/// team routes rather than reconstructing work from a scenario instruction.
/// `69` retains original applied Scenario callback ticks for causal dependencies.
/// `70` retains actual personnel conditions and typed Treatment work.
/// `71` retains an accepted root mission promise and received formation lineage.
/// `72` retains actual identity-emitter intent, never prepaid physical emissions.
/// `73` requires the single retained personnel roster, including removed origins.
/// `74` retains original Transit scopes and the latest successful World arrival.
pub const SNAPSHOT_FORMAT: u32 = 74;

/// The simulation, as a string because "0.1-pre" says more in a bug report than
/// "1" and because nothing compares these for order.
///
/// Bump it whenever the simulation's rules change, however slightly. A save
/// recorded under other rules is not a save this build can honour, and the only
/// available honesty is to refuse it — `Run` has no migration hook and is not
/// meant to.
///
/// `"0.2"` — issue #1086 widened [`crate::sim_digest`] over the scenario's own
/// state (the flag stores, the trigger latches, the scheduled queues, the named
/// records and the whole of comms). Nothing about the PAYLOAD changed, so
/// [`SNAPSHOT_FORMAT`] stays 13 and an unedited world's content digest is
/// unchanged — which is exactly the problem this bump solves. Every pre-#1086
/// save of a scripted world recorded a NARROWER digest than this build
/// recomputes, and [`restore`]'s equality check would have reported that as "the
/// save did not restore cleanly": a corruption message for a save that is
/// perfectly intact and merely older than the rules. The rules dimension is the
/// one that can say what actually happened, and refusing on it is what makes it
/// say so. Earlier fold widenings (#1025/#1028/#1041/#1107/#1156) left this
/// alone because each moved the digest only for the worlds that used the surface
/// it added; this one moves it for essentially every scenario world.
///
/// `"0.3"` — issue #1116. Two rule changes, both of which move a run without
/// changing the payload's shape: `ShipPhysics.roll` is now integrated for every
/// ship rather than only the locally-projected one (a folded field, so every
/// world with a manoeuvring NPC folds differently from the first tick), and a
/// `StoredRun`'s continuation log element gained a `CommandOrder`. A pre-#1116
/// save restores intact and then diverges on its first continuation tick, which
/// is precisely the failure the rules dimension exists to name.
///
/// `"0.4"` — issue #1292 makes typed, attributed GM actions part of the
/// authoritative run. Session pause now participates in the fold and its
/// ordered journal can change both the current clock state and future logical
/// boundaries. A pre-#1292 save recorded a narrower digest even when its payload
/// otherwise parses, so the rules dimension names that change explicitly.
///
/// `"0.5"` — issue #1316 commits collision attribution on every simulation
/// host before fixed-boundary digests and saves. The old browser omitted that
/// history and headless collected it after the save boundary. The row schema is
/// unchanged, but those older authoritative folds cannot continue as this run.
/// `"0.6"` — #1449 applies impulse speed modifiers and hull-damage cancellation
/// on every simulation host and preserves the pending damage continuation.
///
/// `"0.7"` — issue #1438. Two rule changes, neither of which moves the
/// payload's shape. A `RepairRequest` routed to a HUMAN Repair seat now writes
/// the ship's [`crate::console::repair::RepairRequestQueue`] on every peer
/// (`receive_repair_coordination`'s new `HumanRouted` arm), where before only
/// the `Ai` arm wrote it; and the stale-entry prune moved out of the
/// cadence-gated `operate_repair_ai` into the seat-independent
/// `prune_repair_request_queue`, which runs every fixed step regardless of who
/// holds the seat. The queue is folded into no digest, so nothing about the
/// content changed — which, exactly as in `"0.3"`, is why the RULES dimension
/// is the one that can say what happened. `operate_repair_ai` is gated on the
/// fine `repair` System rather than the seat, so on a stock
/// `alliance_cruiser` whose Engineering Station carries the `Simplified`
/// rating (`automated_systems = ["repair"]`) that write now dispatches repair
/// teams and heals hull HP — authoritative state — where the pre-#1438 build
/// did nothing at all. A pre-#1438 save of such a run restores intact with an
/// empty queue and then diverges at its next damage-tier crossing, and the
/// prune's new cadence moves when entries die on AI hulls besides. That is
/// "restores cleanly and then continues as a different run", which is the
/// failure this dimension exists to refuse. The same write also makes the
/// human Repair console's own `queue_depth` readout non-empty on every
/// human-crewed hull, where it was always zero before; that is presentation
/// rather than rules, but it is the same write and is named here so the two
/// are not mistaken for separate changes.
/// "0.8" — ship capabilities now follow the same authored-presence rules for
/// player and NPC hulls. Player-only default shields, phasers and repair teams
/// are removed, impulse steering shares the schema default, and equipment-less
/// ships cannot borrow weapons from a global resource. Older saves would
/// continue under different rules despite retaining the same payload shape.
/// "0.9" — explicit Console ownership and knowledge scopes determine control
/// and individual System AI inputs. Older saves lack the ownership and working
/// view state needed to continue under these rules.
/// "0.10" - Dock composition and complete intent/clearance digest coverage.
/// "0.11" — Dock runs one authored state-tree controller with persistent private memory.
/// "0.12" — explicit repair report reduction and observation continuity.
/// "0.13" — canonical repair assignments and restricted per-job workflows.
/// "0.17" — authored compartment routes gate repair arrival and knowledge;
/// access boundaries retain actual transitions separately from protected orders.
/// "0.18" — directional compartment damage, component conditions, finite-parts
/// restoration and recoverable neutralization replace aggregate ship destruction.
/// "0.19" adds timed cargo and conserved destination-space claims.
/// "0.20" adds automatic local stock cycles and paid finite fabrication.
/// "0.21" adds finite visit-quota replenishment and actual-stock manual exchanges,
/// and removes process-local floating cancellation from mission clocks.
/// "0.22" adds conserved equipment changes without resetting fixed-slot reserves.
/// "0.23" moves historical combat recency onto the shared logical tick clock.
/// "0.24" adds paid sensing continuation and causally anchored spatial fields.
/// "0.25" adds measured per-bank targeting, finite mounted fire and shared
/// weapon resources, temporary EMP and equipment-supported shield enquiry.
/// "0.26" applies fleet hails and their anti-repeat history per issuing hull.
/// "0.27" delivers paid source orders through finite paired receiver links;
/// guidance received after movement steers on the following advance.
/// "0.28" retains paid per-axis direct/assisted intent and makes authored radio
/// listening/emission physical work, with original sent/received facts.
/// "0.29" applies finite co-located officer posting and internal Repair expertise
/// through the sole existing team/job navigation and work owners.
/// "0.30" emits independently paid, measured Science assistance to explicitly
/// opted-in matching guided rounds, retaining their physical target intent.
/// "0.31" settles equipment-bound facility services using both real paid
/// energy owners and the existing finite stock exchange in one transaction.
/// "0.32" pays controlled Dock separation through actual moving equipment and
/// retains its sole canonical physical pose across save/restore.
/// "0.33" bases torpedo auto-fire on the firing ship's canonical rating,
/// independent of other ships' human claims and the peer's local projection.
/// "0.34" lets the ordinary Power tree explicitly select the existing allocation
/// mode; shipped trees use priorities without changing physical resource tuning.
/// "0.35" delivers typed status requests through the existing physical radio,
/// same-inbox reading/decisions and paid sustained returned outcomes.
/// "0.36" binds actual Repair and delivery job facts to World situations and
/// stages their existing script consequences once at the retained event frontier.
/// "0.37" lets the ordinary Comms policy answer an actually heard/read typed
/// status request without consulting unknown reverse-link viability.
/// "0.38" applies authored nonplayer World headings to the existing physics
/// owner, matching the retained transform and runtime-spawn convention.
/// "0.39" compares acquired shield rays against their published position
/// uncertainty, retaining current/fresh/evidence-only weapon decisions.
/// "0.40" settles paid jammer influence before Input hails and shares that
/// actual current-step result with radio, receipts and observation links.
/// "0.41" advances one actual paid local Transit order per ship, with fixed
/// accepted navigation and static arrival policy through the existing physics owner.
/// "0.42" honours explicit instrument Missile selection and normal acquired
/// classification in the actual paid Broad ordnance producer.
/// "0.43" decomposes imported maintenance plans over actual activity owners
/// and retains their choice, causal release and original job evidence once.
/// "0.44" validates original deterministic method selection and explanations
/// and recognises validated physical flight carriers on delayed rebuild.
/// "0.45" applies paid equipment interference and allocated protection to exact
/// actual Comms, Broad, Focused and Passive receiver owners.
/// "0.46" uses actual compatible paid relay routes, dated crew-bound scheduling,
/// canonical stored probe lifecycles and complete shared Security commitments.
/// "0.47" validates activity evidence against its closed execution family.
/// "0.48" joins actual paid communication execution to HTN evidence and routes
/// local Security work through the installed compartment graph.
/// "0.49" requires successful original Scenario application receipts before
/// releasing dependent work and validates frozen Transit equipment/cost terms.
/// "0.50" executes finite Treatment through the existing exclusive Repair jobs.
/// "0.51" gates optional HTN roots on real accepted promises and keeps actual
/// ordered formation suggestions separate from Navigation/Helm execution.
/// "0.52" emits configured identity only from fully paid real equipment and
/// substitutes measured identity after ordinary sensor acquisition.
/// "0.53" retains finite named personnel across actual origin-actor removal,
/// with shared care/posting consumers and accepted creation provenance.
/// "0.54" uses actual paid Transit and tracked layers for authored World routes,
/// retaining original drive receipts and the current accepted arrival context.
/// "0.55" preserves actual paid machinery work across the complete fixed
/// interval so later observations include both Input and Physics operation.
pub const SIMULATION_RULES: &str = "0.55";

/// The authored data, computed rather than remembered.
///
/// Issue #935: this used to hash the scenario TOML text alone, which left
/// entity templates, fragments, and sidecars free to drift under a save. It
/// now folds a [`crate::content_ledger::ContentLedger`] — every authored file
/// the world/entity loader actually consumed for this load, scenario TOML
/// included, keyed by canonical path. Taking the ledger rather than a lone
/// string is deliberate: the digest can only be honest about what the caller
/// hands it, and a bare `&str` parameter invited exactly the narrow read that
/// caused this issue.
///
/// `fnv1a`/`fold_digest` rather than a phoenix-local hash, because
/// `vellum-digest` is already the fleet's digest primitive and a second one
/// would be a second answer.
pub fn content_digest(ledger: &crate::content_ledger::ContentLedger) -> u64 {
    ledger.fold()
}

/// The three dimensions this build writes and reads saves against.
pub fn versions(ledger: &crate::content_ledger::ContentLedger) -> Versions {
    Versions::new(SNAPSHOT_FORMAT, SIMULATION_RULES, content_digest(ledger))
}

/// The stored artifact's full type: `vellum-save`'s envelope, phoenix's payload.
///
/// Named because it appears in four signatures and spelling it out invites the
/// two parameters being swapped. `LoggedCommand` is the log's element type even
/// though this issue always stores an empty log — the type is what makes #849's
/// continuation log a filled-in field rather than a new artifact.
pub type StoredRun = Run<LoggedCommand, PhoenixSnapshot>;

// ── The stored artifact ──────────────────────────────────────────────────────

/// Build the `vellum_save::Run` a save is written as.
///
/// The log is empty and the ledger holds only the capture: this is a saved
/// game, not yet a recording. `vellum-save`'s own
/// `a_snapshot_with_an_empty_log_is_a_saved_game_that_verifies` is the shape
/// this mirrors, and the continuation log is #849's to fill in — at which point
/// nothing here changes, because `Run` already has the field.
///
/// `digest` is the caller's, deliberately: it is `sim_digest::world_digest` of
/// the same world at the same instant, and taking it here would mean this
/// module deciding when a digest is meaningful. That decision belongs to the
/// caller who knows it is standing between `update()` calls.
pub fn run_for(
    payload: PhoenixSnapshot,
    digest: u64,
    seed: u64,
    scenario: impl Into<String>,
    versions: Versions,
) -> StoredRun {
    let tick = payload.tick;
    Run {
        versions,
        scenario: scenario.into(),
        seed,
        snapshot: Some(Snapshot {
            tick,
            digest,
            state: payload,
        }),
        commands: Vec::new(),
        ledger: Ledger {
            every: 0,
            samples: Vec::new(),
            final_tick: tick,
            final_digest: digest,
        },
    }
}

/// The slot a host's one save lives in.
///
/// `vellum_save::is_slot` is what decides whether a name is usable at all, and
/// it is checked by the backends rather than trusted — this is just phoenix's
/// default choice of name.
pub const DEFAULT_SLOT: &str = "autosave";

/// The `localStorage` namespace the browser backend keys under.
///
/// Namespaced because two games served from one origin — which is exactly what
/// a GitHub Pages account is — must not read or overwrite each other's saves.
pub const STORAGE_NAMESPACE: &str = "phoenix";

/// The file name a host is offered when it exports a save (issue #866).
///
/// `.ron` and not `.sav`, because the extension is a promise about the contents
/// and this one is keeping it: `Store` moves `String`, the record is RON text,
/// and an exported save is a file a human can open in an editor and read. That
/// is the property to lean on in a bug report — "paste me the first twenty
/// lines" is a diagnosis, and an opaque blob is not.
pub const EXPORT_FILE_NAME: &str = "phoenix-save.ron";

// ── The portable artifact (issue #866) ───────────────────────────────────────

/// A [`vellum_save::Store`] that holds its slots in memory, for the moment a
/// save is in transit rather than at rest.
///
/// # Why a third backend and not two lines of `to_ron`/`from_ron`
///
/// A browser has no filesystem, so "export a file" cannot be `FileStore` and
/// "import a file" cannot be `FileStore` either: the bytes arrive from a
/// `<input type="file">` and leave through a download, and the only durable
/// thing in between is the RON text itself. The obvious implementation is to
/// call `Run::to_ron` on the way out and `Run::from_ron` plus `versions.check`
/// on the way in — and that is exactly what this type exists to avoid.
///
/// [`load_from`]'s three lines are not three lines: they are an ORDER — read,
/// parse, gate — and the module's own docs explain why the gate runs before a
/// single component is written. A second copy of that sequence for the file
/// path would be a second place for it to be got wrong, and the way it would be
/// got wrong is silent (a host half-adopts a world it is about to be refused).
/// So the file path goes through the same [`save_to`] and [`load_from`] every
/// slot goes through, and what changes is only *where the string lives* — which
/// is the one thing `Store` is for.
///
/// That is also what makes this issue's "no second snapshot schema" true by
/// construction rather than by review: there is nowhere to put one. Three
/// backends, one record, one gate.
///
/// # Not a cache and not a save
///
/// It keeps nothing beyond the call that made it. Each export builds one, writes
/// one slot into it and takes the string straight back out; each import builds
/// one holding the text it was handed and reads it once. Nothing polls it,
/// nothing persists it, and it is deliberately not a `Resource`.
#[derive(Debug, Default)]
pub struct TransferStore {
    pub(super) slots: std::cell::RefCell<std::collections::BTreeMap<String, String>>,
}

impl TransferStore {
    /// An empty store, for an export about to be written into it.
    pub fn empty() -> Self {
        Self::default()
    }

    /// A store already holding `text` in `slot`, for an import about to be read
    /// out of it.
    pub fn holding(slot: &str, text: String) -> Self {
        let store = Self::empty();
        store.slots.borrow_mut().insert(slot.to_string(), text);
        store
    }

    /// Take a slot's text out, leaving the store without it.
    pub fn take(&self, slot: &str) -> Option<String> {
        self.slots.borrow_mut().remove(slot)
    }
}

impl vellum_save::Store for TransferStore {
    /// Nothing here can fail: a `BTreeMap` in this process always answers.
    type Error = std::convert::Infallible;

    fn read(&self, slot: &str) -> Result<Option<String>, Self::Error> {
        Ok(self.slots.borrow().get(slot).cloned())
    }

    fn write(&self, slot: &str, contents: &str) -> Result<(), Self::Error> {
        self.slots
            .borrow_mut()
            .insert(slot.to_string(), contents.to_string());
        Ok(())
    }

    fn remove(&self, slot: &str) -> Result<(), Self::Error> {
        self.slots.borrow_mut().remove(slot);
        Ok(())
    }

    fn slots(&self) -> Result<Vec<String>, Self::Error> {
        Ok(self.slots.borrow().keys().cloned().collect())
    }
}

/// Serialise a run into the text of a portable save file (issue #866).
///
/// Byte-identical to what [`save_to`] hands `LocalStorage` for the same run,
/// because it *is* [`save_to`] — see [`TransferStore`]. An exported file and a
/// browser slot are the same record in the same encoding, which is what makes
/// importing one into another host a resume rather than an import format.
pub fn export_artifact(run: &StoredRun) -> Result<String, String> {
    let store = TransferStore::empty();
    save_to(&store, DEFAULT_SLOT, run)?;
    store
        .take(DEFAULT_SLOT)
        .ok_or_else(|| "the exported save was not written".to_string())
}

/// Read the text of a portable save file and put it through the same gate a
/// local slot goes through (issue #866).
///
/// The mirror of [`export_artifact`], and the same statement about the gate:
/// this is [`load_from`] over a store that happens to hold the file's text, so
/// the refusal it returns is the one a `localStorage` slot would have returned
/// for the same bytes.
///
/// The two refusals a host actually meets are different kinds of bad news and
/// stay different values here rather than collapsing into one sentence:
/// [`LoadRefusal::Unparsable`] means the FILE is damaged — truncated, edited,
/// or not a save at all — and [`LoadRefusal::Moved`] means the file is intact
/// and this BUILD cannot honour it, naming the dimension that moved. "Pick
/// another file" and "this save is from an older build" are different
/// instructions, and a host told the wrong one goes looking in the wrong place.
pub fn import_artifact(text: &str, current: &Versions) -> Result<StoredRun, LoadRefusal> {
    let store = TransferStore::holding(DEFAULT_SLOT, text.to_string());
    load_from(&store, DEFAULT_SLOT, current)
}

/// Which scenario a portable save belongs to, read WITHOUT the version gate.
///
/// The browser needs this before it can run the gate at all, and the ordering is
/// forced rather than chosen: the content dimension is a digest over the files a
/// world load consumed, so there is nothing to check a save against until its
/// world has been loaded — and which world that is, is written in the save. So
/// an import reads the scenario first, loads it, and only then asks
/// [`import_artifact`] whether this build can honour the file.
///
/// It is exactly `Run::from_ron` and one field read: parsing is the only thing
/// that can fail here, which is why a damaged file is caught at THIS step and
/// reported as damaged before any world is loaded on its behalf.
pub fn peek_artifact_scenario(text: &str) -> Result<String, LoadRefusal> {
    StoredRun::from_ron(text)
        .map(|run| run.scenario)
        .map_err(|e| LoadRefusal::Unparsable(e.to_string()))
}

/// Why a stored save did not become a resumable session.
///
/// [`LoadRefusal::Moved`] carries `vellum_save::Moved` **unchanged**, and its
/// `Display` is that type's own sentence. That is the acceptance criterion, not
/// a convenience: a phoenix-worded status would be a second answer to a
/// question the version gate has already answered, and it would lose the one
/// thing the gate exists to report — *which* dimension moved, and to what.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadRefusal {
    /// Nothing is stored in that slot. A first run has no save, which is not an
    /// error.
    Empty,
    /// The store itself would not answer.
    Unreadable(String),
    /// The bytes are not a `Run` this build can parse.
    Unparsable(String),
    /// The version gate refused it.
    Moved(vellum_save::Moved),
}

impl std::fmt::Display for LoadRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadRefusal::Empty => f.write_str("there is no save in that slot"),
            LoadRefusal::Unreadable(why) => write!(f, "the save could not be read: {why}"),
            LoadRefusal::Unparsable(why) => write!(f, "the save could not be parsed: {why}"),
            // Verbatim. See the type's docs.
            LoadRefusal::Moved(moved) => write!(f, "{moved}"),
        }
    }
}

/// Write a run to a slot, through `vellum-save`'s store and nothing else.
pub fn save_to<S: vellum_save::Store>(
    store: &S,
    slot: &str,
    run: &StoredRun,
) -> Result<(), String> {
    let text = run.to_ron().map_err(|e| e.to_string())?;
    store.write(slot, &text).map_err(|e| e.to_string())
}

/// Read a run back and put it through the version gate before anything is
/// activated.
///
/// The gate runs *here*, before a single component is written, because that
/// ordering is the whole reason it exists: restoring first and refusing second
/// would mean a host had already half-adopted a world it is about to be told it
/// cannot have.
pub fn load_from<S: vellum_save::Store>(
    store: &S,
    slot: &str,
    current: &Versions,
) -> Result<StoredRun, LoadRefusal> {
    let text = store
        .read(slot)
        .map_err(|e| LoadRefusal::Unreadable(e.to_string()))?
        .ok_or(LoadRefusal::Empty)?;
    // Read the same Vellum envelope while skipping obsolete payload layouts.
    // A removed stock field must report a version mismatch, not file corruption.
    let header: Run<serde::de::IgnoredAny, serde::de::IgnoredAny> =
        ron::from_str(&text).map_err(|e| LoadRefusal::Unparsable(e.to_string()))?;
    header.versions.check(current).map_err(LoadRefusal::Moved)?;
    let run = StoredRun::from_ron(&text).map_err(|e| LoadRefusal::Unparsable(e.to_string()))?;
    required_boot_identity(&run)?;
    Ok(run)
}

/// Return the pre-world identity a current-format run requires.
///
/// The version check must run before this semantic check so an older artifact
/// reports the authoritative format refusal. A run claiming the current format
/// while omitting either its snapshot or its boot identity is malformed: there
/// is no safe hull/fleet a host can infer on its behalf.
pub fn required_boot_identity(run: &StoredRun) -> Result<&BootIdentity, LoadRefusal> {
    let identity = run
        .snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.state.boot_identity.as_ref())
        .ok_or_else(|| {
            LoadRefusal::Unparsable(
                "the current snapshot format does not carry its required boot identity".to_string(),
            )
        })?;

    // A stationless GM peer flies no roster ship and holds no lobby selection,
    // so it records neither a hull nor (in a GM-only fleet) any ship at all —
    // its identity is the replicated roster it participates in (issue #1445).
    // That relaxation is granted ONLY to a peer that claims no hull: a peer
    // that recorded one must still appear in the roster it claims to fly in.
    let flies_own_ship = identity
        .fleet
        .ships()
        .iter()
        .any(|ship| ship.host == identity.fleet.local());
    if identity.selected_ship.trim().is_empty() {
        if flies_own_ship {
            return Err(LoadRefusal::Unparsable(
                "the current snapshot boot identity has no selected player hull".to_string(),
            ));
        }
    } else if identity.fleet.is_empty() {
        return Err(LoadRefusal::Unparsable(
            "the current snapshot boot identity has an empty fleet roster".to_string(),
        ));
    }
    if !identity.fleet.is_member(identity.fleet.local()) {
        return Err(LoadRefusal::Unparsable(
            "the current snapshot boot identity's local slot is absent from its fleet roster"
                .to_string(),
        ));
    }
    if identity
        .fleet
        .ships()
        .windows(2)
        .any(|pair| pair[0].host >= pair[1].host)
    {
        return Err(LoadRefusal::Unparsable(
            "the current snapshot boot identity's fleet roster is not in unique slot order"
                .to_string(),
        ));
    }
    if identity.game_start_entity_uuids.len() < identity.fleet.len() {
        return Err(LoadRefusal::Unparsable(format!(
            "the current snapshot boot identity carries {} GameStart UUID(s) for a fleet of {}",
            identity.game_start_entity_uuids.len(),
            identity.fleet.len()
        )));
    }
    if identity
        .game_start_entity_uuids
        .windows(2)
        .any(|pair| pair[0].authored_index >= pair[1].authored_index)
    {
        return Err(LoadRefusal::Unparsable(
            "the current snapshot boot identity's GameStart UUIDs are not in unique authored-row order"
                .to_string(),
        ));
    }
    let snapshot = run
        .snapshot
        .as_ref()
        .expect("a boot identity can only be reached through a snapshot");
    if snapshot
        .state
        .gm_actions
        .applied_prefix()
        .iter()
        .any(|grant| grant.apply_tick > snapshot.state.tick)
    {
        return Err(LoadRefusal::Unparsable(
            "the snapshot's applied GM frontier crosses its capture tick".to_string(),
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut seen_slots = std::collections::BTreeSet::new();
    for row in &identity.game_start_entity_uuids {
        if let Some(slot) = &row.authored_slot_id {
            if slot.trim().is_empty() || !seen_slots.insert(slot) {
                return Err(LoadRefusal::Unparsable(
                    "invalid or duplicate GameStart authored launch slot".into(),
                ));
            }
        }
        let uuid = &row.entity_uuid;
        let Some(parsed) = crate::world_id::WorldId::parse(uuid) else {
            return Err(LoadRefusal::Unparsable(format!(
                "the current snapshot boot identity's GameStart UUID for authored row {} is not a minted world id",
                row.authored_index
            )));
        };
        if parsed.namespace != crate::world_id::IdNamespace::Entity {
            return Err(LoadRefusal::Unparsable(format!(
                "the current snapshot boot identity's GameStart UUID for authored row {} is not an entity id",
                row.authored_index
            )));
        }
        if parsed.render() != *uuid {
            return Err(LoadRefusal::Unparsable(format!(
                "the current snapshot boot identity's GameStart UUID for authored row {} is not canonical",
                row.authored_index
            )));
        }
        if !seen.insert(uuid) {
            return Err(LoadRefusal::Unparsable(format!(
                "the current snapshot boot identity repeats GameStart UUID {uuid:?}"
            )));
        }
        let occurrences = snapshot
            .state
            .entities
            .iter()
            .filter(|entity| entity.uuid == *uuid)
            .count();
        if occurrences > 1 {
            return Err(LoadRefusal::Unparsable(format!(
                "the current snapshot boot identity's GameStart UUID {uuid:?} occurs {occurrences} times in the captured entity roster"
            )));
        }
    }

    Ok(identity)
}

// ── Verification ─────────────────────────────────────────────────────────────

/// The `Sampling` simulation [`vellum_save::verify`] checks a restore against.
///
/// Deliberately tiny, and deliberately not `headless::replay::PhoenixSim`: that
/// type is native-only (it lives under the `headless` feature) and a browser
/// host is precisely the thing that has to be told its save will not load. This
/// adapter compiles on both targets because it does nothing but hold a world
/// and hash it.
///
/// # `apply` refuses, and that is not a stub
///
/// This issue's artifact has an **empty log** by construction — a snapshot with
/// no commands is a saved game, which is the whole of what #862 stores — so
/// `replay_into` never reaches `apply`. Refusing rather than pretending is the
/// honest encoding of that: if a log ever does arrive here, it arrived from
/// #849's continuation work, and the right answer is a named refusal rather
/// than a silent no-op that would make an unreplayed command look replayed.
/// When #849 lands, the verifier for a run *with* a log is `PhoenixSim`, which
/// already crosses the production admission boundary.
pub struct SavedGame<'a> {
    pub(super) world: &'a World,
    pub(super) ledger: Ledger,
}

/// Why [`SavedGame`] will not replay a command. See its docs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoContinuationLog;

impl std::fmt::Display for NoContinuationLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a saved game carries no command log to replay (issue #849 adds one)")
    }
}

impl<'a> SavedGame<'a> {
    /// Wrap a restored world for verification.
    ///
    /// The ledger is empty on purpose. `verify` reads this side's ledger only
    /// to look for a *sampled* disagreement, and a saved game samples nothing;
    /// the numbers it actually compares — the capture digest and the final
    /// digest — both come from `run`, checked against `digest()` recomputed
    /// live. Handing the recorded digest in here would make the check confirm
    /// itself.
    pub fn new(world: &'a World) -> SavedGame<'a> {
        SavedGame {
            world,
            ledger: Ledger::default(),
        }
    }
}

impl vellum_replay::Simulation for SavedGame<'_> {
    type Command = LoggedCommand;
    type Rejection = NoContinuationLog;

    fn apply(&mut self, _command: &LoggedCommand) -> Result<(), NoContinuationLog> {
        Err(NoContinuationLog)
    }

    fn is_over(&self) -> bool {
        false
    }

    fn digest(&self) -> u64 {
        crate::sim_digest::world_digest(self.world)
    }
}

impl vellum_save::Sampling for SavedGame<'_> {
    fn ledger(&self) -> &Ledger {
        &self.ledger
    }

    /// A restored world already stands at the capture tick, so there is no tail
    /// to run out. Stepping here would need `&mut World` and a schedule, and
    /// would be running the simulation *inside* a verification — which is the
    /// one thing a check of "did the restore land?" must not do.
    fn advance_to(&mut self, _tick: u64) {}
}

#[cfg(test)]
#[path = "artifact_tests.rs"]
mod tests;
