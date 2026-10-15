//! Schema.
use super::*;

// ── The payload ──────────────────────────────────────────────────────────────

/// One `EntityUuid`-bearing entity's authoritative state.
///
/// Keyed by the uuid string, never by an ECS handle — see the module docs.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EntityState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe_custody: Option<crate::entities::craft::ProbeCustody>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bay_jobs: Option<crate::entities::craft::ShipBayJobs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transit: Option<crate::console::helm::transit::ShipTransitDrives>,
    /// Retained movement requests survive LOD removal of actuator latches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub helm_movement: Option<phoenix_sim_gameplay::ship::helm_control::HelmControlState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weapon_targets: Option<phoenix_sim_gameplay::weapons::assignments::WeaponTargetAssignments>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guided_source: Option<phoenix_model::guided_orders::GuidedSourceKnowledge>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub illumination: Option<crate::console::weapons::illumination::ShipIllumination>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jamming: Option<crate::comms::jamming::ShipJamming>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal_protection: Option<crate::science::protection::ShipSignalProtection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_emitters: Option<crate::science::identity::ShipIdentityEmitters>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concealment_emitters: Option<crate::science::concealment::ShipConcealmentEmitters>,
    /// Nonspatial owner of already-launched rounds after launcher removal.
    #[serde(default, skip_serializing_if = "is_false")]
    pub weapon_flight_carrier: bool,
    /// Ordered bank IDs recreate emitted bolts without a ship recipe.
    /// Derived from actual flight-only banks; no separate runtime ID list.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub carrier_bolt_banks: Vec<String>,
    /// Physical bearings only; target/enabled intent belongs to each System.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mounts: Option<crate::weapons::mounts::ShipMounts>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shield_frequency: Option<super::shield_enquiry::ShieldFrequencyState>,
    /// Navigation's deliberate received fixed-point history and source dating.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub navigation_sharing:
        Option<phoenix_sim_gameplay::science::navigation_sharing::NavigationSharingState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub broad: Option<std::collections::BTreeMap<SystemId, crate::science::broad::BroadTrackState>>,
    pub uuid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fitting: Option<crate::resources::fitting_server::ShipFitting>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equipment_recovery:
        Option<crate::resources::equipment_recovery_server::ShipEquipmentRecovery>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logistics: Option<crate::resources::logistics::LogisticsState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub production: Option<crate::resources::production::ProductionState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<crate::resources::recovery::RecoveryState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port_provider: Option<crate::resources::ports::PortVisitState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub physical_damage: Option<phoenix_sim_gameplay::ship::compartment_damage::DamageState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disablement: Option<crate::ship::disablement::DisablementObservation>,
    /// What a **script spawned this entity from** (issue #863), or `None` for
    /// every authored `[[entity]]` block.
    ///
    /// The one field on this row that is not state the entity *has* but the
    /// recipe it was *made by*, and it is here because the row is otherwise
    /// unusable when the target world has no such entity to write into. A fresh
    /// app re-spawns the authored roster from the world file, so an authored
    /// ship is always there to be overwritten; a mid-run spawn is there only if
    /// the bootstrap happened to replay the run that produced it, which a
    /// resumed browser session — booting with nobody at the consoles — does not.
    ///
    /// With the origin, [`restore`] builds the ship instead of reporting a
    /// [`RestoreGap::MissingEntity`]; the rest of this row then lands on it
    /// exactly as it lands on a bootstrapped one. Without it, the gap is still
    /// the honest answer: see [`crate::world::spawn_origin::SpawnOrigin`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawn: Option<crate::world::spawn_origin::SpawnOrigin>,
    /// `ShipPhysics`' eight fields, in the order the digest folds them:
    /// `x, y, z, yaw, forward_speed, roll, lateral_speed, vertical_speed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub physics: Option<[f32; 8]>,
    /// `(SystemId, current, max)` per system, in the hull's own stable
    /// insertion order — the same walk `fold_hull` takes. The tier thresholds
    /// and display names are NOT stored: they are authored config the fresh
    /// world rebuilds from TOML, and storing them would put authored data in a
    /// save that a content-digest change is already meant to invalidate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hull: Option<Vec<(String, f32, f32)>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub red_alert: Option<bool>,
    /// The ship's Command stance selections (issues #1107–#1109) as
    /// `(station id, stance id)`, sorted by station id.
    ///
    /// The per-ship `ShipStationStances` map, which is folded into the sim
    /// digest by `sim_digest::fold_station_stances_namespace` but — before this
    /// field — did not travel, so a resume dropped every stance an AI, human or
    /// objective order had put in force and folded a different number than the
    /// capture. Stored as sorted scalar pairs for `fold`'s own reason: the map's
    /// `HashMap` iteration order is not stable across instances, and the capture
    /// has to be byte-identical whatever order the entries were inserted in.
    ///
    /// EMPTY is the load-bearing default — a hull nobody commands carries an
    /// empty map — so an absent or empty vec restores to an empty map, which is
    /// byte-identical to a never-commanded ship and folds to the same number.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub station_stances: Vec<(String, String)>,
    /// Authoritative Console work context follows the ship through continuation.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub console_views: std::collections::BTreeMap<
        phoenix_model::identity::ConsoleId,
        phoenix_model::console::ConsoleWorkingView,
    >,
    /// Live crew authority carried only into an explicitly requested mesh
    /// continuation. Standalone restore retains its fresh console policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh_crew: Option<MeshCrewState>,
    /// Damage-disabled fine systems in the ship's authoritative control gate.
    ///
    /// This is the mutable half of `ShipSystemControlSources` that cannot be
    /// recovered in time from the restored hull: command admission reads it
    /// before the next damage-sync pass. The ids are sorted for stable saves;
    /// an empty row explicitly clears any state produced by the bootstrap.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub damage_offline_systems: Vec<String>,
    /// Independent availability latches, never reconstructed from hull HP.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gm_disabled_systems: Vec<String>,
    /// Runtime doctrine selections survive palette withdrawal. Absent means
    /// clear any bootstrap selection and restore its original standing doctrine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub npc_doctrine: Option<crate::gm::operations::npc::AppliedNpcDoctrine>,
    /// The entity's AI LOD lifecycle — see [`AiFidelityState`].
    ///
    /// This is explicit rather than inferred from [`Self::control`]: fidelity
    /// owns one canonical component bundle, and a Low capture must remove that
    /// whole bundle even when the fresh bootstrap happened to promote the same
    /// authored entity before restore.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_fidelity: Option<AiFidelityState>,
    /// The helm axes as they stood at the capture — see [`ControlState`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<ControlState>,
    /// The impulse/boost drive machines actuated by the helm commands above.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drive: Option<DriveState>,
    /// Navigation's current goal and exact monotonic generation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub navigation_waypoint: Option<NavigationWaypointState>,
    /// Navigation's exactly-once clearance issuer frontier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub navigation_clearance_issue: Option<NavigationClearanceIssueState>,
    /// Helm's generation latch for the current Navigation waypoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub helm_waypoint_clearance: Option<HelmWaypointClearanceState>,
    /// The weapon state machines — see [`WeaponState`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weapons: Option<WeaponState>,
    /// The state a Shields AI decision must carry across a save — see
    /// [`ShieldsAiState`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shield_ai: Option<ShieldsAiState>,
    /// Sensors' per-ship threat-warning debounce memory — see
    /// [`SensorsThreatMemoryState`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensors_threat: Option<SensorsThreatMemoryState>,
    /// Intent narration's coalescing frontier and monotonic advisory counter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent_narration: Option<IntentNarrationState>,
    /// Recent combat timestamps, stored as fixed-clock ages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recent_combat: Option<RecentCombatActivityState>,
    /// High-fidelity Sensors' delayed frequency-hint timer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frequency_hint: Option<FrequencyHintContinuationState>,
    /// The ship-wide phaser emitter frequency.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phaser_frequency: Option<ShipPhaserFrequencyState>,
    /// Tactical's one-tick inbox for a delivered frequency hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tactical_frequency_hint: Option<TacticalFrequencyHintInboxState>,
    /// Damage-tier crossing baselines, including exact previous HP.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_system_tiers: Option<LastSystemTiersState>,
    /// Power's brownout notification debounce and same-tick lock edge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power_brownout: Option<PowerBrownoutContinuationState>,
    /// Per-facing Shields down/restore notification debounce.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shields_coordination: Option<ShieldsCoordinationContinuationState>,
    /// In-flight channel-3 messages in enqueue order — see
    /// [`CoordinationQueueState`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordination_queue: Option<CoordinationQueueState>,
    /// The repair crew — see [`RepairState`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repair: Option<RepairState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access: Option<Vec<phoenix_model::interior::BoundaryStatus>>,
    /// The Weapons→Helm arc-bearing seam — see [`ArcRequestState`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arc_request: Option<ArcRequestState>,
    /// The reactor allocation — see [`PowerState`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power: Option<PowerState>,
    /// The ship's fly-through/orbit **pass surface** (`HelmPassSurface`), the
    /// derived helm-leg selection the motion planner steers from.
    ///
    /// This is the state issue #997's duel divergence actually hinged on. The
    /// surface is republished from scratch every AI tick by `ai_policy_state_tick`
    /// — but that system runs `.after(helm_motion_planner)`, so the planner reads
    /// the surface the *previous* tick left behind. A resumed ship whose surface
    /// came back at the bootstrap's value (its own fight, not the captured one)
    /// therefore has its first continuation planner tick select a different leg —
    /// inbound vs escape vs orbit — and steer onto a different bearing two ticks
    /// later, a steering-intent change the digest cannot see until helm
    /// integrates it into yaw.
    ///
    /// The one place a whole component travels rather than a scalar projection,
    /// and the exception is narrow: [`crate::ship::helm_ai::HelmPassSurface`] is a
    /// pure-scalar `Copy` struct with no enum among its fields, so the
    /// variant-order hazard the rest of this module writes scalars to avoid
    /// (see [`WeaponState`]) does not apply — serde's field-name-keyed form is
    /// stable, and a save from a build with a field this one lacks is refused by
    /// the content-version gate, not misread here. Storing it whole is what keeps
    /// the ~25 authored + derived fields impossible to drift out of sync one at a
    /// time. It is a derivation, but one read a tick before it is recomputed, so
    /// the capture-tick value has to survive the restore for the first planner
    /// tick to read — it cannot be rebuilt in time the way `WorldSnapshot` is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pass_surface: Option<crate::ship::helm_ai::HelmPassSurface>,
    /// This ship's shared desired-motion contract as of the capture tick
    /// (issue #1242) — see [`MotionPlanState`].
    ///
    /// `pass_surface`'s sibling, missing for the same reason it was and found the
    /// same way. `HelmMotionPlan` is republished wholesale every AI tick by
    /// `helm_motion_planner`, so it reads like a derivation nothing need store —
    /// but the AI cadence is half the sim cadence, and a capture taken on one of
    /// the ticks in between leaves a restored world holding a DEFAULT plan while
    /// the live one still carries the last one planned. Measured on the duel this
    /// issue was found in: at the restore instant the live cruiser's plan asked
    /// for `desired_facing_local (0.363, 0, -0.932)` and the resumed one for
    /// `(0, 0, -1)` — straight ahead, i.e. no turn at all — and two frames later
    /// its steering had gone to zero while the live ship kept turning. The digests
    /// matched exactly at the restore, because nothing folds this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion_plan: Option<MotionPlanState>,
    /// The entity's infrastructure condition track (issue #1025), whole.
    ///
    /// The second component stored whole rather than projected, and for a
    /// stronger reason than `pass_surface`'s: the track carries *which
    /// operational flags are currently down* alongside the number, and the two
    /// have to come back together. Restore the condition alone and the first
    /// tick after resume re-detects every crossing the mission already spent —
    /// re-firing `on_flag_cleared` on a skyhook that failed twenty minutes ago.
    /// Its `last_hull` is in for the same class of reason: a track that forgot
    /// it would book the structure's entire remaining hull as fresh damage on
    /// the tick after resume.
    ///
    /// The variant-order hazard does not apply — every field is a scalar, a
    /// `String`, or a `Vec` of those.
    ///
    /// # Why this did not bump [`SNAPSHOT_FORMAT`]
    ///
    /// It looks like it should: a save written without this field restores a
    /// degraded structure as intact, while the world flag store — which IS in
    /// the payload — comes back still holding the flags that structure dropped.
    /// That is exactly the "silently wrong, and indistinguishable from correct"
    /// shape a bump exists for.
    ///
    /// It cannot happen. The only saves that lack this field are saves of
    /// worlds written before it existed, and a world gains a structure by
    /// gaining an `[infrastructure]` table in its entity TOML — which moves
    /// `content_digest` and gets the older save refused as content-moved long
    /// before this field is read. A save that could reach the bad state is not
    /// loadable for an unrelated and stronger reason, so the format stays at 4.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub infrastructure: Option<crate::infrastructure::InfrastructureState>,
    /// The ship's tractor-beam state (issue #1156) — whether the beam is engaged
    /// and what it is holding.
    ///
    /// A **projection**, not the whole [`crate::tractor::TractorBeam`] component,
    /// for `infrastructure`'s reason: the authored `[tractor]` coupling terms and
    /// the resolved power group ride the component too and are re-derived from the
    /// template on spawn, so writing them into a save would put content into the
    /// one artefact `content_digest` is answerable for. What travels is the pair
    /// the fold cannot recover — the engage state and the coupled target — which
    /// [`fold_tractor_namespace`](crate::sim_digest) folds and which a resume
    /// would otherwise drop, restoring a hulk as adrift when the crew had it
    /// under tow.
    ///
    /// The variant-order hazard does not apply: every field on
    /// `TractorSaveState` is a `bool`, a `String` or an `Option` of one.
    ///
    /// # Why this did not bump [`SNAPSHOT_FORMAT`]
    ///
    /// `scan`'s argument above, unchanged. A save written without this field
    /// restores a ship as holding nothing when the mission had it towing —
    /// silently wrong — but it cannot happen: a world gains a tractor by its hull
    /// TOML gaining a `[tractor]` table and a `kind = "tractor"` `[[system]]`,
    /// and `EntityConfig` sets `deny_unknown_fields`, so a build that predates
    /// this vocabulary refuses the template outright rather than loading it and
    /// writing a same-content-digest save to disagree with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tractor: Option<crate::tractor::TractorSaveState>,
    /// Canonical Dock channels, mechanical mating state and undock clearance.
    /// Authored component tuning, channel bindings and power-group references
    /// are reconstructed from content on spawn. Every active Dock carries this
    /// state, including idle intent; format 38 refuses older incomplete state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dock: Option<crate::dock::DockSaveState>,
    /// Private tree continuation, including suspended human/offline Systems.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_trees: Option<crate::ai::state_tree_runtime::ShipAiTreeRuntime>,
    /// The ship's transfer-umbilical state (issue #1160) — whether the flow is
    /// running.
    ///
    /// A **projection**, not the whole [`crate::umbilical::TransferUmbilical`]
    /// component, for `dock`'s reason: the authored `[umbilical]` terms and the
    /// resolved power group ride the component and are re-derived from the template
    /// on spawn. What travels is the one thing the fold cannot otherwise recover —
    /// the running intent, which [`fold_umbilical_namespace`](crate::sim_digest)
    /// folds and which a resume would otherwise drop, restoring a live resupply as
    /// stopped. The carry and the last refusal are not persisted: both are
    /// projections the next tick re-derives from the resumed world.
    ///
    /// Did not bump [`SNAPSHOT_FORMAT`] for `dock`'s reason: a world gains an
    /// umbilical by its hull TOML gaining an `[umbilical]` table (and a `kind =
    /// "umbilical"` system), and `EntityConfig` sets `deny_unknown_fields`, so a
    /// build that predates this vocabulary refuses the template outright.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub umbilical: Option<crate::umbilical::UmbilicalSaveState>,
    /// Complete canonical shared Security Jobs. A never-used accepted muster
    /// omits this payload; consumed JobIds, clocks and outcomes survive return.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub security: Option<crate::security::SecuritySaveState>,
    /// The ship's scan record (issue #1032) — the last reading its sensor suite
    /// took, or why the last scan returned nothing.
    ///
    /// A **projection**, not the whole component, for `operations`' reason: the
    /// hull's authored `[scan]` fidelity ladder rides on the component too and
    /// is re-derived from the template on the tick the ship spawns.
    ///
    /// This is the one piece of scan state a fold cannot recover, and that is
    /// exactly why it is here. Everything else about a scan is re-derivable —
    /// the structure's condition, the ship's range, the grid's level — but a
    /// *reading* is what the crew saw when they looked, at the fidelity that
    /// moment bought them, and the structure has moved on since. #1031's
    /// evidence log is stored for the same reason and states it at more length.
    ///
    /// The variant-order hazard does not apply: `ScanRefusal` serialises by
    /// NAME (`#[serde(rename_all = "snake_case")]`), not by index, and every
    /// other field is a scalar, a `String`, or a `Vec` of those.
    ///
    /// # Why this did not bump [`SNAPSHOT_FORMAT`]
    ///
    /// #1025's argument, unchanged and for the same shape of field. A save
    /// written without it would restore a crew as never having scanned when the
    /// mission had them holding a reading — silently wrong, and
    /// indistinguishable from a crew who genuinely had not looked. It cannot
    /// happen: a world gains a scanning ship by its hull TOML gaining a `[scan]`
    /// table, which moves `content_digest` and gets the older save refused as
    /// content-moved long before this field is read.
    ///
    /// The contrast with the two bumps this sits between is worth stating,
    /// because both look similar and neither argument reaches here.
    ///
    /// #1031's evidence is written by a *script call*, so a world could gain
    /// findings with no template change at all and a format-6 payload of a run
    /// that had scanned was byte-indistinguishable from one that had not.
    /// #1035's workforce IS declared in the world file, and still had to bump,
    /// because [`RawWorld`](crate::world::config) sets no `deny_unknown_fields`
    /// — so an older build loads a world authoring `[[workforce]]`, drops the
    /// table on the floor, and writes a save with the SAME content digest.
    ///
    /// That second loophole is precisely the one this field does not have.
    /// `EntityConfig` **does** set `deny_unknown_fields`, so a build that
    /// predates this vocabulary does not quietly ignore a hull's `[scan]` table
    /// — it refuses to load the template at all, and never gets as far as
    /// writing a save to disagree with. A scan reading cannot exist without the
    /// table that produced it, and that table is content the loader is strict
    /// about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scan: Option<crate::science::ScanSaveState>,
    /// Canonical analytic field parameters and their causal logical anchor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region_field: Option<crate::regions::field::RegionFieldState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passive: Option<crate::science::passive::PassiveState>,
    /// The entity's civilian traffic state (issue #1028), whole.
    ///
    /// Stored whole for the same reason the condition track is: the parts have
    /// to come back together. A civilian's lane, its leg, its standing order,
    /// where it stands with that order and the tick that stage is due on are one
    /// fact about a negotiation in progress. Restore the order without the
    /// compliance state and a craft that had already refused starts obeying;
    /// restore the compliance state without the due tick and a craft frozen
    /// mid-acknowledgement answers on the first tick after resume, or never.
    ///
    /// The variant-order hazard applies to exactly one field — `order` is an
    /// enum — and it is handled the way `RON` handles every other: by name, not
    /// by index. Adding a fourth order verb is additive; RENAMING one is not,
    /// and would need the same format bump any other renamed variant does.
    ///
    /// The **route assignment** is not here twice. `CivilianSection` is authored
    /// configuration, re-derived from the entity's TOML at spawn; what the state
    /// carries is the lane it is *currently* on, which a complied divert may
    /// have changed. That is why the section is not captured and this is.
    ///
    /// # Why this did not bump [`SNAPSHOT_FORMAT`]
    ///
    /// Exactly the [`EntityState::infrastructure`] argument, one feature later.
    /// A save that lacks this field is a save of a world written before it
    /// existed; a world gains civilian traffic by gaining a `[civilian]` table
    /// in an entity TOML and `[[route]]` blocks in its world TOML, both of which
    /// move `content_digest` and get the older save refused as content-moved
    /// long before this field is read. The format stays at 5.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub civilian: Option<crate::civilian::CivilianState>,
    /// The entity's debris-contact state (issue #1347) — where the mass has
    /// drifted to, and what the crew have established about it.
    ///
    /// # Why a rock's POSITION is in this field and not in [`Self::physics`]
    ///
    /// Every other moving thing in a save comes back where it was because it
    /// carries `ShipPhysics` and `physics` restores it. Debris carries none: a
    /// rock is not a hull, it has no helm and no integrator, and
    /// `debris::server::tick_debris_drift` is the only writer of its
    /// `Transform` in the simulation. The other candidate — `SpawnOrigin` — is
    /// where the mass was SHED, deliberately (see that type's own note: the
    /// position a spawn consumed, never where the thing has since travelled).
    /// So without this field a resumed corridor puts every mass back at the top
    /// of its run with its deadline reset, and the host's clock and the joiner's
    /// disagree by however long the beat had been running.
    ///
    /// # Why the LATCHES are authoritative and not re-derivable
    ///
    /// `assessed` / `confirmed` / `urgent` / `struck` are the record of what a
    /// crew found out, and the beat turns on the difference between a rock the
    /// simulation knows is on course and a rock somebody has READ. A joiner
    /// that restored the geometry alone would come back with every contact
    /// unread: its Backfilled Tactical would drop the confirmed lock (the
    /// candidate source filters on `confirmed`) and its Sensors seat would
    /// re-scan a field the host had already worked — the same shape of
    /// local-state leak the #1300 GM digest failure was.
    ///
    /// # Why this did not bump [`SNAPSHOT_FORMAT`]
    ///
    /// Exactly the [`EntityState::civilian`] argument. A world gains debris by
    /// gaining a `[debris]` table on an entity template and the scenario script
    /// that sheds it, both of which move `content_digest` and get an older save
    /// refused as content-moved long before this field is read. The format stays
    /// at 5.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debris: Option<crate::debris::DebrisSaveState>,
    /// `ObjectiveCursors` as `(objective id, waypoint index, settled)`.
    ///
    /// Where a patrolling ship is *around its route*, which is not derivable
    /// from where it is in space: a route crosses itself, and the cursor is the
    /// only thing that says which leg the ship is on. A wave NPC restored at
    /// index 0 steers for the start of a lap it was halfway around — and
    /// `simulate_low_lod_ships` snaps its yaw straight at that waypoint, so the
    /// error is instant and total rather than a drift.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub patrol_cursors: Vec<(String, u32, bool)>,
    /// `ShipSystemBlackboards`, sorted by system id — see the type note below.
    ///
    /// # Why a blackboard is authoritative and not a client projection
    ///
    /// AC5 excludes client projections, and a blackboard is *broadcast* to a
    /// console, which makes this look like the same thing. It is not, and the
    /// difference is which direction the arrow points: the console copy is a
    /// projection OF this map, and this map is the **frozen cross-system read
    /// surface** the ship's own AI decides from. `helm_shared_target_view`
    /// reads the Viewscreen blackboard's `combat_lock` and `science_target`
    /// deliberately through this freeze rather than off Tactical's live
    /// selection (issue #829), precisely so a cross-system read cannot reach
    /// another system's synchronous state. Restoring the wire copies to the
    /// browser is not this field's job; restoring what the helm reads next tick
    /// is.
    ///
    /// The measured consequence of leaving it out is written down in
    /// `tests/snapshot_resume.rs`: a resumed ship whose blackboards were the
    /// *bootstrap's* found its own target lock naming a ship its frozen view
    /// had never seen, resolved no travel target, cleared its recovery windows
    /// and fell out of `torpedo_run` into `acquire` on the first AI tick.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blackboards: Vec<(String, crate::core::messages::SystemBlackboard)>,
}

