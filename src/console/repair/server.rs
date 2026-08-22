use bevy::prelude::*;

use crate::command_admission::ai_emit::emit_ai_command;
use crate::core::broadcast::{Audience, Cadence, SimBroadcaster};
use crate::core::messages::ModifierSlot;
use crate::core::messages::{
    QueueEntryPreview, RepairBlackboard, ServerMessage, SystemBlackboard, SystemHullStatus,
    SystemId, TeamSlot,
};
use crate::modifiers::repair_teams::RepairTeams;
use crate::modifiers::ShipModifiers;
use crate::ship::damage::DamageTier;
use crate::ship::system_registry::{repair_system_id, REPAIR_SYSTEM_ID};
use crate::ship_plugin::ShipSystemControlSources;

// ── Resources ─────────────────────────────────────────────────────────────────

/// Per-entity component wrapping the pure `RepairTeams` state machine.
///
/// Issue #830 dropped the legacy global `Resource` derive: every ship reads and
/// writes its own `ShipRepairTeams` component (player + NPC alike), so there is
/// no ship-wide singleton to fall back to.
#[derive(Component, Clone)]
pub struct ShipRepairTeams(pub RepairTeams);

/// Priority queue of pending repair requests for a ship (issue #682).
/// Sorted by severity (worst tier first, then largest deficit).
/// Deduped by station_id: a new request for an already-queued station keeps
/// the worst tier and largest deficit.
#[derive(Component, Clone, Debug, Default)]
pub struct RepairRequestQueue {
    pub entries: Vec<RepairQueueEntry>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RepairQueueEntry {
    pub station_id: String,
    pub station_label: String,
    pub tier: DamageTier,
    pub deficit: f32,
}

impl RepairRequestQueue {
    /// Queue a repair request, merging into an existing entry for the same
    /// station (worst tier and largest deficit win).
    ///
    /// A `Destroyed`-tier entry used to be dropped on the floor here, back when
    /// a repair team could not lift the Destroyed latch. Issue #1013 made the
    /// on-site sweep repair destroyed systems, so a destroyed station is a real
    /// repair job and is queued like any other — and, just as importantly, a
    /// station already queued at a lighter tier now takes the tier UPGRADE
    /// through the merge below instead of the whole call returning early and
    /// leaving a stale reading behind.
    pub fn push_or_merge(&mut self, entry: RepairQueueEntry) {
        if let Some(existing) = self
            .entries
            .iter_mut()
            .find(|e| e.station_id == entry.station_id)
        {
            if entry.tier > existing.tier {
                existing.tier = entry.tier;
            }
            if entry.deficit > existing.deficit {
                existing.deficit = entry.deficit;
            }
            if !entry.station_label.is_empty() {
                existing.station_label = entry.station_label.clone();
            }
        } else {
            self.entries.push(entry);
        }
    }