/// Navigation's authoritative shared waypoint, projected to stable scalars.
///
/// `position = None` is a clear waypoint. A present position with no
/// `source_uuid` is a fixed waypoint; with a UUID it is an anchored waypoint
/// whose last-known coordinates are the two position scalars. The outer
/// [`EntityState`] option records component presence, so a default component is
/// still stored as `Some(default)` and clears bootstrap state on restore.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NavigationWaypointState {
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub accepted_requests: std::collections::BTreeMap<
        String,
        crate::console::navigation::server::AcceptedTrafficRequest,
    >,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_uuid: Option<String>,
    pub generation: u64,
}

/// Navigation's authoritative exactly-once clearance issuer frontier.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NavigationClearanceIssueState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issued_generation: Option<u64>,
    #[serde(default)]
    pub helm_axes_were_ai: bool,
}

/// Helm's authoritative latch for the Navigation generation it may follow.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HelmWaypointClearanceState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<u64>,
}

/// A ship's authoritative Shields AI continuation state, projected to scalars.
///
/// `ShieldsDamageHistory` and `PendingShieldsThreatBearing` are runtime ECS
/// types, not stored schema. The payload writes only the decision state they
/// own: per-arc `(recorded_tick, damage)` records, the last-observed HP
/// baseline, and the one-shot pending bearing. The runtime expires records on
/// exact [`SimTick`] boundaries, so the authoritative tick travels verbatim;
/// projecting it through elapsed `f32` seconds can move a record across the
/// strict authored-window boundary by an ULP after restore.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ShieldsAiState {
    /// Per-facing recent damage as `(recorded_tick, amount)`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub damage_by_arc: Vec<Vec<(u64, i32)>>,
    /// Per-facing HP observed on the previous Shields AI decision.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub last_hp: Vec<i32>,
    /// One-shot Sensors bearing waiting for the next Shields AI decision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_threat_bearing_rad: Option<f32>,
}

/// Sensors' authoritative threat-warning debounce memory, projected to
/// serializable scalars.
///
/// A fresh [`crate::ship::sensors::SensorsThreatState`] treats the hostile that
/// was already visible at capture as a new threat and enqueues its bearing a
/// second time. That duplicate reaches Shields only after the authored
/// Coordination delay, making the omission look like a later Shields focus
/// divergence rather than a restore-time gap. These are the exact four values
/// the producer reads to decide whether another warning is due.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SensorsThreatMemoryState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_threat_uuid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_bearing_rad: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_distance: Option<f32>,
}

/// Intent narration's complete per-ship continuation memory.
///
/// The runtime map is projected into station-id order so a `HashMap` seed can
/// never enter the stored bytes. Every [`IntentSnapshotState`] field travels:
/// an axis omitted here can resurface after restore as a decision change that
/// never happened. `generation` is the ship-wide ordering counter already
/// stamped onto queued advisories; it is not a clock.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct IntentNarrationState {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub last: Vec<(String, IntentSnapshotState)>,
    #[serde(default)]
    pub generation: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct IntentSnapshotState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub combat_posture: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hull_fraction: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shield_focus: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub brownout_groups: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manoeuvre: Option<String>,
}

/// Canonical logical combat timestamps; ages cannot retain old f32 event bits.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecentCombatActivityState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_damage_taken_secs: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_hostile_fire_taken_secs: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_weapon_fired_secs: Option<f32>,
    #[serde(default)]
    pub prev_hull: f32,
}

/// High-fidelity Sensors' target-relative frequency-hint delay state.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FrequencyHintContinuationState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_target: Option<String>,
    #[serde(default)]
    pub elapsed_secs: f32,
    #[serde(default)]
    pub hint_sent: bool,
}

/// One NPC ship's delayed phaser-frequency match continuation.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NpcFrequencyMatchState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_target: Option<String>,
    #[serde(default)]
    pub elapsed_secs: f32,
    #[serde(default)]
    pub match_sent: bool,
}

/// The complete NPC frequency-match resource, keyed by stable ship uuid.
///
/// The runtime map uses process-local `Entity` handles. Vector order is uuid
/// order so map seeding cannot enter the stored bytes. `Some(default)` is
/// load-bearing: an authoritative empty map must clear any timers the fresh
/// bootstrap happened to start before restore.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NpcFrequencyMatchStatesState {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub states: Vec<(String, NpcFrequencyMatchState)>,
}

/// A ship's authoritative emitter frequency, projected out of its component.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShipPhaserFrequencyState {
    #[serde(default = "default_ship_phaser_frequency")]
    pub frequency: f32,
}

pub(super) fn default_ship_phaser_frequency() -> f32 {
    crate::ship::state::ShipPhaserFrequency::default().continuation()
}

impl Default for ShipPhaserFrequencyState {
    fn default() -> Self {
        Self {
            frequency: default_ship_phaser_frequency(),
        }
    }
}

/// Tactical's authoritative one-tick frequency-hint inbox.
///
/// The outer [`EntityState`] option records component presence; this inner
/// option records whether a hint is pending. A present empty inbox therefore
/// survives as `Some(default)` and clears a bootstrap hint on restore.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TacticalFrequencyHintInboxState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frequency: Option<f32>,
}

/// Exact damage-tier and HP baselines, each map independently sorted by System
/// id so even an incomplete intermediate map is reproduced rather than joined
/// into an invented entry.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LastSystemTiersState {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tiers: Vec<(String, u8)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hp: Vec<(String, f32)>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PowerBrownoutContinuationState {
    /// Sorted group ids projected from the runtime `HashSet`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notified_groups: Vec<String>,
    #[serde(default)]
    pub locked_changed: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ShieldsCoordinationContinuationState {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub down_notified: Vec<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub restore_notified: Vec<bool>,
}

/// One ship's in-flight channel-3 messages, in authoritative enqueue order.
///
/// The live queue is deliberately not serializable. This projection carries
/// its already-serializable wire envelope plus the scalar sender-origin tag and
/// the number of fixed steps before each message first becomes due. Restore
/// replaces the fresh bootstrap queue wholesale and rebases that discrete
/// boundary onto the destination process clock.
///
/// This is the attached `CoordinationQueue`; top-level
/// [`PhoenixSnapshot::coordination_staging`] separately carries the unread
/// `CoordinationEnqueue` suffix waiting to enter it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CoordinationQueueState {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending: Vec<QueuedCoordinationState>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueuedCoordinationState {
    /// `0 = Human`, `1 = Ai`, `2 = Offline`, `3 = Simplified`.
    pub sender_origin: u8,
    pub address: crate::core::messages::CoordinationAddress,
    pub payload: crate::core::messages::CoordinationPayload,
    pub presentation: crate::core::messages::CoordinationPresentation,
    pub sender_label: String,
    /// Number of future fixed steps, counting the next step as one, until the
    /// lag router's `due_tick <= SimTick` predicate becomes true.
    pub due_after_fixed_ticks: u64,
}

/// One unread `CoordinationEnqueue`, projected out of Bevy's message buffers.
///
/// The event's ECS `Entity` handle is process-local, so the payload stores the
/// source ship's stable uuid and resolves it only after entity restoration.
/// Every other field is already a scalar or the serializable Coordination wire
/// vocabulary. Vector position is the message sequence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CoordinationEnqueueState {
    pub source_uuid: String,
    /// `0 = Human`, `1 = Ai`, `2 = Offline`, `3 = Simplified`.
    pub sender_origin: u8,
    pub address: crate::core::messages::CoordinationAddress,
    pub payload: crate::core::messages::CoordinationPayload,
    pub presentation: crate::core::messages::CoordinationPresentation,
    pub sender_label: String,
    pub sender_system: String,
}

/// A ship's **weapon state machines**, named by this issue's AC2.
///
/// Every field here is a machine that is *mid-something* at the capture tick,
/// and a resumed ship whose machines came back cold is a different ship: a beam
/// that was two thirds through its burn stops burning, a bank that was on
/// cooldown is free to fire, a tube that was three seconds into a load is
/// empty, and a shield arc that had taken a broadside is whole again. Each of
/// those changes what happens on the *first* tick after a restore, which is
/// exactly the window a digest match at the instant of restore cannot see.
///
/// # Runtime only, never authored
///
/// The rule [`EntityState::hull`] states is kept here throughout: what is
/// stored is what the run *changed*, never what the TOML said. A tube's
/// `facing_deg`, `fire_arc_deg`, `volley_max`, `load_time`, barrel names and
/// firing pattern are all authored, the fresh world rebuilt them, and a save
/// that disagreed about them is a save the content-version gate refuses. Only
/// the load state, the counts and the last-fired markers travel.
/// Already committed guided rounds also carry their frozen physical profiles:
/// these are launch facts, not values re-read from a later launcher or refit.
///
/// # Written as scalars
///
/// For [`ControlState`]'s reason: a `derive` on `TubeLoadState` would silently
/// make its variant *order* stored surface, and a scalar written out at the
/// call site makes that commitment visible where it is made.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WeaponState {
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub beam_impact_samples:
        std::collections::BTreeMap<String, (String, phoenix_model::damage::HitSample)>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub beam_strike_bonuses: std::collections::BTreeMap<String, f32>,
    /// Every live phaser beam as `(bank, target uuid, remaining_secs,
    /// damage_accumulator, pending_cooldown_secs, operation)`, in the bank order
    /// `ActiveBeam` already keeps.
    ///
    /// `damage_accumulator` is in the tuple deliberately. It is the fractional
    /// damage carried between ticks so that 5 HP/s applies accurately at any
    /// frame rate; a beam restored without it is a beam whose sub-tick debt was
    /// forgiven, and `ActiveBeam::restore_live_banks` exists so that a restore
    /// cannot round-trip through `start` and lose it.
    ///
    /// `pending_cooldown_secs` is here for the same class of reason and a
    /// sharper one (issue #929). A bank with `cycle_jitter` authored draws ONE
    /// factor when it lights and applies it to that cycle's burn AND to the
    /// cooldown behind it; the cooldown is therefore decided at light time and
    /// carried on the slot. A resume that dropped it would serve the AUTHORED
    /// rest instead of the drawn one — the resumed ship would silently fall back
    /// onto the fixed cadence for that cycle, which is exactly the
    /// de-synchronisation the field exists to create. Carrying it means a run
    /// restored mid-burn continues the cycle it was in rather than redrawing,
    /// and it is why this mechanism did not need a new snapshot seam at all: the
    /// state rides on a component the payload already captured.
    /// `operation` is frozen at light time. It is required for each active slot
    /// so a resumed sustained burn cannot silently become a timed fixed cycle.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub beams: Vec<(
        String,
        String,
        f32,
        f32,
        f32,
        phoenix_model::weapon_operation::BeamOperation,
    )>,
    /// `(bank, remaining_secs)` for every bank still cooling, in bank order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub phaser_cooldowns: Vec<(String, f32)>,
    /// Every tube's contents and load machine, in the tube order the hull
    /// authored.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tubes: Vec<TubeState>,
    /// The shared magazine. `None` when this ship has no torpedo system at all,
    /// which is how a hull with no tubes stays distinguishable from one that
    /// has shot itself dry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub magazine: Option<crate::resources::Stock>,
    /// Torpedoes in the air, which are authoritative in the plainest sense —
    /// they are moving, steering and about to detonate on somebody.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub torpedoes_in_flight: Vec<TorpedoInFlight>,
    /// Volleys part-way through their burst cadence.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bursts: Vec<BurstState>,
    /// Per-arc shield hull as `(arc id, current, max)` in the arc order the
    /// TOML declared — `ShipArcHull`'s own iteration order, which it keeps
    /// separately from its map for exactly this determinism reason.
    ///
    /// This is the STRUCTURAL hull behind the emitters, not the shield CHARGE
    /// the arcs are currently holding — those are two different quantities and
    /// `shield_charge` below carries the other one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub arc_hull: Vec<(String, f32, f32)>,
    /// Per-facing shield CHARGE as `(arc id, hp, hp_frac, offline_remaining,
    /// is_focused)`, in `ShieldSystem.facings` order — the fractional
    /// deterministic sim state a resume was silently dropping (issue #997
    /// follow-up: a restored world defaulted every arc back to full charge).
    ///
    /// `hp_frac` rides along for `beams`' `damage_accumulator` reason: it is the
    /// sub-tick regen/decay carried between fixed-timestep ticks, and a charge
    /// restored without it is a charge whose fractional debt was forgiven.
    /// `max_hp` is deliberately absent — it is not runtime charge but a
    /// focus-derived value `ShieldSystem::restore_facings` rebuilds from each
    /// arc's TOML baseline, so storing it would commit a derived quantity to
    /// the wire.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shield_charge: Vec<(String, i32, f32, f32, bool)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shield_operations: Vec<crate::weapons::shield::ShieldOperationState>,
    /// Every blaster bank's volley/cooldown cycle and bolts, in the authored
    /// bank order the component keeps — see [`BlasterRuntime`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blasters: Vec<BlasterRuntime>,
}