    /// Severity ordering shared by [`Self::pop_worst`] and [`Self::peek`]:
    /// worst tier, then largest deficit, then SMALLEST station id.
    ///
    /// The station-id tie-break exists for AC4 determinism (issue #785).
    /// `Iterator::max_by` returns the LAST maximum on a tie, so without it the
    /// winner depended on `entries` insertion order — which is the order repair
    /// requests happened to be delivered in. The smallest-key rule matches the
    /// selector's documented tie-break, so the queue and the authored ranking
    /// resolve ties the same way.
    fn severity_cmp(a: &RepairQueueEntry, b: &RepairQueueEntry) -> std::cmp::Ordering {
        a.tier
            .cmp(&b.tier)
            .then_with(|| {
                a.deficit
                    .partial_cmp(&b.deficit)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            // Reversed so the SMALLEST station id is the maximum on a full tie.
            .then_with(|| b.station_id.cmp(&a.station_id))
    }

    /// Pop the highest-severity entry (worst tier, then largest deficit, then
    /// smallest station id).
    pub fn pop_worst(&mut self) -> Option<RepairQueueEntry> {
        if self.entries.is_empty() {
            return None;
        }
        let idx = self
            .entries
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| Self::severity_cmp(a, b))
            .map(|(i, _)| i)
            .unwrap();
        Some(self.entries.swap_remove(idx))
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn peek(&self) -> Option<&RepairQueueEntry> {
        if self.entries.is_empty() {
            return None;
        }
        self.entries.iter().max_by(|a, b| Self::severity_cmp(a, b))
    }
}

// ── Plugin ────────────────────────────────────────────────────────────────────

pub struct RepairPlugin;

impl Plugin for RepairPlugin {
    fn build(&self, app: &mut App) {
        // The shared AI decision cadence (issue #889): `operate_repair_ai` was
        // one of four hosts #895's FixedUpdate migration left ungated — see
        // the identical note on `NavigationPlugin::build`. `register_ai_cadence`
        // is idempotent, so `RepairPlugin` used standalone still gets
        // `AiTickReady` inserted rather than panicking on a missing `Res`.
        crate::ai::cadence::register_ai_cadence(app);
        // The dispatch router registers itself in Physics, pinning its own
        // `.after(operate_repair_ai)` ordering (issue #830). See `super::dispatch`.
        super::dispatch::register_repair_dispatch(app);
        // External repair-team dispatch (issue #1161): the command handler in
        // Input, the range-maintenance and condition-work systems in Modifiers.
        // Registered here so a hull that authored `[repair.external_dispatch]`
        // can send a team to a nearby ally; a hull without it carries no
        // component and the systems are no-ops for it.
        super::external_server::register_external_repair(app);
        app.add_systems(
            FixedUpdate,
            (
                // AC4 DETERMINISM (issue #785) — pin the remaining intra-Physics
                // edge. `operate_repair_ai` (decide/emit) →
                // `handle_dispatch_repair_team` + `handle_set_repair_priority` +
                // `handle_set_repair_target_priority` (apply) →
                // `tick_repair_teams` (advance) must run in that order every
                // tick. #830 pinned only the first edge, so `tick_repair_teams`
                // stayed ambiguous against the appliers even though all three
                // mutate `ShipRepairTeams`: Bevy's parallel executor then
                // serialised them run-varyingly, and a dispatch landing BEFORE
                // the tick let a `Returning { remaining }` slot hit
                // `remaining <= 0` and flip straight to `Travelling` instead of
                // staying `Returning`. That — not HashMap order — was the root
                // cause of `all_busy_teams_ignore_further_dispatches` flaking.
                // Production was weaker than its own `npc_repair_app` fixture,
                // which already `.chain()`s the quartet; this closes the gap.
                tick_repair_teams
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(super::dispatch::handle_dispatch_repair_team)
                    .after(super::dispatch::handle_set_repair_priority)
                    .after(super::dispatch::handle_set_repair_target_priority),
                operate_repair_ai
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .run_if(crate::ai::cadence::ai_tick_ready),
                publish_repair_blackboard.in_set(crate::sim_sets::SimSet::Publish),
            ),
        )
        .add_plugins(repair_state_broadcaster());
    }
}

// ── Broadcaster ───────────────────────────────────────────────────────────────

/// Returns a [`SimBroadcaster`] pre-configured with the `RepairState` producer.
///
/// Broadcasts `RepairState` at 10 Hz to the `Repair` console holder only.
/// Registered by [`RepairPlugin`].
///
/// Reads the LocalShip's own per-entity `ShipRepairTeams` component (issue #830
/// dropped the global-Resource fallback). Stays `LocalShip`-filtered: this is
/// the player's own repair wire, and NPC team state never reaches a client.
pub fn repair_state_broadcaster() -> SimBroadcaster {
    SimBroadcaster::new().register(
        Audience::HoldingSystem(SystemId("repair".into())),
        Cadence::Hz(10.0),
        |world: &mut World| {
            let mut q =
                world.query_filtered::<&ShipRepairTeams, With<crate::server_app::LocalShip>>();
            let Some(slots) = q.iter(world).next().map(|t| t.0.slots().to_vec()) else {
                return vec![];
            };
            vec![ServerMessage::RepairState { teams: slots }]
        },
    )
}

// ── Systems ───────────────────────────────────────────────────────────────────

/// Tick repair teams each frame: advance timers and apply HP restoration.
///
/// Iterates every ship (`With<Ship>`) — player and NPC — so ships with a
/// per-entity `ShipRepairTeams` component (spawned when their TOML declares
/// a `[repair]` block) tick their own teams against their own
/// `EntitySystemHull`. Each ship applies its own `ShipModifiers.RepairRate`
/// multiplier.
///
/// The ship's own `ShipConfigComponent` rides along because the sweep (issue
/// #1013) needs to know which systems share a station. It is `Option` for the
/// same reason the teams component is: a ship spawned without one still ticks,
/// its teams simply falling back to the pre-sweep "fix one system, walk home"
/// behaviour — see `RepairTeams::tick`.
pub fn tick_repair_teams(
    time: Res<Time>,
    mut ship_q: Query<
        (
            Option<&mut ShipRepairTeams>,
            &ShipModifiers,
            &mut crate::entities::spawner::EntitySystemHull,
            Option<&crate::entities::spawner::EntityUuid>,
            Option<&crate::ship_plugin::ShipConfigComponent>,
        ),
        With<crate::server_app::Ship>,
    >,
    // Balance telemetry. `Option<ResMut<Messages<_>>>` so bare-`App` fixtures
    // that never registered the message still pass parameter validation.
    mut balance_events: Option<
        ResMut<bevy::ecs::message::Messages<crate::core::balance::BalanceEvent>>,
    >,
) {
    let dt = time.delta_secs();

    for (teams_comp, modifiers, mut hull, ship_uuid, config) in ship_q.iter_mut() {
        let Some(mut teams) = teams_comp else {
            continue;
        };
        let repair_mult = modifiers.get(&ModifierSlot::RepairRate);
        // Capture total hull current before/after the tick so the restored HP
        // can be reported as a per-ship `RepairApplied` delta (issue #841).
        // This is the only path that actually ticks a ship's teams — the global
        // `ShipRepairTeams` resource is publish-only, never ticked.
        let before = hull.0.total_current();
        teams
            .0
            .tick(dt * repair_mult, &mut hull.0, config.map(|c| &c.0));
        let restored = hull.0.total_current() - before;
        if restored > 0.0 {
            if let (Some(msgs), Some(uuid)) = (balance_events.as_mut(), ship_uuid) {
                msgs.write(crate::core::balance::BalanceEvent::RepairApplied {
                    ship: uuid.0.clone(),
                    hp: restored,
                });
            }
        }
    }
}

// ── Blackboard publish ─────────────────────────────────────────────────────────

/// Per-`Ship` publisher (issue #830). Each ship builds its own repair blackboard
/// from its own `ShipRepairTeams` / `EntitySystemHull` / `RepairRequestQueue`
/// components and writes it into its own `ShipSystemBlackboards`. Ships without a
/// `[repair]` block carry no `ShipRepairTeams`; the missing-default idiom gives
/// them an empty team set. Only ships with `[behaviour]` carry
/// `ShipSystemBlackboards`, so the query naturally scopes to AI-bearing ships;
/// the wire broadcaster stays `LocalShip`-filtered.
fn publish_repair_blackboard(
    mut ship_q: Query<
        (
            Option<&ShipRepairTeams>,
            Option<&crate::entities::spawner::EntitySystemHull>,
            Option<&RepairRequestQueue>,
            // External repair-team dispatch (issue #1161). Present only on a
            // hull that authored `[repair.external_dispatch]`; a hull without it
            // leaves every external-dispatch field `None` and is byte-identical
            // on the wire to one built before this existed.
            Option<&super::external_server::ExternalRepairDispatch>,
            &mut crate::server_app::ShipSystemBlackboards,
        ),
        With<crate::server_app::Ship>,
    >,
    // Resolve a dispatched target's world entity-name id for the readout — no
    // English crosses the wire, exactly as the tractor publisher reports its
    // coupled target's name.
    named: Query<(
        &crate::entities::spawner::EntityUuid,
        &crate::entities::spawner::EntityName,
    )>,
) {
    for (teams_opt, hull_opt, repair_queue_ref, external_opt, mut blackboards) in ship_q.iter_mut()
    {
        let default_teams;
        let teams: &ShipRepairTeams = match teams_opt {
            Some(t) => t,
            None => {
                default_teams =
                    ShipRepairTeams(crate::modifiers::repair_teams::RepairTeams::default());
                &default_teams
            }
        };
        let team_slots: Vec<TeamSlot> = teams.0.slots().to_vec();

        // Build the SystemHullStatus list from the authoritative `SystemHull`
        // iteration.
        let system_hull: Vec<SystemHullStatus> = hull_opt
            .map(|h| {
                h.0.iter()
                    .map(|(sid, entry)| SystemHullStatus {
                        system_id: sid.clone(),
                        display_name: entry.display_name.clone(),
                        current: entry.current,
                        max_hp: entry.max,
                        tier: h.0.tier_for(sid),
                        debuff_magnitude: h.0.debuff_magnitude_for(sid),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let damageable_systems: Vec<SystemId> =
            system_hull.iter().map(|s| s.system_id.clone()).collect();

        let queue_depth: Vec<QueueEntryPreview> = repair_queue_ref
            .map(|rq| {
                let mut entries = rq.entries.clone();
                entries.sort_by(|a, b| {
                    b.tier.cmp(&a.tier).then_with(|| {
                        b.deficit
                            .partial_cmp(&a.deficit)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                });
                entries
                    .iter()
                    .map(|e| QueueEntryPreview {
                        station_id: e.station_id.clone(),
                        station_label: e.station_label.clone(),
                        tier: e.tier,
                        deficit: e.deficit,
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Emit the new SystemId-keyed hull + damageable list only (legacy
        // `console_hull` / `damageable_consoles` wire fields were dropped in #619).
        // External repair dispatch (issue #1161): the authored reach, the target
        // a team is working abroad and its name id, and the last refusal's
        // string id. All `None` on a hull that authored no dispatch capability.
        let (
            external_dispatch_range,
            external_dispatch_target,
            external_dispatch_target_name,
            external_dispatch_refusal,
        ) = match external_opt {
            Some(external) => {
                let target = external.dispatched_target.clone();
                let target_name = target.as_ref().and_then(|uuid| {
                    named
                        .iter()
                        .find(|(id, _)| &id.0 == uuid)
                        .map(|(_, name)| name.0.clone())
                });
                (
                    Some(external.config.range),
                    target,
                    target_name,
                    external.last_refusal.map(|r| r.string_id().to_string()),
                )
            }
            None => (None, None, None, None),
        };

        let bb = RepairBlackboard {
            teams: team_slots,
            travel_duration_secs: teams.0.timings().travel_duration,
            system_hull,
            damageable_systems,
            // Host-internal copy: unprojected. `system_hull` and `queue_depth` both
            // carry exact per-system detail and are filtered on the wire by
            // `visibility::project_repair_blackboard`, which also fills in the
            // aggregate (issue #737) and the destroyed-capability share (issue
            // #1014). The repair AI controller reads this copy and needs every
            // system.
            queue_depth,
            aggregate_hull_fraction: None,
            destroyed_hull_fraction: None,
            external_dispatch_range,
            external_dispatch_target,
            external_dispatch_target_name,
            external_dispatch_refusal,
        };

        blackboards.0.insert(
            SystemId(REPAIR_SYSTEM_ID.to_string()),
            SystemBlackboard::Repair(bb),
        );
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

// ── AI controller stub ─────────────────────────────────────────────────────────

pub fn all_systems_in_station_are_operational(
    station_id: &str,
    hull: &crate::ship::damage::SystemHull,
    config: &crate::ship::config::ShipConfig,
) -> bool {
    let systems_in_station: Vec<_> = config
        .systems
        .iter()
        .filter(|s| s.station.as_ref().map(|st| st.0.as_str()) == Some(station_id))
        .collect();
    !systems_in_station.is_empty()
        && systems_in_station
            .iter()
            .all(|s| hull.tier_for(&s.id) == DamageTier::Operational)
}

// `best_damaged_system_in_station` was removed in #830: `operate_repair_ai` now
// emits a station-granular `DispatchRepairTeam` and the host repair router's
// `resolve_repair_target` picks the fine system in the station, so the AI no
// longer resolves the fine system inline. See `operate_repair_ai` for why this
// is a deliberate change of the healed-first heuristic, not an equivalence.

// ── Authored repair-target ranking (issue #785) ────────────────────────────────

/// The candidate key standing for [`crate::core::messages::RepairTarget::Core`] — the
/// ownerless ship-wide repair bucket. Selector candidate identity is a plain
/// `String` (nothing requires a real UUID), so Repair keys candidates on the
/// STATION ID and the winning key IS the dispatch target. This is the one key
/// that maps to `RepairTarget::Core` instead of `RepairTarget::Station(..)`; it
/// is also the `SystemId` the router resolves `Core` to, so the two agree.
pub const REPAIR_CORE_BUCKET_KEY: &str = "core";

/// Per-ship resolved Repair target selector (issue #785).
///
/// Holds the ship's data-driven [`crate::ai::selector::TargetSelector`], decoded
/// from the authored `[repair.selector]` block, plus the authored ship
/// `power_rating`, which [`operate_repair_ai`] exposes to the selector's
/// expressions as `self_fact(power_rating)`. Attached at spawn beside the
/// Sensors/Tactical/Navigation selectors.
///
/// Since #885b stage 5d there is no Rust-side synthesised default behind it: a
/// ship without the component ranks nothing and [`operate_repair_ai`] skips it.
/// Mirrors [`crate::console::navigation::NavigationTargetSelector`].
#[derive(Component, Clone, Debug)]
pub struct RepairTargetSelector {
    /// The resolved ranking policy.
    pub selector: crate::ai::selector::TargetSelector,
    /// Authored ship power rating, seeded from `EntityConfig.power_rating`.
    pub power_rating: Option<f32>,
}

/// One repair candidate's observable readings, resolved host-side before the
/// pure fact seed (AC1 — every field is authoritative observable damage or team
/// availability; nothing here is private AI memory).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RepairCandidateReading {
    /// [`DamageTier`] discriminant: Operational 0, Damaged 1, Disabled 2,
    /// Destroyed 3. A structural enum ordinal, not a tunable value.
    pub tier_ordinal: u8,
    /// Absolute HP missing, as reported by the repair request (or, for the core
    /// bucket, read off the hull).
    pub deficit: f32,
    /// Aggregate `1 - current/max` across every hull system the station owns.
    /// For the ownerless `core` bucket — which owns no station and so has no
    /// station-scan result — this is the hull's own `core` entry damage.
    pub damage_fraction: f32,
    /// The most damaged single system in the station — the one the router's
    /// `resolve_repair_target` will actually send the team to. For the `core`
    /// bucket this equals `damage_fraction` (it is a single hull entry).
    pub worst_system_damage_fraction: f32,
    /// How many hull systems the station owns (1 for the `core` bucket).
    pub system_count: usize,
    /// Whether this candidate is the ship-wide `core` bucket.
    pub is_core: bool,
    /// Whether a coordination-delivered `RepairRequest` currently names this
    /// station. The canonical eligibility keys on this, so the AI ranks only
    /// damage that was actually reported (issue #830 removed the raw hull poll).
    pub source_repair_request: bool,
    /// Whether a team is already Travelling to / Repairing this station, or an
    /// earlier team in this same tick was just dispatched to it. The canonical
    /// eligibility excludes these, which is what makes N free teams pick N
    /// DISTINCT stations (AC2/AC4).
    pub assigned: bool,
}

/// Seed one repair candidate's CANDIDATE-context facts (issue #785).
///
/// Pure and Bevy-free (AGENTS.md rule #10): the host resolves the live station
/// damage before calling this, so the authored eligibility/score expressions
/// evaluate over real readings. The #779 empty-facts lesson applies — every fact
/// an authored guard can reference is seeded here, so a `candidate_fact(...)`
/// guard actually fires instead of silently reading "absent → false".
pub fn seed_repair_facts(reading: &RepairCandidateReading) -> crate::world::flags::AiFacts {
    use crate::entities::ai_flag_hosts as fid;
    let mut facts = crate::world::flags::AiFacts::new();
    facts.set_fact(fid::TIER_ORDINAL, reading.tier_ordinal as f64);
    facts.set_fact(fid::DEFICIT, reading.deficit as f64);
    facts.set_fact(fid::DAMAGE_FRACTION, reading.damage_fraction as f64);
    facts.set_fact(
        fid::WORST_SYSTEM_DAMAGE_FRACTION,
        reading.worst_system_damage_fraction as f64,
    );
    facts.set_fact(fid::SYSTEM_COUNT, reading.system_count as f64);
    facts.set_fact(fid::IS_CORE, if reading.is_core { 1.0 } else { 0.0 });
    facts.set_fact(
        fid::SOURCE_REPAIR_REQUEST,
        if reading.source_repair_request {
            1.0
        } else {
            0.0
        },
    );
    facts.set_fact(
        fid::SOURCE_CORE_BUCKET,
        if reading.is_core { 1.0 } else { 0.0 },
    );
    facts.set_fact(fid::ASSIGNED, if reading.assigned { 1.0 } else { 0.0 });
    facts
}

/// Seed the operating ship's SELF-context facts for the repair selection
/// (issue #785). Pure and Bevy-free, same contract as [`seed_repair_facts`].
///
/// The seeded facts, each an authored guard's vocabulary:
///   - `free_team_count` — how many repair team slots are `Idle` this tick, i.e.
///     how many selections `operate_repair_ai` will run.
///   - `total_hull_health_fraction` — ship-wide `total_current / total_max`, so
///     `1.0` is a pristine hull and `0.0` a flattened one. Named for HEALTH
///     deliberately: the candidate-side `damage_fraction` is its INVERSE
///     (`1 - current/max`, `0.0` pristine), and two similarly-named facts with
///     opposite senses is exactly the authoring trap this name avoids.
///   - `power_rating` — the authored ship power rating, absent (not zero) when
///     the ship declares none, so `self_fact(power_rating)` guards do not fire
///     on an unrated ship.
///   - `red_alert` — 1.0 while the ship is at red alert, else 0.0.
pub fn seed_repair_self_facts(
    free_team_count: usize,
    total_hull_health_fraction: f32,
    power_rating: Option<f32>,
    red_alert: bool,
) -> crate::world::flags::AiFacts {
    use crate::entities::ai_flag_hosts as fid;
    let mut facts = crate::world::flags::AiFacts::new();
    facts.set_fact(fid::FREE_TEAM_COUNT, free_team_count as f64);
    facts.set_fact(
        fid::TOTAL_HULL_HEALTH_FRACTION,
        total_hull_health_fraction as f64,
    );
    facts.set_fact(fid::RED_ALERT, if red_alert { 1.0 } else { 0.0 });
    if let Some(pr) = power_rating {
        facts.set_fact(fid::POWER_RATING, pr as f64);
    }
    facts
}

/// The station a team slot is currently committed to, or `None` when the slot
/// holds no commitment (`Idle`, or a target whose station cannot be resolved).
///
/// This is the ONLY carrier of Repair's retained selector pick: it is derived
/// from the authoritative [`TeamSlot`] every tick and is never cached in an
/// AI-owned component (the AC5 reset invariant — see [`operate_repair_ai`]).
/// `Returning` deliberately does not count: the team has left, so its station is
/// free to be re-picked, exactly as before #785.
///
/// Issue #1013 lengthens how long a commitment lasts without changing this rule:
/// a sweeping team stays `Repairing` while it works through its station's
/// systems, so the station stays excluded for the whole visit rather than
/// freeing up between systems. That is the same answer the pre-sweep code gave
/// while a team was en route or working, just held for longer — and it is the
/// right one, since a second team sent to a station the first is already
/// sweeping would duplicate the trip.
///
/// # Why the hull is needed to answer this
///
/// A sweeping team MOVES between systems inside its group, and in the ownerless
/// group it can move OFF the literal `core` row: a hull that carries two
/// ownerless rows (the shipped `alliance_cruiser` carries `core` and `science`)
/// buckets both under [`REPAIR_CORE_BUCKET_KEY`], so a team dispatched to
/// `RepairTarget::Core` may legitimately be found `Repairing` `science`. The
/// config cannot name that group — an ownerless row is by definition one
/// `ShipConfig` does not describe, so `config.system(..)` is `None` — which is
/// exactly why the hull decides it: a HULL-TRACKED id the config does not
/// describe is the core bucket, and only an id the hull does not track at all
/// (a station NAME, the `resolve_repair_target` fallback shape) is no
/// commitment. Answering `None` for a swept-to ownerless row would drop the
/// core bucket out of `excluded` mid-visit and let a second team be dispatched
/// to the group the first is already sweeping — the #785 AC4 "N free teams pick
/// N DISTINCT stations" guarantee, broken by the sweep.
fn committed_station_for_slot(
    slot: &TeamSlot,
    config: &crate::ship_plugin::ShipConfigComponent,
    hull: &crate::ship::damage::SystemHull,
) -> Option<String> {
    let system_id = match slot {
        TeamSlot::Travelling { system_id, .. } | TeamSlot::Repairing { system_id, .. } => {
            system_id.as_ref()?
        }
        _ => return None,
    };
    if system_id.0 == REPAIR_CORE_BUCKET_KEY {
        return Some(REPAIR_CORE_BUCKET_KEY.to_string());
    }
    if let Some(station) = config
        .0
        .system(system_id)
        .and_then(|sc| sc.station.as_ref())
    {
        return Some(station.0.clone());
    }
    // Ownerless: the config gives this id no station (either it describes no
    // such system, or it describes one with no `station`), which is the same
    // test `repair_teams::sweep_group` and `damage_sync` use to bucket a row.
    // Committed to the core bucket iff the hull actually tracks the row; an
    // untracked id is a station name, which commits to nothing.
    hull.get(system_id)
        .map(|_| REPAIR_CORE_BUCKET_KEY.to_string())
}

/// Aggregate a station's observable hull damage: `(damage_fraction,
/// worst_system_damage_fraction, system_count)`.
fn station_damage_readings(
    station_id: &str,
    hull: &crate::ship::damage::SystemHull,
    config: &crate::ship_plugin::ShipConfigComponent,
) -> (f32, f32, usize) {
    let mut total_max = 0.0_f32;
    let mut total_current = 0.0_f32;
    let mut worst = 0.0_f32;
    let mut count = 0_usize;
    for system in config
        .0
        .systems
        .iter()
        .filter(|s| s.station.as_ref().map(|st| st.0.as_str()) == Some(station_id))
    {
        let Some(entry) = hull.get(&system.id) else {
            continue;
        };
        count += 1;
        total_max += entry.max;
        total_current += entry.current;
        if entry.max > 0.0 {
            worst = worst.max(1.0 - entry.current / entry.max);
        }
    }
    let fraction = if total_max > 0.0 {
        1.0 - total_current / total_max
    } else {
        0.0
    };
    (fraction, worst, count)
}

/// Per-kind AI loop for repair. Iterates every ship (`With<Ship>`) whose
/// Repair system is `ControlSource::Ai` and dispatches its idle teams to the
/// stations the AUTHORED [`RepairTargetSelector`] ranks highest. Ships with no
/// per-entity `ShipRepairTeams` component silently skip — an NPC without a
/// `[repair]` block simply has no teams to dispatch.
///
/// # Authored ranking (issue #785)
///
/// The hardcoded `(tier desc, deficit desc)` comparator is RETIRED outright
/// (the #784 Power shape, not the #783 Shields retained-kernel shape). Repair is
/// the fourth host of the reusable #776 [`crate::ai::selector::TargetSelector`]
/// and the first whose candidate identity is not an entity UUID: a
/// `SelectorCandidate.uuid` is just a `String`, so Repair keys candidates on the
/// STATION ID and the winning key IS the `RepairTarget`. No side-table is
/// needed (contrast #778 Navigation's `uuid → WaypointMode` map). Ranking stays
/// at STATION granularity because the typed input only addresses stations and
/// the core bucket; which fine system inside the station heals first remains the
/// shared `resolve_repair_target`'s call (§2 symmetry, #830). Per-system damage
/// is exposed to the ranking as candidate FACTS, never as a new
/// `RepairTarget` variant the human wire could not send.
///
/// This uses the selector rather than the #775 channel/verb policy because
/// "eligibility + additive utility over a VARIABLE candidate set" is selector
/// vocabulary. Shields (#783) stayed on channel/verb precisely because its arcs
/// are a fixed 4-set of in-ship indices; Repair's damaged-station set changes
/// every tick.
///
/// # Multi-team assignment — greedy sequential, deterministic by construction
///
/// Teams are visited in ASCENDING slot index. Before each selection the
/// candidate set is rebuilt with the `assigned` fact set for every station a
/// team is already committed to plus every station an earlier team in this same
/// tick was just given, and the authored eligibility drops them — so N free
/// teams pick N DISTINCT stations. Exclusion lives in a `BTreeSet` and the
/// candidate vector is built in sorted station-id order, so no `HashMap`
/// iteration order can reach the decision; residual score ties fall through to
/// the selector's documented smallest-key tie-break. This is deliberately
/// greedy-with-exclusion (matching the retired behaviour), not an optimal
/// assignment.
///
/// # AC5 — no AI-owned state, so reset is automatic
///
/// Policies are stateless and #785 adds NO new AI state component. The two
/// carriers of retained state are both authoritative:
///   1. `RepairRequestQueue.entries`, pruned below the moment every system in
///      the request's group is Operational — so a completed repair's target
///      disappears (AC4). "Group" is the station's own systems, or every
///      ownerless hull row for the `core` bucket. Since issue #1013 a Destroyed
///      system counts as work remaining rather than as a reason to give up.
///   2. The selector's hysteresis `current`, derived PER TICK from the
///      authoritative [`TeamSlot`] via [`committed_station_for_slot`] and never
///      cached. A completed repair returns the slot to `Idle` ⇒ `current` is
///      `None` ⇒ the next selection starts from initial policy state.
///
/// Human takeover is the `operate_ai` gate below plus admission, which
/// independently rejects an `ai:` emission on a human-held Repair system.
///
/// After PRD #597 gap-5 closure: same code path for player Backfill AI and
/// NPC AI. The only differentiator is `ShipSystemControlSources`
/// (data-driven) and `LocalShip` marker.
///
/// Decide-and-emit (issue #830): the queue-based *station* decision is
/// unchanged, but instead of calling `teams.dispatch(..)` directly (the §2
/// violation) each assignment is emitted as an admitted `DispatchRepairTeam {
/// team_idx, target: Station(..) }` through the shared
/// [`crate::command_admission::validate_and_admit`] seam with this ship's own
/// `ai:<uuid>` token — the identical admitted path a human Engineering dispatch
/// takes. `handle_dispatch_repair_team` applies it later this tick (Physics,
/// `.after(operate_repair_ai)`).
///
/// # Which fine system heals first is now the shared applier's call
///
/// A station-granular admitted payload cannot carry the AI's old *private*
/// per-system choice, so the fine target is resolved by the router's
/// `resolve_repair_target` — the same code a human dispatch runs. This is the
/// point of admission symmetry (§2): both sources resolve the fine system
/// identically. It is a deliberate change, not an equivalence. The retired
/// inline `best_damaged_system_in_station` ranked candidates by **absolute HP
/// deficit** (`max - current`); `resolve_repair_target` ranks by **damage
/// fraction** (`1 - current/max`). For a station owning a single repairable
/// system the two agree, but shipped hulls have multi-system stations of
/// differing max HP (e.g. `alliance_destroyer`'s helm owns
/// `helm-engine-{port,starboard}` / `helm-radar` at max 15 and
/// `helm-lateral-thrust` at max 10), so when several of a station's systems are
/// damaged at once the healed-first system can differ from the pre-#830 choice.
/// The AI adopting the human path's fraction-ranking is the intended refinement
/// (same class as #826 dissolving the AI's bespoke resolution into the shared
/// seam), not a regression.
pub fn operate_repair_ai(
    sessions: Res<crate::lobby::Sessions>,
    // The read-only AI-host world context — flag chain, sessions, and origin
    // stamps — behind one bare-`Res` system param (issue #1207). A fixture that
    // runs this host must register it (`register_ai_host_env`) or fail loudly at
    // schedule build, so a bare `App` cannot silently diverge from production.
    ai_env: crate::ai::host::AiHostEnv,
    mut ships: Query<
        (
            Entity,
            Option<&crate::entities::spawner::EntityUuid>,
            &ShipSystemControlSources,
            Option<&ShipRepairTeams>,
            Option<&crate::entities::spawner::EntitySystemHull>,
            Option<&mut RepairRequestQueue>,
            Option<&crate::ship_plugin::ShipConfigComponent>,
            Option<&RepairTargetSelector>,
            Option<&crate::ship::state::ShipRedAlert>,
            // The external repair-dispatch record (issue #1161), read for the
            // one question it answers here: how many of this ship's teams are
            // held abroad against a designated target, and so unavailable to the
            // hull's own damage-control sweep (AGENTS.md rule 6).
            Option<&super::external_server::ExternalRepairDispatch>,
            &mut crate::core::messages::AdmittedCommands,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    for (
        ship_entity,
        entity_uuid,
        sources,
        teams_comp,
        hull_comp,
        repair_queue_comp,
        config_comp,
        target_selector,
        red_alert_comp,
        external_dispatch,
        mut admitted,
    ) in ships.iter_mut()
    {
        // Control-Source gate through the shared AI host spine (issue #1208): a
        // human holder (or an offline system) stands the auto-dispatcher down.
        // Repair resolves a data-driven SELECTOR the spine does not model, so only
        // its gate — the one step it shares with the policy hosts — routes here.
        if !crate::ai::host::ai_operates(&sources.0, repair_system_id()) {
            continue;
        }
        // The queue + config are always present together on production ships
        // (the entity spawner inserts both unconditionally). Ships lacking
        // either simply have nothing to auto-dispatch — the old queue-less
        // hull-poll fallback (a direct-write §2 violation) is removed (#830).
        let (Some(teams), Some(hull), Some(mut rq), Some(config)) =
            (teams_comp, hull_comp, repair_queue_comp, config_comp)
        else {
            continue;
        };

        // Retain a request while ANY system in its GROUP still needs a team —
        // the systems the config gives to that station, or, for the `core`
        // bucket, every ownerless hull row (see the branch below; the two are
        // one rule wearing two lookups, because a station is named by the config
        // and the ownerless bucket can only be named by the hull).
        //
        // The tier predicate was `!= Operational && != Destroyed` until issue
        // #1013: a station whose systems had all been shot to 0 HP had its
        // request evicted, because a repair team could not lift the Destroyed
        // latch and sending one would have been a pointless trip. The on-site
        // sweep now repairs destroyed systems, so evicting them is what would
        // strand them — nothing else in the game clears a Destroyed latch.
        // `!= Operational` is now the whole test, and it is the same predicate
        // `repair_teams::next_sweep_target` ranks candidates by, so what the
        // dispatcher keeps sending teams for and what an arrived team works on
        // are one rule.
        rq.entries.retain(|entry| {
            // The `core` bucket owns NO station in `ShipConfig` — validation
            // actively forbids a station with that id, and `damage_sync` files
            // EVERY ownerless system under it — so the station-owned scan below
            // would find zero systems and prune every core request. Prune it
            // against the hull instead, over the whole ownerless GROUP.
            //
            // The group, not just the `core` row: `damage_sync` addresses a
            // request for any system the config gives no station to under this
            // one id, and a hull may carry several such rows (the shipped
            // `alliance_cruiser` carries `core` and `science`). Testing only the
            // literal `core` row evicted the request whenever `core` itself was
            // Operational, so a destroyed sibling was never dispatched for and
            // stayed destroyed forever — nothing else in the game clears the
            // latch. This is deliberately the same set `repair_teams::
            // sweep_group` calls ownerless and the same `!= Operational` test
            // `next_sweep_target` ranks by, so what the dispatcher keeps sending
            // teams for and what an arrived team works on are one rule.
            if entry.station_id == REPAIR_CORE_BUCKET_KEY {
                return hull.0.iter().any(|(sid, _)| {
                    config
                        .0
                        .system(sid)
                        .and_then(|s| s.station.as_ref())
                        .is_none()
                        && hull.0.tier_for(sid) != DamageTier::Operational
                });
            }
            config
                .0
                .systems
                .iter()
                .filter(|s| {
                    s.station.as_ref().map(|st| st.0.as_str()) == Some(entry.station_id.as_str())
                })
                .any(|s| hull.0.tier_for(&s.id) != DamageTier::Operational)
        });

        // Free team indices, ASCENDING — the deterministic visit order (AC4).
        // Emission does not mutate `teams` this tick (the applier does, later in
        // Physics), so `lowest_free_team()` would return the same idx every
        // time; we draw from a locally-consumed list instead.
        //
        // Minus whatever an external repair dispatch is holding abroad (issue
        // #1161). This is the capacity-as-cost trade the PRD decided: a crew
        // sending teams to an ally with every team committed is a crew whose own
        // damaged systems go unswept, and this line is what makes that true
        // rather than a claim in a design note. The teams never leave the hull —
        // they are still `Idle` in every readout — they are just not available
        // to be sent anywhere. The human dispatch router reads the same source,
        // so a team sent to an ally cannot be undercut by whichever path did not
        // know about it (AGENTS.md rule 6).
        let committed = external_dispatch
            .map(|e| e.committed_repair_teams())
            .unwrap_or(0);
        let free_teams: Vec<usize> = teams.0.free_team_indices(committed);
        if free_teams.is_empty() {
            continue;
        }

        // Stations already committed to by a Travelling/Repairing team. BTreeSet,
        // never a HashSet: this set gates the authored eligibility, so its
        // iteration order must never reach the decision.
        let mut excluded: std::collections::BTreeSet<String> = teams
            .0
            .slots()
            .iter()
            .filter_map(|slot| committed_station_for_slot(slot, config, &hull.0))
            .collect();

        // No authored `[repair.selector]` ⇒ no component ⇒ no dispatch ranking.
        // Since #885b stage 5d there is no synthesised stand-in.
        let Some(selector_comp) = target_selector else {
            continue;
        };

        // ── Candidate readings, built in sorted station-id order (AC1) ────────
        // Source `damaged-stations`: exactly the coordination-delivered repair
        // requests. Issue #830 removed the raw hull poll and #785 does not bring
        // it back — the AI ranks only damage that was actually reported. The
        // per-station hull aggregate below is the authoritative observable
        // detail those requests are ranked BY, not an extra discovery channel.
        let mut sorted_entries: Vec<&RepairQueueEntry> = rq.entries.iter().collect();
        sorted_entries.sort_by(|a, b| a.station_id.cmp(&b.station_id));

        let mut readings: Vec<(String, RepairCandidateReading)> =
            Vec::with_capacity(sorted_entries.len() + 1);
        for entry in &sorted_entries {
            let (damage_fraction, worst, system_count) =
                station_damage_readings(&entry.station_id, &hull.0, config);
            readings.push((
                entry.station_id.clone(),
                RepairCandidateReading {
                    tier_ordinal: entry.tier as u8,
                    deficit: entry.deficit,
                    damage_fraction,
                    worst_system_damage_fraction: worst,
                    system_count,
                    is_core: entry.station_id == REPAIR_CORE_BUCKET_KEY,
                    source_repair_request: true,
                    assigned: false,
                },
            ));
        }
        // Source `core-bucket`: the ownerless ship-wide bucket, surfaced whenever
        // the ship's hull actually carries a `core` entry. It carries
        // `source_core_bucket` but no repair request of its own, so under the
        // canonical eligibility it never independently selects — the same
        // enrich-don't-steer shape as Navigation's `chart-contacts`.
        let core_id = SystemId(REPAIR_CORE_BUCKET_KEY.to_string());
        if let Some(core_entry) = hull.0.get(&core_id) {
            let core_fraction = if core_entry.max > 0.0 {
                1.0 - core_entry.current / core_entry.max
            } else {
                0.0
            };
            let core_reading = RepairCandidateReading {
                tier_ordinal: hull.0.tier_for(&core_id) as u8,
                deficit: (core_entry.max - core_entry.current).max(0.0),
                damage_fraction: core_fraction,
                worst_system_damage_fraction: core_fraction,
                system_count: 1,
                is_core: true,
                source_repair_request: false,
                assigned: false,
            };
            match readings
                .iter_mut()
                .find(|(key, _)| key == REPAIR_CORE_BUCKET_KEY)
            {
                // A repair request already named `core`: keep its REPORTED tier
                // and deficit (the coordination-delivered reading), but take the
                // damage aggregates from the hull. `station_damage_readings`
                // scans `config.systems` for `station == Some("core")` and a
                // station with that id is FORBIDDEN by `ShipConfig` validation,
                // so it necessarily returned `(0.0, 0.0, 0)` for this entry —
                // leaving it seeded would zero `damage_fraction` /
                // `worst_system_damage_fraction` / `system_count` and make every
                // authored guard over them dead for the core bucket.
                Some((_, existing)) => {
                    existing.damage_fraction = core_reading.damage_fraction;
                    existing.worst_system_damage_fraction =
                        core_reading.worst_system_damage_fraction;
                    existing.system_count = core_reading.system_count;
                    existing.is_core = true;
                }
                None => readings.push((REPAIR_CORE_BUCKET_KEY.to_string(), core_reading)),
            }
        }
        readings.sort_by(|a, b| a.0.cmp(&b.0));

        if readings.is_empty() {
            continue;
        }

        // SELF context. Candidates all sit at the operating ship, so the planar
        // horizon never gates; positions are the shared origin.
        let total_max = hull.0.total_max();
        let total_hull_health_fraction = if total_max > 0.0 {
            hull.0.total_current() / total_max
        } else {
            1.0
        };
        let self_ctx = crate::ai::selector::SelfContext {
            position: [0.0, 0.0, 0.0],
            facts: seed_repair_self_facts(
                free_teams.len(),
                total_hull_health_fraction,
                selector_comp.power_rating,
                red_alert_comp.map(|ra| ra.0).unwrap_or(false),
            ),
        };

        // The scenario flag chain, anchored at the layer that spawned this
        // ship (issue #891 stage 2).
        let flag_chain = ai_env.flag_chain(ship_entity);

        // ── Greedy sequential selection, one authored `select` per free team ──
        for team_idx in free_teams {
            let candidates: Vec<crate::ai::selector::SelectorCandidate> = readings
                .iter()
                .map(|(key, reading)| {
                    let mut reading = reading.clone();
                    reading.assigned = excluded.contains(key);
                    crate::ai::selector::SelectorCandidate {
                        uuid: key.clone(),
                        position: [0.0, 0.0, 0.0],
                        facts: seed_repair_facts(&reading),
                    }
                })
                .collect();

            // AC5: the retained pick is read back off the AUTHORITATIVE slot
            // every tick, never from a cached AI field. An Idle slot — the only
            // kind we dispatch — holds no commitment, so this is `None` and each
            // selection starts from initial policy state.
            let current = teams
                .0
                .slots()
                .get(team_idx)
                .and_then(|slot| committed_station_for_slot(slot, config, &hull.0));

            let Some(winner) = selector_comp.selector.select(
                &self_ctx,
                &candidates,
                current.as_deref(),
                &flag_chain,
            ) else {
                // Nothing eligible left this tick; later teams cannot do better
                // over the same candidate set, so stop.
                break;
            };

            let target = if winner == REPAIR_CORE_BUCKET_KEY {
                crate::core::messages::RepairTarget::Core
            } else {
                crate::core::messages::RepairTarget::Station(crate::core::messages::StationId(
                    winner.clone(),
                ))
            };
            emit_ai_command(
                entity_uuid,
                crate::ship::system_registry::repair_system_id(),
                crate::core::messages::SystemControlPayload::DispatchRepairTeam {
                    team_idx: team_idx as u8,
                    target,
                },
                sources,
                &sessions,
                Some(config),
                &mut admitted,
            );
            excluded.insert(winner);
        }
    }
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