/// One torpedo tube's runtime state.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TubeState {
    pub id: String,
    /// `TubeLoadState` as `0` = `Unloaded`, `1` = `Loading`, `2` = `Loaded`,
    /// `3` = `Unloading`. Anything else restores as `Unloaded` rather than
    /// panicking, for [`ControlState::impulse_phase`]'s reason.
    pub load_phase: u8,
    /// The `Loading`/`Unloading` timer, `(remaining, total)`. Zeroes for the
    /// two settled phases.
    pub load_timer: [f32; 2],
    pub loaded_count: u32,
    pub target_count: u32,
    /// Barrel indices the most recently launched round left from, and the
    /// 1-based pattern step it came from. Both are read to render the Tactical
    /// indicator and to pick the *next* barrel, so a tube restored without them
    /// resumes its firing pattern from the wrong place.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub active_barrels: Vec<u32>,
    pub pattern_step: u32,
}

/// One torpedo mid-flight.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TorpedoInFlight {
    pub last_guidance: Option<phoenix_model::guided_guidance::GuidanceObservation>,
    pub profile: phoenix_model::guided_round::GuidedRoundProfile,
    pub durability_remaining: f32,
    #[serde(default)]
    pub strike_damage_bonus: f32,
    pub uuid: String,
    pub position: [f32; 3],
    pub heading: f32,
    pub pitch: f32,
    pub lifespan_remaining: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_uuid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_uuid: Option<String>,
    pub tube_id: String,
}

/// One tube's pending burst.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BurstState {
    pub launch_uuid: String,
    pub profile: phoenix_model::guided_round::GuidedRoundProfile,
    pub burst_interval_secs: f32,
    pub tube_id: String,
    pub pending: u32,
    pub timer: f32,
    /// The launch origin and heading captured at fire time; every shot of the
    /// volley leaves from here, so it is state and not a derivation.
    pub launch: [f32; 3],
    pub launch_heading: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_uuid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_uuid: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub barrel_origins: Vec<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub barrel_sequence: Vec<(u32, u32)>,
    pub next_shot_index: u32,
}

/// One blaster bank's runtime **volley + cooldown** cycle and its bolts in the
/// air (issue #997).
///
/// The blaster analogue of [`WeaponState::phaser_cooldowns`] and the tube/burst
/// state — and the piece whose absence issue #997's duel divergence turned out
/// to hinge on. The WEAPONS DOCTRINE (`tick_weapons_arc_request`) chooses which
/// family to ask Helm to bring an arc onto by walking the ship's families and
/// stopping at the first whose emitters are ONLINE and USABLE; a bank on
/// cooldown is not usable. A resumed destroyer whose blasters came back at the
/// authored default — ready, no cooldown — is usable when the live ship's are
/// still cooling, so its doctrine picks a different family, emits a different
/// arc-bearing request, and its Helm steers onto a different bearing. That is
/// the destroyer's yaw parting company two ticks after a restore whose digest
/// matched exactly.
///
/// Only the run-changed volley cycle and the bolts travel; the bank's authored
/// `config` (arc, cadence, damage, barrels, pattern) is rebuilt from TOML, the
/// rule [`WeaponState`] keeps throughout.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BlasterRuntime {
    /// Steps left to fire in the active volley (`0` when idle).
    pub pending_volley: u32,
    /// The resolved active firing schedule — `(barrel indices, at_secs)` steps.
    /// Stored rather than re-derived because the cursor below indexes it, and a
    /// re-derivation could disagree with a config that changed between builds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub schedule: Vec<(Vec<u32>, f32)>,
    pub next_step: u32,
    pub volley_elapsed: f32,
    /// Barrels that fired on the most recent step — read for the Tactical
    /// indicator and to pick the next barrel, so a bank restored without them
    /// resumes its pattern from the wrong place, as a tube does.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub active_barrels: Vec<u32>,
    pub current_step: u32,
    pub on_cooldown: bool,
    pub cooldown_remaining: f32,
    pub charging: bool,
    pub charge_elapsed: f32,
    /// Bolts this bank has in flight — moving, and about to hit somebody, so
    /// authoritative in the plainest sense (as torpedoes in flight are).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub in_flight: Vec<BlasterBolt>,
}

/// One blaster bolt mid-flight — see [`BlasterRuntime::in_flight`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BlasterBolt {
    pub impact: phoenix_model::damage::DamageProfile,
    pub id: String,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub pitch: f32,
    pub visual_scale: f32,
    pub heading: f32,
    pub speed: f32,
    pub lifespan_remaining: f32,
    pub collision_radius: f32,
    pub damage: i32,
    pub shield_pierce: f32,
    pub source_uuid: String,
}

/// A ship's **repair state**, the other half of AC2's "weapon/repair state".
///
/// A repair team is a timer with a destination. Restored idle, every team that
/// was three seconds into a five-second walk arrives late — or never, because
/// the dispatch that sent it is not re-issued — and the systems they were
/// mending stop mending. The queue and the alert latch travel with them for the
/// same reason: a queue restored empty re-raises requests the capture had
/// already spent, and `RepairHumanAlerted` is precisely the latch that stops a
/// crew being told twice.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RepairState {
    pub readings: Vec<(
        crate::core::messages::RepairGroup,
        Vec<crate::console::repair::server::RepairSourceReading>,
    )>,
    /// One canonical job/team owner, including external commitments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jobs: Option<phoenix_sim_gameplay::jobs::RepairJobState>,
    /// Pending requests as `(repair group, label, tier, deficit)`, in the
    /// severity order the queue keeps.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub queue: Vec<(
        crate::core::messages::RepairGroup,
        String,
        crate::ship::damage::DamageTier,
        f32,
        Vec<phoenix_model::observation::ObservationMetadata>,
    )>,
    /// The "this crew has already been told" latch, as `(repair group, tier)`
    /// sorted by id — the component is a `HashMap`, and a payload may not
    /// inherit its iteration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alerted: Vec<(
        crate::core::messages::RepairGroup,
        crate::ship::damage::DamageTier,
    )>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recoveries: Vec<(
        crate::core::messages::RepairGroup,
        phoenix_model::observation::ObservationMetadata,
    )>,
}

/// The Weapons→Helm **arc-bearing seam** (issues #677/#767), both halves of it.
///
/// This is the divergence issue #997 was opened for. The seam is a debounce on
/// the Weapons side and a pending bearing on the Helm side, and the two are one
/// piece of state: `tick_weapons_arc_request` only emits a channel-3
/// [`CoordinationPayload::ArcBearingRequest`] when its debounce key *changes*,
/// and the request it emits lands in Helm's [`PendingArcBearingRequest`], which
/// `ai_helm_steering` folds into the steering bias every tick until the geometry
/// self-clears.
///
/// A ship captured mid-engagement holds a *settled* debounce (`last = Some(key)`,
/// already emitted) and a *pending* bearing Helm is still folding in. A resumed
/// ship that booted both `default()` has `last = None` — so the first
/// `ai_tick_ready` cadence tick after the restore re-fires a request the live
/// world suppresses — and an empty pending bearing, so its helm steers without
/// the bias the live ship still carries. Either alone changes steering intent,
/// which the digest cannot see (it folds `ShipPhysics`, not steering input) until
/// helm integrates it into yaw ~2 ticks later. That is exactly the lagged, opt-
/// level-independent divergence #997 measured.
///
/// # Stored as its wire types, not scalars
///
/// [`WeaponFamily`] and [`WeaponEmitterArc`] already derive serde — they are the
/// channel-3 payload types, so their shape was stored surface the repository had
/// committed to before this module existed. Copying them into scalars here would
/// not remove that commitment, only duplicate it — the historical TeamSlot
/// exception, for the same reason.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ArcRequestState {
    /// `WeaponsArcRequestState.last` — the settled debounce key
    /// `(family, target uuid, usable arcs)`. `Some` means a request for this key
    /// has already been emitted and must *not* re-fire on the first cadence tick
    /// after the restore.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<(
        crate::core::messages::WeaponFamily,
        String,
        Vec<crate::core::messages::WeaponEmitterArc>,
    )>,
    /// `PendingArcBearingRequest.target` — the uuid Helm is biasing to bring a
    /// weapon arc onto, or `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_target: Option<String>,
    /// `PendingArcBearingRequest.arcs` — the emitting family's usable ONLINE
    /// emitter arcs Helm folds into its steering bias and self-clears against.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_arcs: Vec<crate::core::messages::WeaponEmitterArc>,
}

/// Runtime state of the two helm drives outside the High-fidelity bundle.
///
/// `BoostCommand` and `ImpulseCommand` are desired transitions; these are the
/// battery/charge machines integration actually reads. They must travel
/// separately because inserting a High bundle marks its commands as Added,
/// which the normal transition system intentionally ignores on promotion.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DriveState {
    /// `ShipImpulse.phase`: 0 = Idle, 1 = Charging, 2 = Active.
    pub impulse_phase: u8,
    pub impulse_charge_progress: f32,
    /// Previous Input-phase HP, not current HP: Damage may have run since it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub impulse_previous_hull_hp: Option<f32>,
    pub boost_active: bool,
    pub boost_battery: f32,
}

/// One entity's authoritative AI LOD tier and transition boundary.
///
/// `AiHighFidelity` is a marker, but not a derivation a restore may leave to the
/// next LOD pass: every high-fidelity policy gates on it, and demotion is
/// deliberately delayed. The marker therefore travels as an explicit boolean.
/// [`crate::ai::server::LodTransitionTimer`] is separate from the canonical
/// high-fidelity bundle and its absence is meaningful (a ship in its first Low
/// stretch has never transitioned), so its presence is preserved independently.
///
/// The canonical logical timestamp is copied exactly so the first eligible
/// demotion tick cannot move when a fresh host has a different elapsed epoch.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiFidelityState {
    #[serde(default)]
    pub high_fidelity: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_transition_secs: Option<f64>,
}

/// One asteroid's authoritative state.
///
/// A rock's position is authoritative because it is what a collision resolves
/// against — the digest's own reason for folding it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AsteroidState {
    pub uuid: String,
    pub translation: [f32; 3],
    /// The rock's orientation, `[x, y, z, w]`. Not folded by the digest — a
    /// tumbling rock collides the same either way — but stored because a
    /// restore now *spawns* rocks, and a spawned rock with a default rotation
    /// is a visibly different rock from the one that was saved.
    #[serde(default)]
    pub rotation: [f32; 4],
    /// `(SystemId, current, max)`, as [`EntityState::hull`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hull: Option<Vec<(String, f32, f32)>>,
    /// The rock's entity TOML path, and the reason it is here is [`restore`]'s
    /// spawn path: to *build* a missing rock the restore needs its collider,
    /// mesh, tags and radar appearance, and all four are read from this file
    /// rather than stored. It is joined from the streaming window's slot data,
    /// which is the only place a rock's config path survives after spawn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_path: Option<String>,
    /// The rock's `AsteroidShieldPierce`, which is per-*field* tuning resolved
    /// at spawn from whichever contribution the composed evaluator picked. That
    /// makes it neither authored-per-rock nor recomputable without re-running
    /// the evaluator, so it travels with the rock.
    #[serde(default)]
    pub shield_pierce: f32,
}

/// The asteroid streamer's own progress, and the reason AC1's world needs it.
///
/// Combat Test's belts are *streamed*: a rock exists when the player's cell
/// window covers it, so a fresh app bootstrapped at the spawn point has a
/// different rock population than a capture taken after the player has flown
/// somewhere. Restoring the rocks without restoring the window that owns them
/// closes half the gap and opens a worse one — the streamer would still believe
/// it was anchored where the fresh boot left it, and the very next
/// `update_asteroid_window` tick would full-rebuild the belt out from under the
/// restore.
///
/// With the anchor, the player cell and the composition key all put back, that
/// tick recomputes the same cell from the restored ship's position, finds it
/// unchanged, and returns without touching anything. The streamer resumes
/// rather than restarting.
///
/// Cosmetic slots are **not** here: they hold raw `Entity` handles, which this
/// payload never stores (see the module docs), and they are set dressing with
/// no uuid, no hull and no collider. [`restore`] clears them so the streamer
/// repopulates them on its next scroll.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AsteroidWindowState {
    pub arena_gx: i32,
    pub arena_gz: i32,
    pub despawn_cells: u32,
    pub spawn_cells: u32,
    pub resolution: f32,
    /// The player's lattice cell as of the last streamer tick. `None` before
    /// the first one has run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player_grid: Option<(i32, i32)>,
    /// The fingerprint of the contribution set the window's contents were built
    /// from. Restored so the streamer does not read its own live fields as a
    /// composition change and rebuild.
    pub composition_key: u64,
    pub needs_init: bool,
    /// The occupied slots, sorted by `(z, x)` so the payload is byte-stable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slots: Vec<WindowSlot>,
}

/// One occupied ring-buffer slot.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WindowSlot {
    pub x: u32,
    pub z: u32,
    pub uuid: String,
    pub config_path: String,
    pub hp: i32,
    pub max_hp: i32,
    pub y: f32,
}

/// One active world layer in the capture's authoritative activation order.
///
/// The vector of these rows is the topology: `path` identifies the layer,
/// `loader_path` preserves its parent flag scope, and vector position preserves
/// the order in which scripted triggers were appended to the shared trigger
/// table. The absolute runtime activation ordinal is deliberately not stored;
/// only the relative order of layers that are still active can affect handler
/// alignment after a resume.
///
/// Failed-load sentinels in `WorldLayerMap` are not layers and do not appear
/// here. A genuinely active but otherwise empty layer does, which is why capture
/// tests `WorldRuntime::is_active` rather than inferring success from its fields.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerFlags {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loader_path: Option<String>,
    /// Captured UUIDs aligned with `WorldRuntime::spawned_entities`.
    ///
    /// `Some` is a live entity declared by this layer's `[[entity]]` block.
    /// `None` is either a dead slot or a script-spawned entity; script spawns
    /// already carry their own serialisable `EntitySpawnOrigin` recipe. Keeping
    /// the slots preserves declaration order so a fresh activation can exchange
    /// its newly-minted UUIDs for the captured identities before entity restore.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub declared_entity_uuids: Vec<Option<String>>,
    /// Retained Objective ownership in the layer's original mutation order.
    /// The unload path consumes this list even after the activation journal
    /// has already been applied; recreating the palette does not recreate it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub owned_objective_ids: Vec<String>,
    pub flags: FlagStore,
}

/// The **scenario-progression** state a scripted world resumes from (issue
/// #864).
///
/// `flags` above is the half of a scenario's memory that already travelled: the
/// counters a `when` predicate reads. This is the other half — *what has
/// already happened*, and *what is still owed*. A resumed scenario without it
/// stands at a matching digest and then replays its own opening: every
/// single-shot trigger the run had spent is re-armed and fires a second time,
/// every `after(n, |ctx| …)` callback the run was waiting on has been forgotten,
/// and the mission clock those firings are measured against has been rewound to
/// the age of whatever fresh app was booted to restore into.
///
/// Mission age is stored as an exact Duration (integer seconds and nanoseconds). The runtime rebases that age
/// against the resumed process's own fixed clock using Duration arithmetic;
/// authored seconds remain ordinary f32 projections of that one exact age.
///
/// # Honestly not covered
///
/// `pending_delayed_actions` (the declarative `action_delays` queue, which a
/// scripted `ctx.schedule.in_seconds(n).<verb>(…)` also feeds) is **not** here.
/// Carrying it means giving [`crate::world::config::TriggerAction`] — 22
/// variants over `AiDirective`, `UtilityConfig`, `ObjectiveSource`,
/// `ModifierSlot`, `IntModifierSlot` and `crate::core::balance::Outcome` — a serde
/// derive, which would pin six *authored-config* types' shape as save format.
/// This module refuses that commitment everywhere else it comes up (see
/// [`PhoenixSnapshot::game_over`], which stores `Outcome` as a label precisely
/// so its variant order does not become stored surface), and no shipped world
/// authors `action_delays` or `in_seconds` today — the deferral vocabulary the
/// shipped scripted set actually uses is `after(..)`, which is
/// [`Self::script_callbacks`]. Widening to the action queue is a separate
/// issue's commitment, not a line to slip in here.
///
/// A layer's own trigger state is not stored separately either, for a narrower
/// reason: a layer's states are *merged into* the base trigger registry when
/// its `[script]` set compiles at load (issue #1045) and removed from it at
/// unload, so [`Self::triggers`] already carries every trigger that can fire.
/// What identifies them is the `origin_layer` tag on each state — matched by
/// `world::trigger_registry::WorldTriggerRegistry::remove_layer` — and a resumed world rebuilds
/// them by re-running the same layer loads.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ScenarioState {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub opportunities: Vec<crate::world::opportunities::OpportunityRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_choice: Option<phoenix_sim_world::world::planning::PlanChoice>,
    #[serde(
        default,
        skip_serializing_if = "phoenix_sim_world::world::activities::SituationInstances::is_empty"
    )]
    pub situations: phoenix_sim_world::world::activities::SituationInstances,
    /// Exact mission age; None before the mission clock is anchored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mission_elapsed: Option<std::time::Duration>,
    /// One row per live trigger state, in `WorldContentRuntime::triggers`
    /// order — see [`TriggerRuntimeState`]. Every row is written, not just the
    /// fired ones, so the count is itself the alignment check the restore makes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub triggers: Vec<TriggerRuntimeState>,
    /// `(group, member names)` sorted by group, members sorted — the membership
    /// an `on_all_destroyed group = "…"` condition is judged against. A
    /// `HashMap` of `HashSet`s in the runtime; a payload may not inherit either
    /// iteration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entity_groups: Vec<(String, Vec<String>)>,
    /// `(entity name, uuid)` sorted by name: `WorldContentRuntime::name_to_uuid`,
    /// the map every name-carrying command in `world::dispatch` resolves through
    /// (issue #863).
    ///
    /// It belongs beside `entity_groups` for the reason that field is here — both
    /// are what a scenario knows about *which* entity it means — and it was left
    /// out until now for a reason that stops being true the moment a restore can
    /// spawn: a fresh boot rebuilt the map by re-running the same spawns, and the
    /// mint being deterministic meant it rebuilt it with the same uuids. A
    /// restore-spawned entity has no such re-run behind it, so the name it
    /// answers to has to travel with it or the resumed scenario cannot destroy
    /// it, cannot target it, and cannot notice it dying.
    ///
    /// Sorted, because a `HashMap`'s iteration order is not a payload's to
    /// inherit.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub name_to_uuid: Vec<(String, String)>,
    /// `(uuid, aggregate hull fraction)` sorted by uuid: the last sample
    /// `collect_world_events` compares against to decide whether a hull crossed
    /// *downward*.
    ///
    /// Restored because leaving it out manufactures an event. A fresh app's
    /// ships are whole, so its last sample is ~1.0; the restore then writes the
    /// capture's mauled hull underneath it, and the very next tick reads that
    /// as a fresh downward crossing and fires every `on_hull_below` template the
    /// captured run had already spent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub observed_hull_fractions: Vec<(String, f32)>,
    /// `WorldContentRuntime::pending_world_events`, in queue order — see
    /// [`WorldEventRecord`]. Usually empty at a tick boundary; non-empty exactly
    /// when the previous tick's delayed actions or script callbacks queued a
    /// chaining event for the next one, which is the tick a capture can land on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_events: Vec<WorldEventRecord>,
    /// `WorldScriptRuntime::pending_callbacks`, **in queue order**.
    ///
    /// Not sorted, and that is the deliberate reading of this module's
    /// stable-key rule rather than an exception to it. The rule exists because a
    /// payload must not inherit a `HashMap`'s iteration order; this queue is a
    /// `Vec` whose order is the order the scripts scheduled into it, which is
    /// already a deterministic function of the run every peer reproduces — and
    /// it is *load-bearing*, because `PendingCallbacks::drain_due` fires due
    /// callbacks in exactly this order and their effects apply in that order.
    /// Sorting the payload would hand the resumed world a different firing
    /// order than the live one it is being compared against. Each entry is a
    /// [`ScheduledCall`] — `(fire_tick, script_path, fn_name)` — which is
    /// already the stable key the rule asks for: no AST, no closure, no handle.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub script_callbacks: Vec<ScheduledCall>,
    /// `WorldContentRuntime::deadlines`, whole (issue #1024).
    ///
    /// The named half of the queue above. [`Self::script_callbacks`] already
    /// carries the `ScheduledCall` a pending deadline is waiting on — but the key
    /// is `(fire_tick, script_path, fn_name)` and nothing in it says *which*
    /// deadline, whether the crew can see it, or whether the two that are NOT in
    /// the queue were cancelled or already fired. Restoring the queue without the
    /// table therefore resumes a mission whose deadlines have no names and no
    /// history: a cancelled `stabiliser_failure` comes back armed at its authored
    /// time, and a window the crew bought two minutes on comes back due when it
    /// originally was.
    ///
    /// Stored as the whole [`DeadlineTable`] rather than a per-field projection,
    /// which is [`EntityState::pass_surface`]'s exception, taken for its reason:
    /// every field is a scalar, a `String`, or the already-stored
    /// [`ScheduledCall`], with one small enum
    /// ([`DeadlineState`](crate::world::deadlines::DeadlineState)) that travels by
    /// its own `snake_case` serde name rather than by variant order. There is
    /// nothing here whose shape a save would pin that this payload does not pin
    /// already — and writing it whole is what keeps the table impossible to drift
    /// out of sync with the queue one field at a time.
    ///
    /// In authored order, never sorted, for [`Self::script_callbacks`]' reason:
    /// the order is the world file's, it is what the panel renders, and it is
    /// already deterministic across peers.
    #[serde(default, skip_serializing_if = "DeadlineTable::is_empty")]
    pub deadlines: DeadlineTable,
    /// `WorldContentRuntime::commitments`, whole (issue #1029).
    ///
    /// The promises the run made, with who each was made to, its terms, its
    /// stated resolution condition, whether it ended up kept or broken, and the
    /// ticks at both ends.
    ///
    /// Unlike every other field on this struct, **nothing in the world file
    /// predicts whether this is empty**: a promise exists because of what the
    /// player said, not because of what an author declared. That is why the
    /// content digest cannot stand in for the format bump here — see
    /// [`SNAPSHOT_FORMAT`] — and it is also why the whole ledger is stored
    /// rather than a projection of the open ones: a resumed run that could not
    /// tell a kept promise from one never made would let the crew give the same
    /// word twice.
    ///
    /// In the order the promises were made, never sorted, for
    /// [`Self::deadlines`]' reason.
    #[serde(default, skip_serializing_if = "CommitmentLedger::is_empty")]
    pub commitments: CommitmentLedger,
    /// `WorldContentRuntime::evidence`, whole (issue #1031).
    ///
    /// What the crew found out, what each finding was about, how they learned it
    /// and on which tick.
    ///
    /// Stored for [`Self::commitments`]' reason — nothing in the world file
    /// predicts whether it is empty, because a finding exists because of what the
    /// crew *did* — and stored WHOLE for [`Self::deadlines`]' reason: every field
    /// is a `String`, a `u64`, or one small enum travelling by its own
    /// `snake_case` serde name.
    ///
    /// This is the only dossier state a save carries, and the distinction is the
    /// whole of `src/dossier/`: the facts on a sheet are re-folded from the
    /// condition track, the ledger and the roster every tick and would disagree
    /// with them if persisted, whereas what the crew learned is recoverable from
    /// nothing at all.
    ///
    /// In the order the findings were made, never sorted, for
    /// [`Self::deadlines`]' reason — and here the order is also what the fact
    /// sheet renders.
    #[serde(default, skip_serializing_if = "EvidenceLog::is_empty")]
    pub evidence: EvidenceLog,
    /// `WorldContentRuntime::workforce`, whole (issue #1035).
    ///
    /// Which sides of the world's labour dispute are out right now, what each
    /// makes of the crew, and whether the register has been armed at all.
    ///
    /// The `armed` latch is the field that makes this necessary rather than
    /// merely tidy. Without it a resumed mission's first tick re-arms from the
    /// world file and puts a settled strike straight back on — the loudest
    /// possible way to lose a negotiation the crew already won, and the same
    /// class of silent re-arming that carried [`Self::deadlines`] into the
    /// payload. The two live facts have to come back with it: restore the latch
    /// without the records and the register is armed and empty, so every
    /// structure reads as worked.
    ///
    /// Stored as the whole register rather than a per-side projection, for
    /// [`Self::deadlines`]' reason: every field is a scalar, a `String` or a
    /// `bool`, there is no enum among them, and writing it whole is what keeps
    /// three facts about one side impossible to drift apart one at a time.
    ///
    /// In authored order, never sorted — the world file's order, which every
    /// peer reads the same way.
    #[serde(
        default,
        skip_serializing_if = "crate::world::workforce::WorkforceRegister::is_empty"
    )]
    pub workforce: crate::world::workforce::WorkforceRegister,
    /// `WorldContentRuntime::pending_gm_event_fires` (issue #1301): the
    /// layer-qualified ids of GM Fires that have crossed their canonical apply
    /// boundary and whose handler has not run yet.
    ///
    /// It has to travel for [`Self::deadlines`]' reason, sharpened: the
    /// authoritative fact that the Fire was Applied lives in the GM action
    /// journal, which this payload already carries, so a resume WITHOUT this
    /// field reports an event fired that never ran its handler — and, because a
    /// second Fire of the same one-shot reduces to a No-op against that same
    /// journal, the GM could never make it run either.
    ///
    /// A `BTreeSet` in the runtime, written here in its own sorted order: this
    /// is the one collection on this struct whose order is neither authored nor
    /// a queue, so sorting IS its identity rather than a payload convention.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_gm_event_fires: Vec<String>,
    /// `WorldContentRuntime::pending_gm_spawns` (issue #1305): the GM
    /// placements that have crossed their canonical apply boundary and whose
    /// hulls have not been spawned yet.
    ///
    /// It travels for [`Self::pending_gm_event_fires`]' reason exactly: the
    /// authoritative fact that the placement was Applied lives in the GM action
    /// journal this payload already carries, and the journal's idempotency
    /// means a second press under a new correlation is a SECOND hull rather
    /// than a retry — so a resume without this field silently loses the one the
    /// operator was told they placed, with no way to get it back that does not
    /// also risk two.
    ///
    /// A `Vec` written in the runtime's own order, which is canonical grant
    /// order: unlike the sorted fire set, the sequence here decides which
    /// placement draws which uuid from the `WorldIdMint`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_gm_spawns: Vec<crate::gm::operations::spawn::PendingGmSpawn>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_gm_despawns: Vec<String>,
    /// The retained inverse data for allowed GM removals, and its two cross-tick
    /// arms (issue #1444).
    ///
    /// Additive, and deliberately without a [`SNAPSHOT_FORMAT`] bump: an older
    /// save simply has none, and a run restored from one is honestly told so —
    /// its journal's removal rows carry no captured pair, the projection reports
    /// the capture as lost and no Undo control is offered. That is the whole
    /// difference, and it is exactly the answer this build would give for a
    /// capture the run's own bound had evicted.
    ///
    /// It has to travel for [`Self::pending_gm_despawns`]' reason, twice over: an
    /// accepted removal's capture is armed in `PreUpdate` and taken in
    /// `FixedUpdate`, and an accepted inverse is armed and executed the same way,
    /// so a capture taken between the two halves of either would otherwise
    /// resume a world whose journal reports work nothing will ever do.
    #[serde(
        default,
        skip_serializing_if = "crate::gm::operations::despawn_undo::GmDespawnCaptures::is_empty"
    )]
    pub gm_despawn_captures: crate::gm::operations::despawn_undo::GmDespawnCaptures,
    // Historical scenarios must parse so the version gate can refuse them
    // while the save catalogue still retains their scenario/seed/tick metadata.
    #[serde(default)]
    pub contact_overrides: crate::gm::operations::contact::ContactOverrides,
    /// M7 interpreted contact identity, scoped to the observing player ship.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub contact_classifications: crate::gm::operations::contact::ContactClassifications,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub presentation: crate::gm::operations::presentation::PresentationState,
    #[serde(
        default,
        skip_serializing_if = "crate::gm::operations::information::ContactInformation::is_empty"
    )]
    pub contact_information: crate::gm::operations::information::ContactInformation,
    /// `WorldContentRuntime::paused_gm_events` (issue #1303): the
    /// layer-qualified ids of GM-operable events a Game Master has paused.
    ///
    /// It has to travel for [`Self::pending_gm_event_fires`]' reason with the
    /// sign reversed. A paused event's trigger is not evaluated at all, so a
    /// resume WITHOUT this field silently un-pauses every paused event: the
    /// restored world starts evaluating conditions the GM deliberately stopped,
    /// while the journal this payload already carries still says the Pause was
    /// Applied and a second Pause of it would therefore reduce to a No-op. The
    /// GM would be told the event is paused, watch it fire, and have no lever
    /// left that could change either fact.
    ///
    /// Written in the `BTreeSet`'s own sorted order, for
    /// [`Self::pending_gm_event_fires`]' reason.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paused_gm_events: Vec<String>,
    /// `WorldContentRuntime::pending_gm_event_skips` (issue #1304): the
    /// layer-qualified ids of GM Skips that have crossed their canonical apply
    /// boundary and whose occurrence has not happened yet.
    ///
    /// It travels for [`Self::pending_gm_event_fires`]' reason, and it is the
    /// LONGER-lived of the two: a Fire arm is normally consumed the tick it
    /// lands, while a Skip arm is waiting on the world and can legitimately
    /// outlive any number of saves. A resume without it runs the occurrence the
    /// GM decided the crew would not see, with the journal still reporting the
    /// arm Applied.
    ///
    /// Sorted in the `BTreeSet`'s own order, for the field above's reason.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_gm_event_skips: Vec<String>,
}

/// One queued `WorldEvent`, written as a tag plus its fields.
///
/// Written out rather than by deriving serde on
/// [`crate::world::content::WorldEvent`], for [`ControlState`]'s reason: a
/// derive would make that enum's shape stored surface, and a tag written at the
/// call site makes the commitment visible where it is made. An unrecognised
/// `kind` is dropped on restore rather than panicking — the same rule
/// [`ControlState::impulse_phase`] states.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WorldEventRecord {
    /// Tag13 is a retained bound physical activity fact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<phoenix_model::activity::ActivityFact>,
    /// `0` Destroyed, `1` Attacked, `2` HullDroppedBelow, `3` TimerElapsed,
    /// `4` Hailed, `5` FlagSet, `6` FlagCleared, `7` WorldLoaded,
    /// `8` EnteredRegion, `9` ExitedRegion, `10` WaypointReached, `11` Disabled.
    pub kind: u8,
    /// Reversible defeat entry/recovery for tag 11.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
    /// Tag 12 carries one observer-aware paid enquiry boundary.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub system: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub aim: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub intrusive: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enquiry_phase: Option<crate::world::config::EnquiryPhase>,
    /// The event's primary name: an entity uuid, a region uuid, or a flag name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub subject: String,
    /// The event's secondary name: an attacker uuid or a waypoint anchor.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub other: String,
    /// `FlagSet`/`FlagCleared`'s owning layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_layer: Option<String>,
    /// `HullDroppedBelow`'s `(previous, current)` fractions, or
    /// `TimerElapsed`'s elapsed seconds in `[0]`.
    #[serde(default)]
    pub numbers: [f32; 2],
}

/// The **comms** state a mid-conversation save resumes from (issue #984, S8).
///
/// [`ScenarioState`] is what the scenario has done and is still owed. This is
/// the conversation it is in the middle of *having*: the inbox the Comms
/// officer is looking at, the dialogue entries that make those messages
/// answerable, and the scripted `open_comms` requests that have been queued but
/// not yet materialised into threads.
///
/// Nothing comms-shaped was in this payload before that issue - not the inbox,
/// not the dialogues - so a save taken mid-thread came back to a world with an
/// empty Comms console and a scenario waiting for an answer that could no longer
/// be given. That is the whole gap.
///
/// # Every dialogue reduces losslessly, and the reason is structural
///
/// [`ActiveDialogue::current_node`] is a [`CommsDialogueNode`], and while
/// `[[comms]]` existed a *declarative* one's responses could carry
/// `Vec<TriggerAction>` and a nested `follow_up` tree. Serialising those is the
/// commitment [`ScenarioState`] refuses for `pending_delayed_actions`, so such a
/// node was left out and the loss reported. Issue #985 deleted the front-end
/// that produced them: a node is now built only by `project_node`, whose
/// responses are `(text, important)` and nothing else, so [`DialogueState`] is a
/// faithful copy of every node there can be and the refusal path is gone with
/// the shape that needed it.
///
/// # Honestly not covered
///
/// `contacts`, `range_flags` and `range_active` — and the per-fleet-slot
/// `fleet_range_flags` / `fleet_range_active` beside them (issue #1343) — are
/// **derived**, not
/// progression: the hail roster is rebuilt every tick from the live entities
/// that carry `[comms] hailable = true` (`update_comms_range_flags`), which also
/// recomputes both range maps from ship and entity transforms. A resumed world
/// derives all five from state this payload *does* restore. `needs_broadcast`
/// is set true by the restore rather than carried, because after a restore it is
/// unconditionally true.
///
/// `OnScreenMessage` - which message the Comms officer put on the viewscreen -
/// is a presentation choice, not scenario progression, and falls under this
/// module's standing exclusion of client projections. Nothing folds it and it is
/// re-established by the next `ShowOnScreen`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CommsState {
    /// `CommsInboxRes`, **in inbox order** - which is injection order, the order
    /// `CommsInbox::messages()` projects onto the wire and the order
    /// `operate_comms_response_ai` decides in. Not sorted, for
    /// [`ScenarioState::script_callbacks`]' reason: it is a `Vec` whose order is
    /// already a deterministic function of the run, and it is read in that
    /// order by an actor that emits commands from it.
    ///
    /// [`crate::core::messages::CommsMessage`] is the wire type and was already
    /// `Serialize`/`Deserialize` - this stores it verbatim rather than
    /// projecting it, because every field on it (`is_read`, `selected_response`,
    /// `is_urgent`, `thread_id`, `sender_uuid`) is authoritative state the
    /// response handler reads back out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inbox: Vec<CommsMessage>,
    /// Every `CommsRuntime::active_dialogues` entry, sorted by message id - the
    /// standing rule for anything that is a `HashMap` in the runtime, and the
    /// message id is the map's own key.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dialogues: Vec<DialogueState>,
    /// `CommsRuntime::open_hails`, ordered by source ship and then target.
    /// Retains which hull issued each not-yet-cleared hail; without it a resumed
    /// Backfill comms officer re-hails a contact it had already hailed, which
    /// seats a duplicate thread in the restored inbox.
    #[serde(default)]
    pub open_hails: phoenix_model::comms_history::HailHistory,
    /// `WorldScriptRuntime::pending_comms_opens`, **in queue order**.
    ///
    /// Not sorted, and the justification is [`ScenarioState::script_callbacks`]'
    /// verbatim, one step stronger: `open_scripted_comms_threads` drains this
    /// `Vec` front-to-back and mints a message id per request from the
    /// tick-scoped [`WorldIdMint`], so a reordered queue does not merely apply
    /// effects in a different order - it hands different threads different ids,
    /// which `world_digest` folds through the mint's per-namespace counters. The
    /// order is already deterministic (it is the order the scripts pushed), so
    /// there is nothing to normalise away.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_opens: Vec<OpenCommsRequest>,
    /// `CommsRuntime::pending_ai_responses` as `(key, record)`, already ordered
    /// (a `BTreeMap`, ship-UUID-first) — every unmanned Comms console's running waits
    /// on the weighted decisions it has been left holding (issue #1343).
    ///
    /// The key carries the actual ship UUID as well as the message id, because
    /// there is one inbox but a Comms console per actor: a save that flattened the two
    /// hulls' waits together would resume a mission whose second hull had
    /// silently adopted the first one's schedule.
    ///
    /// It travels for the reason `open_hails` does: it is a record of where the
    /// run got to, not something a resumed world can re-derive. Dropping it
    /// would restart every wait from zero on a resume, so a save taken four
    /// seconds into a five-second pause would come back with nine seconds still
    /// to run — a save silently changing the mission's timing, which is the same
    /// failure re-hailing a cleared contact was.
    ///
    /// The due ticks are absolute and the tick counter is itself restored, so
    /// they mean the same thing on the way back in.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_ai_responses: Vec<(
        crate::comms::server::PendingAiResponseKey,
        crate::comms::server::PendingAiResponse,
    )>,
}

/// One live dialogue - see [`CommsState`] for why every one of them travels.
///
/// The node is stored as body and per-response `(text, important)` rather than
/// as a [`CommsDialogueNode`]. Since issue #985 that IS the whole of a node, so
/// this is a faithful copy and not a lossy shortcut, and nothing here pins a
/// runtime type's shape as stored surface.
// No `Default`: a dialogue row is always built whole from a live
// `ActiveDialogue`, and there is no meaningful empty one - a `message_id` of
// `""` addresses nothing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DialogueState {
    /// The `CommsMessage::id` this dialogue answers - `active_dialogues`' key,
    /// and what a `RespondToMessage` addresses.
    pub message_id: String,
    /// The thread every message in this conversation shares.
    pub thread_id: String,
    /// The shown node's body text, as a `strings.csv` id.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body: String,
    /// The runtime values interpolated into `body`'s `{placeholder}` tokens.
    ///
    /// Captured rather than re-derived, because there is nothing to re-derive
    /// from: the figures were computed by the script at the moment the node was
    /// entered, off state that has since moved on. A resumed save that dropped
    /// them would re-render the node with its placeholders bare.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub body_params: std::collections::BTreeMap<String, String>,
    /// `(text, important)` per shown response, in the order the player sees them
    /// - the index a `RespondToMessage` submits.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub responses: Vec<(String, bool)>,
    /// The Backfill choice metadata of each shown response (issue #1343),
    /// PARALLEL to `responses` and empty when the node authors none — which is
    /// every node but Falling Skyway's lift band.
    ///
    /// A parallel vector rather than a third slot in the tuple above, so a save
    /// written before this existed reads back through the same code unchanged: a
    /// short or absent vector reads as "no metadata", which is exactly what such
    /// a save meant.
    ///
    /// It travels because a restored node is NOT rebuilt from the script — it is
    /// restored from this payload. A resume that dropped the weights would hand
    /// the conversation back to the legacy first-response policy, so an unmanned
    /// console would answer Falling Skyway's lift decisions with the stand-by
    /// for ever and the act would never resolve: a behaviour change across a
    /// save, which is the one thing this module exists to prevent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub response_ai: Vec<crate::comms::content::CommsResponseAi>,
    /// The Rhai tree that answers this dialogue.
    ///
    /// It was `Option` while declarative threads existed; issue #985 made it
    /// required, because [`ActiveDialogue::script`] is no longer optional either.
    ///
    /// [`ScriptedDialogue`] is strings only (`script_path`, `node_fn`, the
    /// parallel `on_pick` names) and already derives serde for this. Those names
    /// resolve against the **recompiled** script set, and what makes that safe is
    /// issue #864's content binding rather than anything here:
    /// the load returns `CompiledScripts::content_hash` as a ledger record and
    /// its caller applies it (issue #1241), `content_digest` folds it, and
    /// `Versions::check` refuses
    /// a save whose scripts moved - so a restored `node_fn`/`on_pick` pair is
    /// always read against the identical compiled units it was captured from.
    /// The runtime backstop is still there underneath
    /// (`EnterError::Unresolved` refuses the pick visibly rather than acting on
    /// a name that no longer exists), and so is the load-time `on_pick` lint.
    pub script: ScriptedDialogue,
}

/// One captured post-mission report row (issue #1344).
///
/// A stored mirror of [`crate::core::report::ReportRow`] rather than the type
/// itself, for the same reason `game_over` stores its outcome as a LABEL: the
/// live type carries a `ReportRowState` enum, and deriving `Serialize` on it
/// would make that enum's variant order pinned save-file surface for no gain.
/// `state` is [`crate::core::report::ReportRowState::as_str`], which is already
/// this project's report vocabulary, and an unrecognised label on restore drops
/// the row rather than guessing at a fate.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SnapshotReportRow {
    pub id: String,
    pub heading: String,
    pub outcome: String,
    pub state: String,
    pub score: i32,
}

/// One authored `GameStart` row's stable identity.
///
/// The row index is stored alongside the UUID rather than relying on the UUID
/// vector's position alone. A `when` predicate may skip a row on either boot;
/// keying by authored index makes that drift fail honestly as a missing/extra
/// entity instead of shifting every later UUID onto the wrong entity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameStartEntityUuid {
    pub authored_index: u32,
    pub entity_uuid: String,
    /// Immutable actual launch placement, including after this actor is gone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authored_slot_id: Option<String>,
}

/// The choices that determine which authored player-ship world is built before
/// [`restore`] may run.
///
/// This reuses [`crate::lockstep::FleetRoster`] itself rather than introducing a
/// second fleet schema. `selected_ship` is the canonical template path held by
/// [`crate::lobby::SelectedShipResource`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootIdentity {
    /// The hull this peer itself booted, or empty on a peer that flies none.
    ///
    /// A stationless GM peer (issue #1445) is a full deterministic participant
    /// with no lobby ship selection at all: `?gm=1` deliberately inserts no
    /// [`crate::lobby::SelectedShipResource`]. Its identity is therefore the
    /// roster — which is replicated and peer-identical — and empty here means
    /// exactly "this peer flies nothing", not "the hull was lost". Every peer
    /// that DOES fly a roster ship still records its hull, and
    /// [`required_boot_identity`] refuses an empty value from one of those.
    #[serde(default)]
    pub selected_ship: String,
    pub fleet: crate::lockstep::FleetRoster,
    /// Every authored `GameStart` entity that actually spawned, in authored row
    /// order. This includes the fleet's player ships and any GameStart NPCs.
    ///
    /// Defaulted only so a damaged format-15 development artifact still parses
    /// far enough for [`required_boot_identity`] to give the semantic refusal.
    /// A current-format run with a missing, short, invalid, out-of-order, or
    /// duplicate list is never admitted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub game_start_entity_uuids: Vec<GameStartEntityUuid>,
}

/// Captured authoritative world state: everything issue #894's record says a
/// divergence is defined over, at one tick.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PhoenixSnapshot {
    /// Required single global owner, including retained terminal identities.
    pub personnel: phoenix_sim_gameplay::personnel::PersonnelRoster,
    /// The logical tick the capture was taken between. Mirrors
    /// `vellum_save::Snapshot::tick`, which is the envelope's copy; this is the
    /// resource's own value, and [`restore`] writes it back.
    pub tick: u64,
    /// The pre-world choices this payload requires. Present in every real host;
    /// optional only so deliberately partial bare-`App` fixtures remain useful.
    /// Current-format save-slot admission treats absence as malformed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boot_identity: Option<BootIdentity>,
    /// `Time<Fixed>`'s exact accumulator toward the next logical tick.
    ///
    /// `SimTick` says which tick completed; this says whether the next rendered
    /// update is already part-way toward another one. It is stored as integer
    /// nanoseconds because Bevy's accumulator is a `Duration`, and projecting it
    /// through seconds can move the next fixed-step boundary by one update.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_overstep_nanos: Option<u64>,
    /// The authoritative product pause value at this boundary. This is stored
    /// separately from the journal because raw trusted-host pause remains a
    /// valid solo-host writer, while a coordinated fleet derives the same value
    /// from [`PhoenixSnapshot::gm_actions`].
    pub paused: bool,
    /// The complete bounded GM action/idempotency journal, including actions
    /// already received for a future logical boundary and its exact applied
    /// prefix. The prefix cannot be inferred from `tick`: immediately after
    /// FixedLast advances it, a grant at that new value still awaits PreUpdate.
    pub gm_actions: crate::gm::actions::action::GmActionJournal,
    /// Station-scoped takeover membership at the captured boundary. This is
    /// stored alongside the journal so a future grant and an already-applied
    /// grant remain distinguishable across FixedLast/PreUpdate boundaries.
    #[serde(default)]
    pub gm_puppets: crate::gm::operations::puppet::StationPuppets,
    /// Source-stripped Station commands already accepted by the canonical GM
    /// reducer but not yet copied into this tick's `AdmittedCommands` buffer.
    #[serde(default)]
    pub gm_station_commands: crate::gm::operations::puppet::PendingGmStationCommands,
    /// Direct damage/heal effects already resolved by the canonical GM reducer
    /// but not yet landed by this tick's damage phase (issue #1310). Same
    /// cross-schedule reason as `gm_station_commands`, with the durable result
    /// already claiming an exact hull amount and a lethal flag.
    #[serde(default)]
    pub gm_direct_effects: crate::gm::operations::effect::PendingGmDirectEffects,
    /// Faction hostilities a GM moved in this run, with the value each pair
    /// held before any GM touched it (issue #1442).
    ///
    /// The registry itself is authored content, rebuilt from
    /// `assets/factions/*.toml` on load and covered by `content_digest`; what
    /// cannot be rebuilt is the GM's own decisions over it, which is exactly
    /// what this carries. `before` travels with `current` so a restore of an
    /// OLDER save can revert a pair the live run has since moved, rather than
    /// leaving a later hostility standing under an earlier journal.
    ///
    /// The variant-order hazard does not apply: every field is a `String` or a
    /// `bool`.
    #[serde(default)]
    pub gm_faction_overrides: crate::gm::operations::faction::GmFactionOverrides,
    /// How long each GM placement has stood inside a player ship's sensor
    /// range, and whether its undo latch has closed (issue #1443).
    ///
    /// Authoritative history, not a repaint. The latch is a PERMANENT refusal
    /// and the count that produced it is cumulative over the whole run, so
    /// neither can be recomputed from a restored world: a resume that dropped
    /// them would hand a GM back an undo the crews had already had two seconds
    /// to see, and would make eligibility a function of when somebody saved.
    ///
    /// The variant-order hazard does not apply: every field is a `String`, a
    /// `u64` or a `bool`.
    #[serde(default)]
    pub gm_spawn_exposure: crate::gm::operations::exposure::GmSpawnExposure,
    /// How long each NPC ship has had nothing to do (issue #1435).
    ///
    /// The one piece of the GM attention queue that travels. The occurrences
    /// themselves are recomputed from live facts every publish and their reading
    /// state (filters, snoozes, holds) is private to a browser, so neither is
    /// here; but elapsed idleness is HISTORY, and no restored world can be asked
    /// how long a hull has been standing about. Dropping it would forgive a ship
    /// that was twenty-nine simulation seconds into its grace when the save was
    /// taken and make the advisory a function of when somebody saved.
    ///
    /// Restored only into a peer that keeps the watch (one presenting a GM
    /// desk); a save taken by a peer with no attention queue simply carries an
    /// empty one, which is the truth about what it was watching.
    #[serde(default)]
    pub gm_idle_npcs: crate::gm::observation::attention::GmIdleNpcWatch,
    /// How long each Station has been carrying at or above its overload count
    /// (issue #1438), in fixed simulation steps.
    ///
    /// In for `gm_idle_npcs`' reason and no other: sustained overload is
    /// ELAPSED HISTORY, and a restored world cannot be asked how long a seat
    /// has been underwater. A save taken twenty-nine seconds into a Station's
    /// overload would otherwise resume forgiving it, and the advisory would
    /// become a function of when somebody saved rather than of what the crew
    /// were being asked to do.
    ///
    /// Did not bump [`SNAPSHOT_FORMAT`]: the field is `#[serde(default)]`
    /// presentation-class advisory state, so an older save restores an empty
    /// stopwatch — a Station that has to earn its Overloaded label again over
    /// the authored duration, which is a conservative and visible answer rather
    /// than a silently wrong one.
    #[serde(default)]
    pub gm_station_workload: crate::gm::observation::workload::GmWorkloadWatch,
    /// Exact Objective lifecycle/authored state, excluding presentation transitions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub objective_records: Vec<crate::objectives::ObjectiveRecord>,
    /// Named multi-ship Objective instances, including fixed completion credit.
    #[serde(default)]
    pub objective_instances: crate::objective_instances::ObjectiveInstanceManager,
    /// Launch-frozen authored ship positions, including omitted absent slots.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frozen_ship_slots: Option<crate::ship_slots::FrozenShipSlots>,
    pub rng: Option<SimRngState>,
    pub mint: Option<WorldIdMintState>,
    pub phase: Option<GamePhase>,
    /// `(reason, outcome label)`. The outcome is a label rather than the
    /// `Outcome` enum because `Outcome` is not `Serialize` — and the labels are
    /// already this run's report vocabulary, so nothing is lost and one fewer
    /// enum's variant order becomes stored surface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_over: Option<(Option<String>, Option<String>)>,
    /// The structured post-mission report's rows, in authored order (issue
    /// #1344). Order is content here, not presentation — see `MissionReport` —
    /// so it is stored as a `Vec` and restored verbatim. Empty for a run that
    /// authored no report, which is the overwhelming majority, hence
    /// `skip_serializing_if`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mission_report: Vec<SnapshotReportRow>,
    /// `(scope, objective)` pairs in sorted key order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub captain_boosts: Vec<(String, String)>,
    /// The whole `WorldResource` payload — see the module docs for why this is
    /// wider than the digest's projection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub world: Option<WorldData>,
    /// The non-asteroid entity projection registry used by reconnect/broadcast.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracked_entities: Option<TrackedEntitiesState>,
    /// The base world's flag store.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flags: Option<FlagStore>,
    /// Active layer topology in activation order, including each layer's flag
    /// store and loader ownership. Runtime activation ordinals make the order
    /// deterministic; the vector position is the stored order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layer_flags: Vec<LayerFlags>,
    /// The AI's shared world view as of the capture tick (issue #1242) —
    /// `crate::ai::server::WorldSnapshot`'s entity list.
    ///
    /// This is the state the duel divergence actually hinged on, and the reason
    /// the payload carries it rather than rebuilding it is the whole argument of
    /// this issue. `build_world_snapshot` runs under `run_if(ai_snapshot_ready)`,
    /// the SLOWEST of the three cadences, so at any given tick the live view is
    /// deliberately a little behind the world — it holds where things were when
    /// it was last built. `restore` used to call `build_world_snapshot` once to
    /// repopulate it, which produces a view that is *correct for the restore
    /// tick* and therefore NOT the view the captured run was steering from.
    ///
    /// Measured on the duel: the live cruiser's view put its target at
    /// `[19.846668, 0, -25.908308]`; the rebuilt one put it at
    /// `[20.491692, 0, -26.884445]` — the same ship, one snapshot interval
    /// further along. Its combat-orbit leg steers off that vector, so the resumed
    /// cruiser turned differently, and the digest showed it two frames later.
    ///
    /// A rebuild cannot be made byte-identical here, because the value being
    /// rebuilt is a function of a tick the payload does not otherwise carry. The
    /// only honest options are to store it or to accept the divergence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_world: Option<Vec<crate::ai::AiWorldEntity>>,
    /// The scenario's *progression*: what has already fired, what is still
    /// scheduled, and the mission clock both are measured against — see
    /// [`ScenarioState`]. `None` only for a world with no `WorldContentRuntime`
    /// at all (a bare-`App` fixture).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scenario: Option<ScenarioState>,
    /// The conversation the scenario is in the middle of having — the inbox, the
    /// dialogues that make it answerable, and the scripted thread opens still
    /// queued. See [`CommsState`]. `None` only for a world with no
    /// `CommsRuntime` at all (a bare-`App` fixture).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comms: Option<CommsState>,
    /// `AiPolicyTickClock` — the tick-derived clock every stateful AI policy
    /// measures `state_time` against (issue #882's AC4).
    ///
    /// One `f64`, and the most load-bearing one in this payload.
    /// [`PolicyState::entered_at_secs`] is a reading *of this clock*, so
    /// restoring the policies without it hands every ship a state entered three
    /// seconds into a clock that now reads a sixteenth of a second:
    /// `memory_at` clamps the resulting negative `state_time` to zero, every
    /// time-gated transition evaluates as though the state had only just been
    /// entered, and the next AI tick walks a different edge. That was the whole
    /// of the tick-2 divergence this slice first measured and attributed to
    /// cold weapon machines — it was not the weapons. It was two ships falling
    /// out of `torpedo_run` and `inbound` back into `acquire`, one tick after a
    /// restore whose digest had matched exactly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_policy_clock: Option<f64>,
    /// Per-NPC delayed frequency-match timers, projected from process-local
    /// Entity keys to stable ship uuids. Present even when the map is empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub npc_frequency_matches: Option<NpcFrequencyMatchStatesState>,
    /// The player ship's global phaser Auto/Manual selection. Present whenever
    /// the Weapons plugin owns the resource, including `Some(PhaserMode::Auto)`
    /// even though `Auto` is the wire enum's default variant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phaser_mode: Option<crate::core::messages::PhaserMode>,
    /// The exact unread suffix behind `handle_coordination_enqueue`'s
    /// world-visible cursor, in message order. This is separate from each
    /// entity's already-attached [`CoordinationQueueState`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub coordination_staging: Vec<CoordinationEnqueueState>,
    /// Sorted by uuid — a payload must not inherit ECS iteration order any more
    /// than the digest may.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entities: Vec<EntityState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub asteroids: Vec<AsteroidState>,
    /// The streamer's window over those rocks — see [`AsteroidWindowState`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asteroid_window: Option<AsteroidWindowState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub collisions: Vec<CollisionRecord>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackedEntitiesState {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reported: Vec<String>,
    pub seeded: bool,
}

/// One ship's shared desired-motion contract and hazard assessment, as of the
/// capture tick (issue #1242) — the scalar projection of
/// [`crate::ship::helm_planner::ShipMotionPlan`].
///
/// A **projection** rather than the component whole, which is `pass_surface`'s
/// exception and not this one's: `ShipMotionPlan` carries `Vec3`s and a
/// `uuid::Uuid`, so storing it whole would pin glam's serde shape and the uuid
/// crate's as save format. Six `f32`s, a string and a bool say the same thing in
/// a form this module already knows how to keep stable.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MotionPlanState {
    /// `DesiredMotion::desired_velocity_local`, ship-local.
    pub desired_velocity_local: [f32; 3],
    /// `DesiredMotion::desired_facing_local`, ship-local.
    pub desired_facing_local: [f32; 3],
    /// `HazardAssessment::hazard_forces`, ship-local.
    pub hazard_forces: [f32; 3],
    /// `HazardAssessment::urgency`.
    pub urgency: f32,
    /// `HazardAssessment::primary_hazard`, as its string form — the shape every
    /// other id in this payload takes ([`EntityState::target_lock`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_hazard: Option<String>,
    /// `HazardAssessment::moving_hazard_threat`.
    pub moving_hazard_threat: f32,
    /// `ShipMotionPlan::docking_active`.
    pub docking_active: bool,
}

/// Token-free authority frontier for an already-running mesh participant.
/// Ratings alone omit human-seeking and Station-puppet overrides, so the
/// resolved source entries travel too. Damage/GM offline latches are already
/// restored separately and remain additive to these ordinary sources.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeshCrewState {
    pub ratings: Vec<(String, String)>,
    pub sources: Vec<(String, MeshControlSource)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub occupied: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum MeshControlSource {
    Human,
    Ai,
    Offline,
    Simplified,
}

fn is_false(value: &bool) -> bool {
    !*value
}
