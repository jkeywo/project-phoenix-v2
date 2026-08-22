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
            emit_repair_ai_command(
                entity_uuid,
                crate::core::messages::SystemControlPayload::DispatchRepairTeam {
                    team_idx: team_idx as u8,
                    target,
                },
                sources,
                &sessions,
                config,
                &mut admitted,
            );
            excluded.insert(winner);
        }
    }
}

/// Emit an admitted Repair AI command targeting the repair system through the
/// shared [`crate::command_admission::validate_and_admit`] seam, using this
/// ship's own `ai:<uuid>` token (mirrors `emit_sensors_ai_command`).
fn emit_repair_ai_command(
    entity_uuid: Option<&crate::entities::spawner::EntityUuid>,
    payload: crate::core::messages::SystemControlPayload,
    sources: &ShipSystemControlSources,
    sessions: &crate::lobby::Sessions,
    config: &crate::ship_plugin::ShipConfigComponent,
    admitted: &mut crate::core::messages::AdmittedCommands,
) -> bool {
    emit_ai_command(
        entity_uuid,
        crate::ship::system_registry::repair_system_id(),
        payload,
        sources,
        sessions,
        Some(config),
        admitted,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::messages::*;
    use crate::lobby::{InboundMessage, LobbyPlugin, OutboundMessage};
    use crate::server_app::SimOutbox;
    use crate::server_app::{ShipImpulse, ShipShields};
    use crate::ship::damage::SystemHull;
    use crate::ship_plugin::ShipSystemControlSources;
    use crate::weapons::shield::ShieldSystem;

    #[derive(Resource, Default)]
    struct Outbox(Vec<OutboundMessage>);

    fn collect(mut reader: MessageReader<OutboundMessage>, mut box_: ResMut<Outbox>) {
        for m in reader.read() {
            box_.0.push(m.clone());
        }
    }

    fn test_app() -> App {
        let mut app = App::new();
        crate::ai::host::register_ai_host_env(&mut app);
        // `FixedUpdate`, where `RepairPlugin` and `AdmissionPlugin` register
        // since issue #895 — configured on `Update` this chain would order
        // nothing, leaving admission unordered against the repair handlers.
        app.configure_sets(
            FixedUpdate,
            (
                crate::sim_sets::SimSet::Input,
                crate::sim_sets::SimSet::Physics,
                crate::sim_sets::SimSet::Damage,
                crate::sim_sets::SimSet::Modifiers,
                crate::sim_sets::SimSet::Publish,
                crate::sim_sets::SimSet::PublishAggregate,
                crate::sim_sets::SimSet::Broadcast,
            )
                .chain(),
        )
        .add_plugins(LobbyPlugin)
        .add_plugins(bevy::time::TimePlugin)
        .add_plugins(crate::server_app::AdmissionPlugin)
        .init_resource::<crate::lobby::WorldResource>()
        .init_resource::<SimOutbox>()
        .init_resource::<Outbox>()
        .add_plugins(RepairPlugin)
        .add_plugins(repair_state_broadcaster())
        .add_systems(PostUpdate, collect);
        // One fixed step per update, 200 ms of sim time each (issue #895), so
        // the Hz-based repair broadcast timer fires within a single harness
        // tick.
        crate::ship::test_support::drive_one_fixed_step_per_update(
            &mut app,
            std::time::Duration::from_millis(200),
        );
        // Spawn the player ship entity so handle_dispatch_repair_team can query it.
        let hull_config = &[
            (SystemId("helm".into()), 25.0_f32),
            (SystemId("helm-engine-port".into()), 25.0),
            (SystemId("tactical".into()), 25.0),
            (SystemId("power".into()), 25.0),
            (SystemId("shields".into()), 25.0),
            (SystemId("core".into()), 50.0),
        ];
        app.world_mut().spawn((
            crate::server_app::Ship,
            crate::server_app::LocalShip,
            crate::ship_plugin::ShipConfigComponent::default(),
            crate::ship_plugin::ShipSystemControlSources::default(),
            crate::core::messages::AdmittedCommands::default(),
            crate::ship_plugin::ActiveStationRatings::default(),
            crate::ship_plugin::CoordinationQueue::default(),
            crate::entities::spawner::EntitySystemHull(SystemHull::from_config(hull_config)),
            crate::server_app::ShipSystemBlackboards::default(),
            ShipShields(ShieldSystem::default(), 0.5),
            ShipImpulse(crate::ship::impulse::ImpulseState::new()),
            crate::modifiers::ShipModifiers::new(),
            RepairRequestQueue::default(),
            // Nested tuple to keep the outer bundle within Bevy's 15-arity limit.
            // Issue #830: the global `ShipRepairTeams` Resource is gone; every
            // ship (including this test's LocalShip) carries its own component.
            (
                crate::ship_plugin::RepairHumanAlerted::default(),
                crate::ship_plugin::LastSystemTiers::default(),
                ShipRepairTeams(crate::modifiers::repair_teams::RepairTeams::new(2)),
                // The AUTHORED `[repair.selector]` block every shipped hull
                // carries. Since #885b stage 5d `operate_repair_ai` has no
                // synthesised fallback — a ship with no selector dispatches
                // nothing — so a fixture that wants dispatch must attach the
                // declaration a real hull writes.
                RepairTargetSelector {
                    selector: crate::entities::authored_ai_pins::shipped_selector_toml("repair")
                        .to_selector()
                        .expect("the shipped Repair selector decodes"),
                    power_rating: None,
                },
            ),
        ));
        app
    }

    /// Read the LocalShip's own `ShipRepairTeams` component (issue #830 — no
    /// global Resource). Returns an owned clone for assertion convenience.
    fn local_teams(app: &mut App) -> ShipRepairTeams {
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipRepairTeams, With<crate::server_app::LocalShip>>();
        q.single(app.world())
            .expect("LocalShip must carry ShipRepairTeams")
            .clone()
    }

    /// Dispatch a team on the LocalShip's own `ShipRepairTeams` component.
    fn dispatch_local(app: &mut App, idx: usize, sid: SystemId, name: &str) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipRepairTeams, With<crate::server_app::LocalShip>>();
        q.single_mut(app.world_mut())
            .expect("LocalShip must carry ShipRepairTeams")
            .0
            .dispatch(idx, sid, name.to_string());
    }

    /// Damage the named systems on the LocalShip's hull, adding a row for any
    /// the fixture hull does not already carry.
    ///
    /// The ids passed here are always systems the shipped battleship config
    /// (`ShipConfigComponent::default()`) OWNS from the station under test, so a
    /// `RepairTarget::Station` dispatch resolves through `systems_for_station`
    /// rather than through the station-name fallback.
    ///
    /// That matters since the issue #1013 review: `resolve_repair_target` no
    /// longer falls back to `SystemId(station_id)` when the station's own name
    /// is also a hull row, because such a row is OWNERLESS (bucketed under
    /// `core`) and sweeping from it would walk the team out of the station it
    /// was sent to. This fixture's coarse `helm`/`tactical`/`power`/`shields`
    /// rows are exactly that shape, so the dispatch tests below name the fine
    /// systems a production console would — the console's target list is built
    /// from the ship's fine hull rows.
    ///
    /// HP is set to 80% of max: below max, so the system is a resolvable repair
    /// target, but still `Operational`, so no tier crossing fires and no
    /// unrelated console is taken offline.
    fn damage_owned_fine_systems(app: &mut App, systems: &[&str]) {
        let at_80: Vec<(&str, f32)> = systems.iter().map(|id| (*id, 0.8)).collect();
        damage_owned_fine_systems_to(app, &at_80);
    }

    /// [`damage_owned_fine_systems`] with an explicit HP fraction per system, so
    /// a test can put a station's systems into DIFFERENT damage tiers and
    /// observe which one a sweep or a console tap picks out of them.
    fn damage_owned_fine_systems_to(app: &mut App, systems: &[(&str, f32)]) {
        let local_ship = {
            let mut q = app
                .world_mut()
                .query_filtered::<Entity, With<crate::server_app::LocalShip>>();
            q.single(app.world()).expect("one LocalShip")
        };
        let mut rows: Vec<(SystemId, f32)> = app
            .world()
            .get::<crate::entities::spawner::EntitySystemHull>(local_ship)
            .expect("LocalShip must carry EntitySystemHull")
            .0
            .iter()
            .map(|(sid, entry)| (sid.clone(), entry.max))
            .collect();
        for (id, _) in systems {
            let sid = SystemId((*id).into());
            if !rows.iter().any(|(existing, _)| *existing == sid) {
                rows.push((sid, 25.0));
            }
        }
        let mut hull = SystemHull::from_config(&rows);
        for (id, fraction) in systems {
            let sid = SystemId((*id).into());
            let max = hull.get(&sid).expect("just built this row").max;
            hull.set_hp(&sid, max * fraction);
        }
        app.world_mut()
            .entity_mut(local_ship)
            .insert(crate::entities::spawner::EntitySystemHull(hull));
    }

    fn repair_bb(app: &mut App) -> RepairBlackboard {
        let mut q = app
            .world_mut()
            .query_filtered::<&crate::server_app::ShipSystemBlackboards, With<crate::server_app::LocalShip>>();
        let bbs = q
            .single(app.world())
            .expect("LocalShip must have ShipSystemBlackboards");
        let key = SystemId(REPAIR_SYSTEM_ID.to_string());
        let SystemBlackboard::Repair(bb) = bbs.0.get(&key).unwrap() else {
            panic!("expected Repair blackboard");
        };
        bb.clone()
    }

    fn push(app: &mut App, token: &str, msg: ClientMessage) {
        app.world_mut()
            .resource_mut::<Messages<InboundMessage>>()
            .write(InboundMessage {
                token: token.into(),
                msg,
            });
    }

    fn tick(app: &mut App) -> Vec<OutboundMessage> {
        app.update();
        let sim_entries = std::mem::take(&mut app.world_mut().resource_mut::<SimOutbox>().0);
        let mut out = app.world().resource::<Outbox>().0.clone();
        for (target, msg) in sim_entries {
            out.push(OutboundMessage {
                target,
                msg,
                delivery: crate::core::messages::DeliveryClass::Reliable,
            });
        }
        app.world_mut().resource_mut::<Outbox>().0.clear();
        out
    }

    fn start_game(app: &mut App) {
        push(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(app);
        push(
            app,
            "eng",
            ClientMessage::Identify {
                token: "eng".into(),
                name: "Bob".into(),
            },
        );
        tick(app);
        push(
            app,
            "eng",
            ClientMessage::SelectStation {
                station: "Repair".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::SetReady { ready: true });
        push(app, "eng", ClientMessage::SetReady { ready: true });
        tick(app);
    }

    fn team_is_travelling(teams: &ShipRepairTeams, idx: usize) -> bool {
        matches!(
            teams.0.slots()[idx],
            crate::core::messages::TeamSlot::Travelling { .. }
        )
    }

    fn team_is_idle(teams: &ShipRepairTeams, idx: usize) -> bool {
        matches!(teams.0.slots()[idx], crate::core::messages::TeamSlot::Idle)
    }

    // ── Dispatch tests ──────────────────────────────────────────────────────

    /// Non-Repair console holder sending `DispatchRepairTeam` is ignored.
    #[test]
    fn non_repair_sender_is_ignored() {
        let mut app = test_app();
        start_game(&mut app);

        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::core::messages::SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: RepairTarget::Station(StationId("helm".into())),
                },
            },
        );
        tick(&mut app);

        let teams = local_teams(&mut app);
        assert!(
            team_is_idle(&teams, 0),
            "team 0 should remain idle after non-Repair dispatch"
        );
    }

    /// Repair holder dispatches team to a console → team enters Travelling.
    #[test]
    fn dispatch_sends_team_to_travelling() {
        let mut app = test_app();
        start_game(&mut app);
        damage_owned_fine_systems(&mut app, &["helm-engine-port"]);

        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: crate::core::messages::SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: RepairTarget::Station(StationId("helm".into())),
                },
            },
        );
        tick(&mut app);

        let teams = local_teams(&mut app);
        assert!(
            team_is_travelling(&teams, 0),
            "team 0 should be travelling after dispatch"
        );
    }

    /// A station dispatch must resolve to an owned fine hull system so a team
    /// can finish travelling and restore HP instead of immediately returning.
    #[test]
    fn station_dispatch_repairs_damaged_owned_fine_system() {
        let mut app = test_app();
        start_game(&mut app);

        let local_ship = {
            let mut query = app
                .world_mut()
                .query_filtered::<Entity, With<crate::server_app::LocalShip>>();
            query
                .single(app.world())
                .expect("test fixture must contain one LocalShip")
        };
        app.world_mut()
            .entity_mut(local_ship)
            .insert(ShipRepairTeams(
                crate::modifiers::repair_teams::RepairTeams::default(),
            ));

        let damaged_system = SystemId("helm-engine-port".into());
        let hp_before = 10.0;
        {
            let mut query = app.world_mut().query_filtered::<
                &mut crate::entities::spawner::EntitySystemHull,
                With<crate::server_app::LocalShip>,
            >();
            let mut hull = query
                .single_mut(app.world_mut())
                .expect("test fixture must contain one LocalShip hull");
            hull.0.set_hp(&damaged_system, hp_before);
        }

        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: SystemId(REPAIR_SYSTEM_ID.into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: RepairTarget::Station(StationId("helm".into())),
                },
            },
        );
        tick(&mut app);

        {
            let teams = local_teams(&mut app);
            let TeamSlot::Travelling { system_id, .. } = &teams.0.slots()[0] else {
                panic!("team 0 should be travelling to the damaged fine system");
            };
            assert_eq!(system_id.as_ref(), Some(&damaged_system));
        }

        // Default travel time is five seconds and the test clock advances 0.2s
        // per update. Run long enough to arrive and perform at least one repair.
        for _ in 0..30 {
            tick(&mut app);
        }

        let mut query = app.world_mut().query_filtered::<
            &crate::entities::spawner::EntitySystemHull,
            With<crate::server_app::LocalShip>,
        >();
        let hull = query
            .single(app.world())
            .expect("test fixture must contain one LocalShip hull");
        assert!(
            hull.0.current_for(&damaged_system).unwrap() > hp_before,
            "the arrived team should restore the station-owned fine system"
        );
    }

    /// When team is busy, dispatching to a different console redirects it.
    #[test]
    fn all_busy_teams_ignore_further_dispatches() {
        let mut app = test_app();
        start_game(&mut app);
        // One owned fine system per station this test addresses, so each
        // dispatch resolves without the (now refused) station-name fallback.
        damage_owned_fine_systems(
            &mut app,
            &["helm-engine-port", "tactical-radar", "power-reactor"],
        );

        // Dispatch both teams (default is 2).
        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: crate::core::messages::SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: RepairTarget::Station(StationId("helm".into())),
                },
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: crate::core::messages::SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 1,
                    target: RepairTarget::Station(StationId("tactical".into())),
                },
            },
        );
        tick(&mut app);

        // Redirect team 0 to Power (different console) — now team 0 is Returning with queue
        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: crate::core::messages::SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: RepairTarget::Station(StationId("power".into())),
                },
            },
        );
        tick(&mut app);

        let teams = local_teams(&mut app);
        // team 0 should be Returning (redirected), team 1 still Travelling
        assert!(matches!(
            &teams.0.slots()[0],
            crate::core::messages::TeamSlot::Returning { .. }
        ));
        assert!(team_is_travelling(&teams, 1));
    }

    /// RepairState broadcast includes the team slot states.
    #[test]
    fn repair_state_broadcast_includes_team_slots() {
        let mut app = test_app();
        start_game(&mut app);
        damage_owned_fine_systems(&mut app, &["helm-engine-port"]);

        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: crate::core::messages::SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: RepairTarget::Station(StationId("helm".into())),
                },
            },
        );
        let out1 = tick(&mut app);
        let out2 = tick(&mut app);

        let has_repair_state = out1.iter().chain(out2.iter()).any(|m| {
            matches!(&m.msg, ServerMessage::RepairState { teams } if
                teams.iter().any(|t| matches!(t, crate::core::messages::TeamSlot::Travelling { .. })))
        });
        assert!(
            has_repair_state,
            "RepairState should include a Travelling team after dispatch"
        );
    }

    // ── ControlSystem dispatch tests ─────────────────────────────────────────

    /// Repair holder dispatches via `ControlSystem` → team enters Travelling.
    #[test]
    fn control_system_dispatch_authorized_sends_team_to_travelling() {
        let mut app = test_app();
        start_game(&mut app);
        damage_owned_fine_systems(&mut app, &["helm-engine-port"]);

        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: crate::core::messages::SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: RepairTarget::Station(StationId("helm".into())),
                },
            },
        );
        tick(&mut app);

        let teams = local_teams(&mut app);
        assert!(
            team_is_travelling(&teams, 0),
            "team 0 should be travelling after ControlSystem dispatch"
        );
    }

    /// Non-Repair console holder sending `ControlSystem` dispatch is rejected.
    #[test]
    fn control_system_dispatch_unauthorized_sender_is_rejected() {
        let mut app = test_app();
        start_game(&mut app);

        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::core::messages::SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: RepairTarget::Station(StationId("helm".into())),
                },
            },
        );
        tick(&mut app);

        let teams = local_teams(&mut app);
        assert!(
            team_is_idle(&teams, 0),
            "team 0 should remain idle when non-Repair sender uses ControlSystem"
        );
    }

    /// `ControlSystem` dispatch is blocked when the repair system is AI-controlled.
    #[test]
    fn control_system_dispatch_rejected_when_ai_controlled() {
        let mut app = test_app();
        start_game(&mut app);

        // Set repair system to AI control.
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut ShipSystemControlSources, With<crate::server_app::LocalShip>>();
            for mut cs in q.iter_mut(app.world_mut()) {
                cs.0.set(
                    crate::ship::system_registry::repair_system_id(),
                    crate::ship::control_source::ControlSource::Ai,
                );
            }
        }

        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: crate::core::messages::SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: RepairTarget::Station(StationId("helm".into())),
                },
            },
        );
        tick(&mut app);

        let teams = local_teams(&mut app);
        assert!(
            team_is_idle(&teams, 0),
            "team 0 should remain idle when repair system is AI-controlled"
        );
    }

    #[test]
    fn control_system_dispatch_repair_target_core_dispatches_team() {
        let mut app = test_app();
        start_game(&mut app);

        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: crate::core::messages::SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: RepairTarget::Core,
                },
            },
        );
        tick(&mut app);

        let teams = local_teams(&mut app);
        assert!(
            team_is_travelling(&teams, 0),
            "team 0 should be travelling to Core after RepairTarget::Core dispatch"
        );
    }

    /// End-to-end TOML-driven wiring check: build the runtime `RepairTeams`
    /// the same way `spawn_game_start_entities` does (parse alliance_battleship.toml
    /// → RepairConfig::to_runtime → RepairTeams::new_with_timings) and
    /// assert the timings match the TOML. Changing
    /// `travel_duration_secs = 5.0` to e.g. `99.0` in alliance_battleship.toml
    /// would fail this test.
    #[test]
    fn repair_teams_resource_reflects_battleship_toml_repair_block() {
        // Through the resolver (issue #876): this hull is COMPOSED, so its baked
        // bytes are no longer the document `spawn_game_start_entities` reads.
        let config = crate::entities::include_resolve::load_entity_config(
            "assets/entities/alliance_battleship.toml",
        )
        .expect("alliance_battleship.toml must compose and parse");
        let rc = config
            .repair
            .expect("alliance_battleship must declare [repair]");
        let timings = rc.to_runtime();
        let teams = crate::modifiers::repair_teams::RepairTeams::new_with_timings(2, timings);
        assert_eq!(teams.timings().travel_duration, rc.travel_duration_secs);
        assert_eq!(
            teams.timings().repair_rate_hp_per_sec,
            rc.repair_rate_hp_per_sec
        );
        // And the runtime defaults still match (until someone intentionally
        // diverges them).
        let baseline = crate::modifiers::repair_teams::RepairTimings::default();
        assert_eq!(teams.timings().travel_duration, baseline.travel_duration);
        assert_eq!(
            teams.timings().repair_rate_hp_per_sec,
            baseline.repair_rate_hp_per_sec
        );
    }

    // ── Blackboard publish tests ─────────────────────────────────────────────

    #[test]
    fn publish_repair_blackboard_contains_teams_and_hull() {
        let mut app = test_app();
        start_game(&mut app);
        tick(&mut app);

        let bb = repair_bb(&mut app);
        assert!(!bb.teams.is_empty(), "expected at least one team slot");
        assert!(!bb.system_hull.is_empty(), "expected system_hull entries");
        assert!(
            bb.travel_duration_secs > 0.0,
            "expected positive travel duration"
        );
    }

    #[test]
    fn publish_repair_blackboard_reflects_dispatch() {
        let mut app = test_app();
        start_game(&mut app);
        tick(&mut app);

        dispatch_local(&mut app, 0, SystemId("helm".into()), "Helm");
        tick(&mut app);

        let bb = repair_bb(&mut app);
        assert!(
            bb.teams
                .iter()
                .any(|t| matches!(t, TeamSlot::Travelling { .. })),
            "expected a Travelling team slot after dispatch"
        );
    }

    #[test]
    fn publish_repair_blackboard_contains_damageable_systems() {
        let mut app = test_app();
        start_game(&mut app);
        tick(&mut app);

        let bb = repair_bb(&mut app);
        assert!(
            !bb.damageable_systems.is_empty(),
            "expected damageable_systems"
        );
        assert!(
            bb.damageable_systems.contains(&SystemId("helm".into())),
            "Helm should appear in damageable_systems"
        );
        assert!(
            bb.damageable_systems.contains(&SystemId("core".into())),
            "Core should appear in damageable_systems"
        );
    }

    /// A queue entry whose station's only system transitions Disabled→Destroyed
    /// must be RETAINED by the retain predicate (issue #1013 — the direct
    /// inverse of the pre-#1013 eviction).
    ///
    /// A destroyed system used to be treated as a lost cause, so its station's
    /// request was dropped and no team was ever sent again. Now the on-site
    /// sweep repairs destroyed systems, so dropping the request is precisely
    /// what would strand them: nothing else in the game clears a Destroyed
    /// latch.
    ///
    /// The predicate below is a copy of `operate_repair_ai`'s `rq.entries.retain`
    /// body. Change one and change the other — see
    /// `prune_retains_an_all_destroyed_station_through_the_ai_loop` for the test
    /// that runs the production copy.
    #[test]
    fn queue_entry_retained_when_all_systems_destroyed() {
        use crate::ship::config::{ShipConfig, SystemInstanceConfig};
        use crate::ship::damage::SystemHull;

        let station_id = "helm";
        let system_id = SystemId("helm".into());

        let config = ShipConfig {
            stations: vec![],
            systems: vec![SystemInstanceConfig {
                id: system_id.clone(),
                kind: "helm".into(),
                station: Some(StationId(station_id.into())),
                ai_only: false,
                human_seeking: false,
                seek_order: Vec::new(),
                power_group: None,
                marker: None,
                config: None,
            }],
            power_groups: Default::default(),
            coordination_lag_secs: 2.0,
        };

        let mut hull = SystemHull::from_config(&[(system_id.clone(), 25.0_f32)]);

        let mut rq = RepairRequestQueue { entries: vec![] };
        rq.entries.push(RepairQueueEntry {
            station_id: station_id.into(),
            station_label: "Helm".into(),
            tier: crate::ship::damage::DamageTier::Disabled,
            deficit: 25.0,
        });
        assert_eq!(rq.entries.len(), 1, "entry must be present before retain");

        hull.set_hp(&system_id, 0.0);
        assert_eq!(
            hull.tier_for(&system_id),
            crate::ship::damage::DamageTier::Destroyed,
            "system must be Destroyed after set_hp(0)"
        );

        rq.entries.retain(|entry| {
            config
                .systems
                .iter()
                .filter(|s| {
                    s.station.as_ref().map(|st| st.0.as_str()) == Some(entry.station_id.as_str())
                })
                .any(|s| hull.tier_for(&s.id) != crate::ship::damage::DamageTier::Operational)
        });

        assert_eq!(
            rq.entries.len(),
            1,
            "queue entry must be retained when all station systems are Destroyed \
             — the sweep can repair them"
        );

        // …and it IS dropped once the station is genuinely fixed.
        hull.set_hp(&system_id, 25.0);
        rq.entries.retain(|entry| {
            config
                .systems
                .iter()
                .filter(|s| {
                    s.station.as_ref().map(|st| st.0.as_str()) == Some(entry.station_id.as_str())
                })
                .any(|s| hull.tier_for(&s.id) != crate::ship::damage::DamageTier::Operational)
        });
        assert!(
            rq.entries.is_empty(),
            "a fully repaired station's entry must still be evicted"
        );
    }

    /// Verifies that operate_repair_ai loops over all entities with
    /// ShipSystemControlSources, gating on operate_ai (issue #590 AC).
    #[test]
    fn operate_repair_ai_runs_per_entity_for_ai_controlled_ships() {
        use crate::ship::control_source::{ControlSource, ControlSourceResolver};

        let mut ai_resolver = ControlSourceResolver::new();
        ai_resolver.set(
            crate::ship::system_registry::repair_system_id(),
            ControlSource::Ai,
        );
        let ai_sources = ShipSystemControlSources(ai_resolver);
        let policy = ai_sources
            .0
            .policy_for(&crate::ship::system_registry::repair_system_id());
        assert!(policy.operate_ai, "AI Repair must gate through operate_ai");

        let mut human_resolver = ControlSourceResolver::new();
        human_resolver.set(
            crate::ship::system_registry::repair_system_id(),
            ControlSource::Human,
        );
        let human_sources = ShipSystemControlSources(human_resolver);
        let human_policy = human_sources
            .0
            .policy_for(&crate::ship::system_registry::repair_system_id());
        assert!(!human_policy.operate_ai, "human Repair must not operate AI");
    }

    // ── NPC AI repair through admission (issue #830) ─────────────────────────

    /// A minimal ship config whose `helm` station owns a single `helm` fine
    /// system, so `resolve_repair_target(Station("helm"))` resolves to it.
    fn npc_repair_config() -> crate::ship_plugin::ShipConfigComponent {
        use crate::ship::config::{ShipConfig, SystemInstanceConfig};
        crate::ship_plugin::ShipConfigComponent(ShipConfig {
            stations: vec![],
            systems: vec![SystemInstanceConfig {
                id: SystemId("helm".into()),
                kind: "helm".into(),
                station: Some(StationId("helm".into())),
                ai_only: false,
                human_seeking: false,
                seek_order: Vec::new(),
                power_group: None,
                marker: None,
                config: None,
            }],
            power_groups: Default::default(),
            coordination_lag_secs: 2.0,
        })
    }

    /// Build an app that runs the full per-entity admitted repair pipeline —
    /// `operate_repair_ai` (emit) → `handle_dispatch_repair_team` (apply) →
    /// `tick_repair_teams` — chained so the same-tick emit→apply→repair shape of
    /// production holds. `Sessions` is present because `validate_and_admit`
    /// consults it (the `ai:` path only needs the resource to exist).
    fn npc_repair_app() -> App {
        let mut app = App::new();
        crate::ai::host::register_ai_host_env(&mut app);
        app.add_plugins(bevy::time::TimePlugin);
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(1000),
        ));
        app.insert_resource(crate::lobby::Sessions(
            crate::lobby::session::SessionManager::new(),
        ));
        // Stand in for `admit_system_commands`, which clears `AdmittedCommands`
        // once per tick before the AI decide systems refill it. Without this the
        // AI's `DispatchRepairTeam` would re-apply every tick and recall the team
        // (Travelling → Returning) forever, so it would never reach Repairing.
        app.add_systems(
            Update,
            (
                clear_admitted_commands,
                operate_repair_ai,
                crate::console::repair::dispatch::handle_dispatch_repair_team,
                tick_repair_teams,
            )
                .chain(),
        );
        app
    }

    /// Test-only mirror of admission's per-tick `AdmittedCommands` clear.
    fn clear_admitted_commands(mut q: Query<&mut crate::core::messages::AdmittedCommands>) {
        for mut admitted in q.iter_mut() {
            admitted.0.clear();
        }
    }

    /// Spawn an NPC ship (Ship marker, no LocalShip) whose Repair system is
    /// under the given control source, with a `helm` hull damaged by `damage`,
    /// a queue entry naming the `helm` station, an `EntityUuid` for its `ai:`
    /// token, and an empty `AdmittedCommands`.
    fn spawn_npc_repair(
        app: &mut App,
        source: crate::ship::control_source::ControlSource,
        damage: f32,
    ) -> Entity {
        use crate::ship::control_source::ControlSourceResolver;
        let mut resolver = ControlSourceResolver::new();
        resolver.set(repair_system_id(), source);

        let mut hull =
            crate::ship::damage::SystemHull::from_config(&[(SystemId("helm".into()), 100.0_f32)]);
        let mut rng = crate::sim_rng::unseeded_test_rng();
        hull.apply_damage(damage, &mut rng);

        let mut queue = RepairRequestQueue::default();
        queue.push_or_merge(RepairQueueEntry {
            station_id: "helm".into(),
            station_label: "Helm".into(),
            tier: DamageTier::Disabled,
            deficit: damage,
        });

        app.world_mut()
            .spawn((
                crate::server_app::Ship,
                crate::entities::spawner::EntityUuid("npc-repair-1".into()),
                ShipSystemControlSources(resolver),
                ShipRepairTeams(crate::modifiers::repair_teams::RepairTeams::new(2)),
                crate::entities::spawner::EntitySystemHull(hull),
                crate::modifiers::ShipModifiers::new(),
                queue,
                npc_repair_config(),
                crate::core::messages::AdmittedCommands::default(),
                // The AUTHORED `[repair.selector]` block: since #885b stage 5d
                // an NPC with no selector component ranks nothing and dispatches
                // no team.
                RepairTargetSelector {
                    selector: crate::entities::authored_ai_pins::shipped_selector_toml("repair")
                        .to_selector()
                        .expect("the shipped Repair selector decodes"),
                    power_rating: None,
                },
            ))
            .id()
    }

    /// The NPC applier consumes the AI operator's admitted `DispatchRepairTeam`
    /// in the same tick and sends a team travelling — proving the per-entity
    /// emit→admit→apply chain runs on an NPC ship with no LocalShip marker.
    #[test]
    fn npc_applier_consumes_ai_emitted_dispatch_same_tick() {
        let mut app = npc_repair_app();
        let npc = spawn_npc_repair(
            &mut app,
            crate::ship::control_source::ControlSource::Ai,
            80.0,
        );

        // One warm-up tick (TimePlugin baseline). The AI emits into the NPC's
        // own AdmittedCommands and the applier dispatches on the same tick.
        app.update();

        let teams = app
            .world()
            .get::<ShipRepairTeams>(npc)
            .expect("NPC must have ShipRepairTeams");
        assert!(
            teams
                .0
                .slots()
                .iter()
                .any(|s| matches!(s, TeamSlot::Travelling { .. })),
            "the NPC applier must have dispatched a team from the AI's own \
             AdmittedCommands, got {:?}",
            teams.0.slots()
        );
    }

    /// Regression for PRD #597 gap-5 (retained through #830): an NPC ship's
    /// AI-driven repair restores its own hull over time — now flowing through
    /// admission (`operate_repair_ai` emits, `handle_dispatch_repair_team`
    /// applies, `tick_repair_teams` heals) rather than a direct team write.
    #[test]
    fn npc_ship_with_repair_teams_regenerates_hull_over_time() {
        let mut app = npc_repair_app();
        let npc = spawn_npc_repair(
            &mut app,
            crate::ship::control_source::ControlSource::Ai,
            80.0,
        );
        let hp_before = app
            .world()
            .get::<crate::entities::spawner::EntitySystemHull>(npc)
            .unwrap()
            .0
            .total_current();

        // 200 iterations comfortably covers the 5 s travel + repair time.
        for _ in 0..200 {
            app.update();
        }

        let hp_after = app
            .world()
            .get::<crate::entities::spawner::EntitySystemHull>(npc)
            .expect("NPC must still have hull component")
            .0
            .total_current();
        assert!(
            hp_after > hp_before,
            "NPC hull HP must increase after AI-admitted dispatch + repair \
             (before={hp_before}, after={hp_after})"
        );
    }

    /// A human-held Repair system rejects an `ai:` emission at the admission
    /// gate: `validate_and_admit` returns false and nothing is admitted. This is
    /// the symmetry contract — the AI operator gates on `operate_ai` before
    /// emitting, and admission independently enforces it.
    #[test]
    fn human_held_repair_rejects_ai_emission() {
        use crate::ship::control_source::{ControlSource, ControlSourceResolver};
        let mut resolver = ControlSourceResolver::new();
        resolver.set(repair_system_id(), ControlSource::Human);
        let sources = ShipSystemControlSources(resolver);
        let sessions = crate::lobby::Sessions(crate::lobby::session::SessionManager::new());
        let config = npc_repair_config();
        let mut admitted = crate::core::messages::AdmittedCommands::default();

        let admitted_ok = crate::command_admission::validate_and_admit(
            "ai:npc-repair-1",
            repair_system_id(),
            SystemControlPayload::DispatchRepairTeam {
                team_idx: 0,
                target: RepairTarget::Station(StationId("helm".into())),
            },
            &sources,
            &sessions,
            &config.0,
            &mut admitted,
        );
        assert!(
            !admitted_ok,
            "ai: emission must be rejected when repair is human-held"
        );
        assert!(
            admitted.0.is_empty(),
            "no command may be admitted for a human-held repair system"
        );
    }

    /// SetRepairPriority is ignored when the team is not in Repairing state
    /// (e.g. idle). The handler runs but `RepairTeams::set_priority` returns
    /// false and the slot stays unaffected.
    #[test]
    fn set_repair_priority_on_idle_team_is_ignored() {
        let mut app = test_app();
        start_game(&mut app);
        tick(&mut app);

        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: SystemId("repair".into()),
                payload: SystemControlPayload::SetRepairPriority {
                    team_idx: 0,
                    priority: 3,
                },
            },
        );
        tick(&mut app);

        let teams = local_teams(&mut app);
        assert!(
            team_is_idle(&teams, 0),
            "team 0 should remain idle after SetRepairPriority on idle team"
        );
    }

    /// SetRepairPriority sets the team's priority when the team is actually
    /// in Repairing state. First dispatch the team, wait for it to arrive,
    /// then set priority.
    #[test]
    fn set_repair_priority_on_repairing_team_sets_priority() {
        let mut app = test_app();
        start_game(&mut app);
        // Damage a system the `helm` station OWNS so the dispatch resolves to
        // it and the team has work to do on arrival rather than leaving again.
        // (Damaging the bare `helm` hull ROW no longer works: it is a station
        // NAME, and since the #1013 review `resolve_repair_target` refuses to
        // fall back to a name that is also an ownerless hull row.)
        damage_owned_fine_systems(&mut app, &["helm-engine-port"]);

        // Dispatch team 0 to helm.
        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: RepairTarget::Station(StationId("helm".into())),
                },
            },
        );
        // Tick past travel time (5s default) so team arrives and enters Repairing.
        for _ in 0..30 {
            tick(&mut app);
        }

        // Verify team is Repairing.
        {
            let teams = local_teams(&mut app);
            assert!(
                matches!(&teams.0.slots()[0], TeamSlot::Repairing { .. }),
                "team 0 should be Repairing after travel, got {:?}",
                teams.0.slots()[0]
            );
        }

        // Now send SetRepairPriority.
        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: SystemId("repair".into()),
                payload: SystemControlPayload::SetRepairPriority {
                    team_idx: 0,
                    priority: 2,
                },
            },
        );
        tick(&mut app);

        let teams = local_teams(&mut app);
        assert!(
            matches!(
                &teams.0.slots()[0],
                TeamSlot::Repairing {
                    priority: Some(2),
                    ..
                }
            ),
            "team 0 should have priority=2 after SetRepairPriority, got {:?}",
            teams.0.slots()[0]
        );
    }

    // ── Naming a system instead of an ordinal (issue #1015) ──────────────────

    /// Put team 0 on site at the `helm` station with three systems in three
    /// different states, so the remaining work has a non-trivial ranking:
    /// the team lands on `helm-thrust` (the worst, Disabled) and what is left
    /// ranks `helm-engine-port` first and `helm-engine-starboard` second.
    fn team_on_site_at_helm_with_two_jobs_left(app: &mut App) {
        damage_owned_fine_systems_to(
            app,
            &[
                ("helm-thrust", 0.2),           // Disabled — the dispatch target
                ("helm-engine-port", 0.5),      // Damaged, rank 1 of what remains
                ("helm-engine-starboard", 0.6), // Damaged, rank 2
            ],
        );
        push(
            app,
            "eng",
            ClientMessage::ControlSystem {
                target: SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: RepairTarget::Station(StationId("helm".into())),
                },
            },
        );
        for _ in 0..30 {
            tick(app);
        }
        let teams = local_teams(app);
        assert!(
            matches!(
                &teams.0.slots()[0],
                TeamSlot::Repairing { system_id: Some(s), .. } if s.0 == "helm-thrust"
            ),
            "fixture precondition: team 0 works the worst helm system first, got {:?}",
            teams.0.slots()[0]
        );
    }

    /// The repair console's damaged-systems tap, end to end through admission:
    /// the client names a SYSTEM and the host records that system on the team's
    /// slot. It records no ordinal — `priority` is #1013's standing per-team
    /// instruction and a tap does not touch it, because a rank frozen at tap
    /// time can select a different system by the time the hand-off consumes it
    /// (see `RepairTeams::prioritise_system`).
    #[test]
    fn set_repair_target_priority_pins_the_tapped_system_host_side() {
        let mut app = test_app();
        start_game(&mut app);
        team_on_site_at_helm_with_two_jobs_left(&mut app);

        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: SystemId("repair".into()),
                payload: SystemControlPayload::SetRepairTargetPriority {
                    system_id: SystemId("helm-engine-starboard".into()),
                },
            },
        );
        tick(&mut app);

        let teams = local_teams(&mut app);
        assert!(
            matches!(
                &teams.0.slots()[0],
                TeamSlot::Repairing {
                    priority: None,
                    priority_system_id: Some(pinned),
                    ..
                } if pinned.0 == "helm-engine-starboard"
            ),
            "the tap must pin the named system and write no ordinal, got {:?}",
            teams.0.slots()[0]
        );
    }

    /// The sweep actually goes there: after the tapped system is prioritised the
    /// team hands off to it rather than to the worse-ranked one it would
    /// otherwise have taken. This is the acceptance criterion in observable
    /// form — no assertion on the ordinal at all.
    #[test]
    fn set_repair_target_priority_sends_the_sweep_to_the_tapped_system() {
        let mut app = test_app();
        start_game(&mut app);
        team_on_site_at_helm_with_two_jobs_left(&mut app);

        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: SystemId("repair".into()),
                payload: SystemControlPayload::SetRepairTargetPriority {
                    system_id: SystemId("helm-engine-starboard".into()),
                },
            },
        );
        // Long enough for the team to finish `helm-thrust` and hand off.
        for _ in 0..600 {
            tick(&mut app);
            let teams = local_teams(&mut app);
            if let TeamSlot::Repairing {
                system_id: Some(sid),
                ..
            } = &teams.0.slots()[0]
            {
                if sid.0 != "helm-thrust" {
                    assert_eq!(
                        sid.0, "helm-engine-starboard",
                        "the sweep must hand off to the TAPPED system, not the \
                         worst remaining one"
                    );
                    return;
                }
            }
        }
        panic!("team 0 never handed off to its next job");
    }

    /// A tap naming a system no on-site team is sweeping is a silent no-op —
    /// the same nothing-happens a dispatch to an undamaged station produces.
    #[test]
    fn set_repair_target_priority_for_an_unswept_system_changes_nothing() {
        let mut app = test_app();
        start_game(&mut app);
        team_on_site_at_helm_with_two_jobs_left(&mut app);

        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: SystemId("repair".into()),
                payload: SystemControlPayload::SetRepairTargetPriority {
                    system_id: SystemId("tactical-phaser-fore".into()),
                },
            },
        );
        tick(&mut app);

        let teams = local_teams(&mut app);
        assert!(
            matches!(
                &teams.0.slots()[0],
                TeamSlot::Repairing {
                    priority: None,
                    priority_system_id: None,
                    ..
                }
            ),
            "got {:?}",
            teams.0.slots()[0]
        );
    }

    /// The same station-ownership gate every repair command sits behind: a token
    /// that does not hold Engineering cannot steer the sweep. Admission decides
    /// this from the TARGET system, so the new payload inherits it — this test
    /// is what pins that inheritance rather than assuming it.
    #[test]
    fn set_repair_target_priority_from_a_non_engineering_token_is_rejected() {
        let mut app = test_app();
        start_game(&mut app);
        team_on_site_at_helm_with_two_jobs_left(&mut app);

        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: SystemId("repair".into()),
                payload: SystemControlPayload::SetRepairTargetPriority {
                    system_id: SystemId("helm-engine-starboard".into()),
                },
            },
        );
        tick(&mut app);

        let teams = local_teams(&mut app);
        assert!(
            matches!(
                &teams.0.slots()[0],
                TeamSlot::Repairing {
                    priority: None,
                    priority_system_id: None,
                    ..
                }
            ),
            "an unauthorised tap must not reach the team, got {:?}",
            teams.0.slots()[0]
        );
    }

    // ── Authored repair-target ranking (issue #785) ──────────────────────────
    //
    // AC6: every assertion below reads OBSERVABLE state — `TeamSlot` variants
    // and their `system_id`, `RepairRequestQueue.entries`, and
    // `EntitySystemHull` HP — never a `TargetSelector::select` return value.
    // The selector's own semantics are unit-tested in `src/ai/selector.rs`.

    /// Two stations, each owning one fine system, so a station dispatch resolves
    /// to a distinct observable `system_id`.
    fn two_station_config() -> crate::ship_plugin::ShipConfigComponent {
        use crate::ship::config::{ShipConfig, SystemInstanceConfig};
        let sys = |id: &str, station: &str| SystemInstanceConfig {
            id: SystemId(id.into()),
            kind: "generic".into(),
            station: Some(StationId(station.into())),
            ai_only: false,
            human_seeking: false,
            seek_order: Vec::new(),
            power_group: None,
            marker: None,
            config: None,
        };
        crate::ship_plugin::ShipConfigComponent(ShipConfig {
            stations: vec![],
            systems: vec![sys("alpha-sys", "alpha"), sys("bravo-sys", "bravo")],
            power_groups: Default::default(),
            coordination_lag_secs: 2.0,
        })
    }

    /// Spawn a two-station NPC. `alpha_hp` / `bravo_hp` are absolute HP out of
    /// 100, and each station gets a repair-request queue entry carrying the tier
    /// the hull actually reports (the coordination-delivered reading the AI
    /// ranks). `teams` is the team count. An optional authored selector is
    /// attached as `RepairTargetSelector`; `None` uses the
    /// SHIPPED authored one (there is no host fallback since #885b stage 5d).
    fn spawn_two_station_npc(
        app: &mut App,
        source: crate::ship::control_source::ControlSource,
        alpha_hp: f32,
        bravo_hp: f32,
        teams: usize,
        selector: Option<crate::entities::config::FineSystemAiSelectorToml>,
    ) -> Entity {
        use crate::ship::control_source::ControlSourceResolver;
        let mut resolver = ControlSourceResolver::new();
        resolver.set(repair_system_id(), source);

        let mut hull = crate::ship::damage::SystemHull::from_config(&[
            (SystemId("alpha-sys".into()), 100.0_f32),
            (SystemId("bravo-sys".into()), 100.0_f32),
        ]);
        hull.set_hp(&SystemId("alpha-sys".into()), alpha_hp);
        hull.set_hp(&SystemId("bravo-sys".into()), bravo_hp);

        let mut queue = RepairRequestQueue::default();
        for (station, sid, hp) in [
            ("alpha", "alpha-sys", alpha_hp),
            ("bravo", "bravo-sys", bravo_hp),
        ] {
            // Everything non-Operational is queued, Destroyed included: since
            // issue #1013 a destroyed station is a real repair job, and the
            // production enqueue (`RepairRequestQueue::push_or_merge`) no
            // longer drops it either.
            let tier = hull.tier_for(&SystemId(sid.into()));
            if tier == DamageTier::Operational {
                continue;
            }
            queue.push_or_merge(RepairQueueEntry {
                station_id: station.into(),
                station_label: station.into(),
                tier,
                deficit: 100.0 - hp,
            });
        }

        let entity = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                crate::entities::spawner::EntityUuid("npc-repair-2".into()),
                ShipSystemControlSources(resolver),
                ShipRepairTeams(crate::modifiers::repair_teams::RepairTeams::new(teams)),
                crate::entities::spawner::EntitySystemHull(hull),
                crate::modifiers::ShipModifiers::new(),
                queue,
                two_station_config(),
                crate::core::messages::AdmittedCommands::default(),
                // The AUTHORED `[repair.selector]` block, unless the caller
                // supplies its own below. Since #885b stage 5d there is no host
                // fallback: a ship with no selector component dispatches nothing.
                RepairTargetSelector {
                    selector: crate::entities::authored_ai_pins::shipped_selector_toml("repair")
                        .to_selector()
                        .expect("the shipped Repair selector decodes"),
                    power_rating: None,
                },
            ))
            .id();
        if let Some(cfg) = selector {
            app.world_mut()
                .entity_mut(entity)
                .insert(RepairTargetSelector {
                    selector: cfg.to_selector().expect("test selector must parse"),
                    power_rating: None,
                });
        }
        entity
    }

    /// The observable target system of a team slot, if it has one.
    fn slot_system(slot: &TeamSlot) -> Option<String> {
        match slot {
            TeamSlot::Travelling { system_id, .. } | TeamSlot::Repairing { system_id, .. } => {
                system_id.as_ref().map(|s| s.0.clone())
            }
            _ => None,
        }
    }

    fn team_systems(app: &App, entity: Entity) -> Vec<Option<String>> {
        app.world()
            .get::<ShipRepairTeams>(entity)
            .expect("ship must carry ShipRepairTeams")
            .0
            .slots()
            .iter()
            .map(slot_system)
            .collect()
    }

    /// Issue #891 stage 2, per-host both-directions proof for the Repair
    /// target selector: an authored eligibility gated on a world flag
    /// dispatches nothing while the flag is clear and dispatches the damaged
    /// station once it is set.
    #[test]
    fn operate_repair_ai_flag_guard_reads_the_world_in_both_directions() {
        use crate::entities::config::{FineSystemAiSelectorToml, ScoreTermToml};
        let flag_gated = FineSystemAiSelectorToml {
            param: std::collections::HashMap::new(),
            sources: vec![crate::entities::config::SELECTOR_SOURCE_DAMAGED_STATIONS.to_string()],
            horizon: 1.0e9,
            switch_margin: 0.0,
            eligibility: "candidate_fact(source_repair_request) > 0 \
                          and flag(damage_control_released)"
                .to_string(),
            score: vec![ScoreTermToml {
                when: "candidate_fact(source_repair_request) > 0".to_string(),
                weight: 1.0,
            }],
        };

        let mut app = npc_repair_app();
        app.init_resource::<crate::world::server::WorldContentRuntime>();
        // alpha 50/100 → Damaged, one team free.
        let npc = spawn_two_station_npc(
            &mut app,
            crate::ship::control_source::ControlSource::Ai,
            50.0,
            100.0,
            1,
            Some(flag_gated),
        );

        // Flag CLEAR → nothing is eligible, the team stays idle.
        app.update();
        assert_eq!(
            team_systems(&app, npc)[0],
            None,
            "with the world flag clear the eligibility must dispatch nothing"
        );

        // Flag SET → the SAME eligibility dispatches to the damaged station.
        app.world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .flags
            .set_flag("damage_control_released");
        app.update();
        assert_eq!(
            team_systems(&app, npc)[0].as_deref(),
            Some("alpha-sys"),
            "with the world flag set the same eligibility must dispatch the team"
        );
    }

    /// AC2 baseline: with the canonical default selector the worst-tier station
    /// wins, exactly as the retired `(tier desc, deficit desc)` comparator did.
    #[test]
    fn default_repair_selector_dispatches_worst_tier_station_first() {
        let mut app = npc_repair_app();
        // alpha 50/100 → Damaged; bravo 10/100 → Disabled (worse tier).
        let npc = spawn_two_station_npc(
            &mut app,
            crate::ship::control_source::ControlSource::Ai,
            50.0,
            10.0,
            1,
            None,
        );
        app.update();

        assert_eq!(
            team_systems(&app, npc)[0].as_deref(),
            Some("bravo-sys"),
            "the worse-tier station must win the default ranking"
        );
    }

    /// AC2: an AUTHORED eligibility changes which station is dispatched, proving
    /// the decision comes from data and not from a Rust comparator. Here the
    /// author restricts eligibility to the merely-Damaged tier, so the team goes
    /// to `alpha` — the opposite of the default ranking asserted above.
    #[test]
    fn authored_repair_selector_drives_dispatch() {
        use crate::entities::config::{FineSystemAiSelectorToml, ScoreTermToml};
        let authored = FineSystemAiSelectorToml {
            param: std::collections::HashMap::from([("tier_weight".to_string(), 10.0_f32)]),
            sources: vec![crate::entities::config::SELECTOR_SOURCE_DAMAGED_STATIONS.to_string()],
            horizon: 1.0e9,
            switch_margin: 0.0,
            eligibility: "candidate_fact(source_repair_request) > 0 \
                          and candidate_fact(assigned) < 1 \
                          and candidate_fact(tier_ordinal) == 1"
                .to_string(),
            score: vec![ScoreTermToml {
                when: "candidate_fact(tier_ordinal) >= 1".to_string(),
                weight: 10.0,
            }],
        };

        let mut app = npc_repair_app();
        let npc = spawn_two_station_npc(
            &mut app,
            crate::ship::control_source::ControlSource::Ai,
            50.0,
            10.0,
            1,
            Some(authored),
        );
        app.update();

        assert_eq!(
            team_systems(&app, npc)[0].as_deref(),
            Some("alpha-sys"),
            "the authored eligibility must override the default worst-tier pick"
        );
    }

    /// AC2/AC4: two free teams pick two DISTINCT stations in one tick — the
    /// per-team exclusion, expressed through the authored `assigned` fact.
    #[test]
    fn two_free_teams_are_dispatched_to_distinct_stations() {
        let mut app = npc_repair_app();
        let npc = spawn_two_station_npc(
            &mut app,
            crate::ship::control_source::ControlSource::Ai,
            50.0,
            10.0,
            2,
            None,
        );
        app.update();

        let systems = team_systems(&app, npc);
        assert_eq!(
            systems,
            vec![Some("bravo-sys".to_string()), Some("alpha-sys".to_string())],
            "ascending team indices must take the ranking in order, without \
             both teams piling onto the same station"
        );
    }

    /// AC4 determinism: two stations at the SAME tier and the SAME deficit must
    /// resolve to the same station on every run — the selector's smallest-key
    /// tie-break, not queue insertion order. Repeated across fresh apps because
    /// a single run cannot observe executor variation.
    #[test]
    fn tied_repair_candidates_resolve_deterministically() {
        for _ in 0..20 {
            let mut app = npc_repair_app();
            let npc = spawn_two_station_npc(
                &mut app,
                crate::ship::control_source::ControlSource::Ai,
                50.0,
                50.0,
                1,
                None,
            );
            app.update();
            assert_eq!(
                team_systems(&app, npc)[0].as_deref(),
                Some("alpha-sys"),
                "a full tie must always resolve to the smallest station id"
            );
        }
    }

    /// AC4 "completed targets removed": once a station's systems are back to
    /// Operational its repair-request entry is pruned and no further team is
    /// sent there. AC5 falls out of the same observation — the retained pick
    /// lives only in the authoritative `TeamSlot`, so a healed station simply
    /// stops being a candidate with no AI state to reset.
    #[test]
    fn repaired_station_entry_is_removed_and_not_redispatched() {
        let mut app = npc_repair_app();
        let npc = spawn_two_station_npc(
            &mut app,
            crate::ship::control_source::ControlSource::Ai,
            50.0,
            100.0,
            1,
            None,
        );
        app.update();
        assert_eq!(
            team_systems(&app, npc)[0].as_deref(),
            Some("alpha-sys"),
            "the only damaged station must be picked first"
        );

        // Heal alpha outright, then tick: the entry must vanish.
        app.world_mut()
            .get_mut::<crate::entities::spawner::EntitySystemHull>(npc)
            .unwrap()
            .0
            .set_hp(&SystemId("alpha-sys".into()), 100.0);
        app.update();

        assert!(
            app.world()
                .get::<RepairRequestQueue>(npc)
                .unwrap()
                .entries
                .is_empty(),
            "a fully repaired station's queue entry must be removed"
        );

        // Free every team and tick again: nothing is eligible, so nothing is sent.
        app.world_mut().get_mut::<ShipRepairTeams>(npc).unwrap().0 =
            crate::modifiers::repair_teams::RepairTeams::new(1);
        app.update();
        assert!(
            team_systems(&app, npc).iter().all(|s| s.is_none()),
            "no team may be dispatched once every reported station is repaired"
        );
    }

    /// The #779 EMPTY-FACTS lesson: candidate facts are really seeded, so an
    /// authored `candidate_fact(...)` guard actually fires. The same selector is
    /// run twice with only its threshold param moved across the observed
    /// `damage_fraction` (0.5) — below it dispatches, above it does not.
    #[test]
    fn authored_candidate_fact_guard_fires_on_seeded_damage_fraction() {
        use crate::entities::config::{FineSystemAiSelectorToml, ScoreTermToml};
        let selector_with = |threshold: f32| FineSystemAiSelectorToml {
            param: std::collections::HashMap::from([("min_damage".to_string(), threshold)]),
            sources: vec![crate::entities::config::SELECTOR_SOURCE_DAMAGED_STATIONS.to_string()],
            horizon: 1.0e9,
            switch_margin: 0.0,
            eligibility: "candidate_fact(assigned) < 1 \
                          and candidate_fact(damage_fraction) >= param(min_damage)"
                .to_string(),
            score: vec![ScoreTermToml {
                when: "candidate_fact(damage_fraction) >= param(min_damage)".to_string(),
                weight: 1.0,
            }],
        };

        // Threshold below the seeded 0.5 damage fraction → the guard fires.
        let mut app = npc_repair_app();
        let npc = spawn_two_station_npc(
            &mut app,
            crate::ship::control_source::ControlSource::Ai,
            50.0,
            100.0,
            1,
            Some(selector_with(0.4)),
        );
        app.update();
        assert_eq!(
            team_systems(&app, npc)[0].as_deref(),
            Some("alpha-sys"),
            "a guard under the seeded damage_fraction must fire — if facts were \
             empty this would never dispatch"
        );

        // Threshold above it → the same guard cannot fire, so nothing is sent.
        let mut app = npc_repair_app();
        let npc = spawn_two_station_npc(
            &mut app,
            crate::ship::control_source::ControlSource::Ai,
            50.0,
            100.0,
            1,
            Some(selector_with(0.9)),
        );
        app.update();
        assert!(
            team_systems(&app, npc)[0].is_none(),
            "a guard above the seeded damage_fraction must not fire"
        );
    }

    /// AC5 human takeover: with Repair human-held the AI never emits, so no team
    /// leaves the bay however damaged the ship is.
    #[test]
    fn human_held_repair_stops_ai_dispatch() {
        let mut app = npc_repair_app();
        let npc = spawn_two_station_npc(
            &mut app,
            crate::ship::control_source::ControlSource::Human,
            50.0,
            10.0,
            2,
            None,
        );
        for _ in 0..5 {
            app.update();
        }
        assert!(
            team_systems(&app, npc).iter().all(|s| s.is_none()),
            "a human-held Repair system must not auto-dispatch"
        );
    }

    /// AC3 + observable outcome: the authored ranking flows through the ordinary
    /// typed input and the ordinary team-assignment path, so the picked
    /// station's fine system actually gains HP.
    #[test]
    fn authored_ranking_restores_hull_through_the_normal_dispatch_path() {
        let mut app = npc_repair_app();
        let npc = spawn_two_station_npc(
            &mut app,
            crate::ship::control_source::ControlSource::Ai,
            50.0,
            10.0,
            1,
            None,
        );
        let before = app
            .world()
            .get::<crate::entities::spawner::EntitySystemHull>(npc)
            .unwrap()
            .0
            .current_for(&SystemId("bravo-sys".into()))
            .unwrap();
        for _ in 0..200 {
            app.update();
        }
        let after = app
            .world()
            .get::<crate::entities::spawner::EntitySystemHull>(npc)
            .unwrap()
            .0
            .current_for(&SystemId("bravo-sys".into()))
            .unwrap();
        assert!(
            after > before,
            "the ranked station's system must actually heal (before={before}, \
             after={after})"
        );
    }

    /// AC1/AC2 for the `core-bucket` source: a repair request naming the
    /// ownerless `core` bucket dispatches `RepairTarget::Core` — observable as
    /// the team slot's `core` system id — AND outranks a SAME-TIER but less
    /// damaged real station.
    ///
    /// This is the regression that the station-owned reading path could not
    /// see: `core` owns no station in `ShipConfig` (validation forbids one), so
    /// the station scan reports `(0.0, 0.0, 0)` for it. Left seeded, the core
    /// candidate scores nothing from the deficit ladder and the healthier
    /// `helm` wins.
    #[test]
    fn core_bucket_request_outranks_less_damaged_same_tier_station() {
        use crate::ship::config::{ShipConfig, SystemInstanceConfig};

        let mut app = npc_repair_app();
        let mut resolver = crate::ship::control_source::ControlSourceResolver::new();
        resolver.set(
            repair_system_id(),
            crate::ship::control_source::ControlSource::Ai,
        );

        // `helm-sys` at 20/100 → Disabled, damage fraction 0.80, deficit 80.
        // The ownerless `core` hull entry at 10/100 → Disabled, damage fraction
        // 0.90, deficit 90. Same tier, so only the deficit ladder separates
        // them — and only if the core candidate carries its real hull reading.
        let mut hull = crate::ship::damage::SystemHull::from_config(&[
            (SystemId("helm-sys".into()), 100.0_f32),
            (SystemId(REPAIR_CORE_BUCKET_KEY.into()), 100.0_f32),
        ]);
        hull.set_hp(&SystemId("helm-sys".into()), 20.0);
        hull.set_hp(&SystemId(REPAIR_CORE_BUCKET_KEY.into()), 10.0);
        assert_eq!(
            hull.tier_for(&SystemId("helm-sys".into())),
            DamageTier::Disabled
        );
        assert_eq!(
            hull.tier_for(&SystemId(REPAIR_CORE_BUCKET_KEY.into())),
            DamageTier::Disabled,
            "the scenario only bites while both candidates share a tier"
        );

        let mut queue = RepairRequestQueue::default();
        queue.push_or_merge(RepairQueueEntry {
            station_id: "helm".into(),
            station_label: "helm".into(),
            tier: DamageTier::Disabled,
            deficit: 80.0,
        });
        // `damage_sync` files an ownerless system's request under this id.
        queue.push_or_merge(RepairQueueEntry {
            station_id: REPAIR_CORE_BUCKET_KEY.into(),
            station_label: REPAIR_CORE_BUCKET_KEY.into(),
            tier: DamageTier::Disabled,
            deficit: 90.0,
        });

        // NOTE: no station named `core` — `ShipConfig` validation forbids it,
        // which is exactly why the core bucket needs its hull-side reading.
        let config = crate::ship_plugin::ShipConfigComponent(ShipConfig {
            stations: vec![],
            systems: vec![SystemInstanceConfig {
                id: SystemId("helm-sys".into()),
                kind: "generic".into(),
                station: Some(StationId("helm".into())),
                ai_only: false,
                human_seeking: false,
                seek_order: Vec::new(),
                power_group: None,
                marker: None,
                config: None,
            }],
            power_groups: Default::default(),
            coordination_lag_secs: 2.0,
        });

        let npc = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                crate::entities::spawner::EntityUuid("npc-repair-core".into()),
                ShipSystemControlSources(resolver),
                ShipRepairTeams(crate::modifiers::repair_teams::RepairTeams::new(1)),
                crate::entities::spawner::EntitySystemHull(hull),
                crate::modifiers::ShipModifiers::new(),
                queue,
                config,
                crate::core::messages::AdmittedCommands::default(),
                // The AUTHORED `[repair.selector]` block — the deficit ladder
                // under test lives in it, and since #885b stage 5d nothing
                // supplies one for a ship that carries no component.
                RepairTargetSelector {
                    selector: crate::entities::authored_ai_pins::shipped_selector_toml("repair")
                        .to_selector()
                        .expect("the shipped Repair selector decodes"),
                    power_rating: None,
                },
            ))
            .id();

        app.update();

        assert_eq!(
            team_systems(&app, npc)[0].as_deref(),
            Some(REPAIR_CORE_BUCKET_KEY),
            "the more-damaged core bucket must win over the same-tier `helm` \
             station, and must dispatch as RepairTarget::Core"
        );
    }

    // ── The ownerless GROUP, not just the `core` row (issue #1013 review) ─────

    /// A CRUISER-SHAPED hull: two ownerless rows, not one.
    ///
    /// `alliance_cruiser` authors a `science` `[[hull.system_hull]]` with no
    /// `[[system]]` behind it, so it joins `core` in the ownerless bucket —
    /// every other shipped hull's ownerless group is `{core}` alone, which is
    /// why unit fixtures never exposed this. `hull` carries `core` at full HP,
    /// the ownerless `science` row destroyed, and a station-owned `helm-sys` at
    /// full HP that must NOT be mistaken for ownerless.
    fn spawn_two_ownerless_rows(
        app: &mut App,
        team_count: usize,
        core_hp: f32,
        science_max: f32,
    ) -> Entity {
        use crate::ship::config::{ShipConfig, SystemInstanceConfig};
        let mut resolver = crate::ship::control_source::ControlSourceResolver::new();
        resolver.set(
            repair_system_id(),
            crate::ship::control_source::ControlSource::Ai,
        );

        let mut hull = crate::ship::damage::SystemHull::from_config(&[
            (SystemId(REPAIR_CORE_BUCKET_KEY.into()), 20.0_f32),
            (SystemId("science".into()), science_max),
            (SystemId("helm-sys".into()), 20.0),
        ]);
        hull.set_hp(&SystemId(REPAIR_CORE_BUCKET_KEY.into()), core_hp);
        hull.set_hp(&SystemId("science".into()), 0.0);

        // `damage_sync` files EVERY ownerless system's request under this one
        // id, so the destroyed `science` row is reported as a `core` request.
        let mut queue = RepairRequestQueue::default();
        queue.push_or_merge(RepairQueueEntry {
            station_id: REPAIR_CORE_BUCKET_KEY.into(),
            station_label: REPAIR_CORE_BUCKET_KEY.into(),
            tier: DamageTier::Destroyed,
            deficit: science_max,
        });

        // Only `helm-sys` is described; `core` and `science` are ownerless.
        let config = crate::ship_plugin::ShipConfigComponent(ShipConfig {
            stations: vec![],
            systems: vec![SystemInstanceConfig {
                id: SystemId("helm-sys".into()),
                kind: "generic".into(),
                station: Some(StationId("helm".into())),
                ai_only: false,
                human_seeking: false,
                seek_order: Vec::new(),
                power_group: None,
                marker: None,
                config: None,
            }],
            power_groups: Default::default(),
            coordination_lag_secs: 2.0,
        });

        app.world_mut()
            .spawn((
                crate::server_app::Ship,
                crate::entities::spawner::EntityUuid("npc-two-ownerless".into()),
                ShipSystemControlSources(resolver),
                ShipRepairTeams(crate::modifiers::repair_teams::RepairTeams::new(team_count)),
                crate::entities::spawner::EntitySystemHull(hull),
                crate::modifiers::ShipModifiers::new(),
                queue,
                config,
                crate::core::messages::AdmittedCommands::default(),
                RepairTargetSelector {
                    selector: crate::entities::authored_ai_pins::shipped_selector_toml("repair")
                        .to_selector()
                        .expect("the shipped Repair selector decodes"),
                    power_rating: None,
                },
            ))
            .id()
    }

    fn queue_station_ids(app: &App, entity: Entity) -> Vec<String> {
        app.world()
            .get::<RepairRequestQueue>(entity)
            .expect("ship must carry RepairRequestQueue")
            .entries
            .iter()
            .map(|e| e.station_id.clone())
            .collect()
    }

    /// The core request must survive while ANY ownerless row still needs a team,
    /// not just while the literal `core` row does — and the team it keeps alive
    /// must actually reach and repair that row.
    ///
    /// Before the fix the prune tested `tier_for(SystemId("core"))` alone, so a
    /// cruiser with `core` Operational and `science` destroyed had the entry
    /// evicted on the first tick: no request, no candidate, no team, and nothing
    /// else in the game clears a Destroyed latch — the row stayed destroyed for
    /// the rest of the match.
    #[test]
    fn core_request_survives_while_a_non_core_ownerless_row_is_damaged() {
        let mut app = npc_repair_app();
        // `core` at FULL HP — the whole point: the literal core row is
        // Operational and only its ownerless sibling needs work.
        let npc = spawn_two_ownerless_rows(&mut app, 1, 20.0, 4.0);

        app.update();

        assert_eq!(
            queue_station_ids(&app, npc),
            vec![REPAIR_CORE_BUCKET_KEY.to_string()],
            "the core request must be retained while a non-core ownerless row \
             is non-Operational, even with `core` itself Operational"
        );
        assert_eq!(
            team_systems(&app, npc)[0].as_deref(),
            Some(REPAIR_CORE_BUCKET_KEY),
            "and a team must be dispatched for it"
        );

        // 5 s travel, then the sweep on to `science` and 8 s to restore 4 HP at
        // 0.5 HP/s.
        // The virtual clock is clamped to its 0.25 s `max_delta`, so an update is
        // 0.25 s: 20 ticks of travel, then 4 HP at 0.5 HP/s is 32 more.
        for _ in 0..120 {
            app.update();
        }

        let hull = &app
            .world()
            .get::<crate::entities::spawner::EntitySystemHull>(npc)
            .expect("hull")
            .0;
        assert!(
            hull.current_for(&SystemId("science".into())).unwrap() > 0.0,
            "the dispatched team must have swept from `core` on to the destroyed \
             ownerless `science` row and restored it"
        );
        assert_eq!(
            hull.tier_for(&SystemId("science".into())),
            DamageTier::Operational,
            "and worked it back to Operational"
        );
        assert!(
            queue_station_ids(&app, npc).is_empty(),
            "only once EVERY ownerless row is Operational is the request pruned"
        );
    }

    /// AC4 ("N free teams pick N DISTINCT stations") must hold for the WHOLE
    /// visit, including after the team sweeps off the literal `core` row.
    ///
    /// A team that walks from `core` to the ownerless `science` row is still
    /// committed to the core bucket, but the config cannot say so — an ownerless
    /// row is by definition one the config does not describe. Before the fix
    /// `committed_station_for_slot` returned `None` for it, the core bucket
    /// dropped out of `excluded`, and a second team was dispatched to the group
    /// the first was already sweeping.
    #[test]
    fn a_second_team_is_not_dispatched_to_a_core_bucket_being_swept() {
        let mut app = npc_repair_app();
        // `core` damaged but cheap to finish (2 of 20 HP missing ⇒ 4 s of work
        // after 5 s of travel), so the team sweeps on to the long `science` job
        // (40 HP ⇒ 80 s) well inside the run and is still on it at the end.
        let npc = spawn_two_ownerless_rows(&mut app, 2, 18.0, 40.0);

        let mut team_zero_reached_science = false;
        for tick_idx in 0..120 {
            app.update();
            let systems = team_systems(&app, npc);
            assert_eq!(
                systems[1], None,
                "team 1 must stay idle while team 0 sweeps the core bucket \
                 (tick {tick_idx}, slots {systems:?})"
            );
            if systems[0].as_deref() == Some("science") {
                team_zero_reached_science = true;
            }
        }
        assert!(
            team_zero_reached_science,
            "fixture precondition: team 0 must actually sweep off the `core` row \
             on to `science`, or the regression is not being exercised"
        );
    }

    /// `pop_worst` / `peek` must not depend on queue insertion order when two
    /// entries tie on tier and deficit (the residual `max_by` last-wins edge).
    #[test]
    fn queue_severity_tie_breaks_on_smallest_station_id() {
        let entry = |station: &str| RepairQueueEntry {
            station_id: station.into(),
            station_label: station.into(),
            tier: DamageTier::Damaged,
            deficit: 10.0,
        };
        for order in [["alpha", "bravo"], ["bravo", "alpha"]] {
            let mut rq = RepairRequestQueue::default();
            for s in order {
                rq.push_or_merge(entry(s));
            }
            assert_eq!(rq.peek().unwrap().station_id, "alpha");
            assert_eq!(rq.pop_worst().unwrap().station_id, "alpha");
        }
    }

    /// `seed_repair_facts` exposes every reading an authored guard can name.
    #[test]
    fn seed_repair_facts_exposes_observable_damage_readings() {
        let facts = seed_repair_facts(&RepairCandidateReading {
            tier_ordinal: 2,
            deficit: 40.0,
            damage_fraction: 0.4,
            worst_system_damage_fraction: 0.6,
            system_count: 3,
            is_core: false,
            source_repair_request: true,
            assigned: false,
        });
        let near = |got: Option<f64>, want: f64| {
            assert!(
                got.is_some_and(|v| (v - want).abs() < 1e-6),
                "expected ~{want}, got {got:?}"
            );
        };
        assert_eq!(facts.get("tier_ordinal"), Some(2.0));
        assert_eq!(facts.get("deficit"), Some(40.0));
        near(facts.get("damage_fraction"), 0.4);
        near(facts.get("worst_system_damage_fraction"), 0.6);
        assert_eq!(facts.get("system_count"), Some(3.0));
        assert_eq!(facts.get("is_core"), Some(0.0));
        assert_eq!(facts.get("source_core_bucket"), Some(0.0));
        assert_eq!(facts.get("source_repair_request"), Some(1.0));
        assert_eq!(facts.get("assigned"), Some(0.0));

        let self_facts = seed_repair_self_facts(2, 0.75, Some(3.0), true);
        assert_eq!(self_facts.get("free_team_count"), Some(2.0));
        near(self_facts.get("total_hull_health_fraction"), 0.75);
        assert_eq!(self_facts.get("power_rating"), Some(3.0));
        assert_eq!(self_facts.get("red_alert"), Some(1.0));
    }

    // ── Destroyed systems are repair work now (issue #1013) ──────────────────

    /// The PRODUCTION prune, not the copy in
    /// `queue_entry_retained_when_all_systems_destroyed`: `operate_repair_ai`'s
    /// `rq.entries.retain` must keep an all-Destroyed station's request, because
    /// the on-site sweep can repair it. Before #1013 this entry was evicted on
    /// the first AI tick and the station was stranded.
    ///
    /// Retention alone is not the property that matters, so this also asserts
    /// the free team is actually DISPATCHED to the destroyed station. A queue
    /// entry the AUTHORED eligibility then refuses (the retired
    /// `candidate_fact(tier_ordinal) < 3` clause) is a queue entry that survives
    /// the prune and steers nothing — the team sits idle beside a station only
    /// it can fix. That is the assertion the sweep's own tests could not make,
    /// because they never run the selector.
    #[test]
    fn prune_retains_an_all_destroyed_station_through_the_ai_loop() {
        let mut app = npc_repair_app();
        // alpha flattened to 0 → Destroyed; bravo untouched.
        let npc = spawn_two_station_npc(
            &mut app,
            crate::ship::control_source::ControlSource::Ai,
            0.0,
            100.0,
            1,
            None,
        );
        assert_eq!(
            app.world()
                .get::<crate::entities::spawner::EntitySystemHull>(npc)
                .unwrap()
                .0
                .tier_for(&SystemId("alpha-sys".into())),
            DamageTier::Destroyed,
            "fixture precondition: alpha must be Destroyed"
        );
        assert_eq!(
            app.world().get::<RepairRequestQueue>(npc).unwrap().len(),
            1,
            "fixture precondition: the destroyed station is queued"
        );

        app.update();

        let stations: Vec<String> = app
            .world()
            .get::<RepairRequestQueue>(npc)
            .unwrap()
            .entries
            .iter()
            .map(|e| e.station_id.clone())
            .collect();
        assert_eq!(
            stations,
            vec!["alpha".to_string()],
            "an all-Destroyed station's request must survive the prune"
        );
        assert_eq!(
            team_systems(&app, npc)[0].as_deref(),
            Some("alpha-sys"),
            "the free team must actually be dispatched to the destroyed station — \
             a retained request the authored eligibility refuses would leave the \
             team idle and the station stranded exactly as before #1013"
        );
    }

    /// `push_or_merge` no longer drops a Destroyed-tier request on the floor,
    /// and a station already queued at a lighter tier takes the UPGRADE instead
    /// of the whole call bailing out.
    #[test]
    fn destroyed_tier_requests_are_queued_and_merged() {
        let mut rq = RepairRequestQueue::default();
        rq.push_or_merge(RepairQueueEntry {
            station_id: "alpha".into(),
            station_label: "Alpha".into(),
            tier: DamageTier::Destroyed,
            deficit: 100.0,
        });
        assert_eq!(rq.len(), 1, "a Destroyed request must be queued");

        let mut rq = RepairRequestQueue::default();
        rq.push_or_merge(RepairQueueEntry {
            station_id: "alpha".into(),
            station_label: "Alpha".into(),
            tier: DamageTier::Disabled,
            deficit: 40.0,
        });
        rq.push_or_merge(RepairQueueEntry {
            station_id: "alpha".into(),
            station_label: "Alpha".into(),
            tier: DamageTier::Destroyed,
            deficit: 100.0,
        });
        assert_eq!(rq.len(), 1, "same station, so still one entry");
        assert_eq!(rq.peek().unwrap().tier, DamageTier::Destroyed);
        assert_eq!(rq.peek().unwrap().deficit, 100.0);
    }

    /// End-to-end through the human dispatch path: a station whose only system
    /// is Destroyed accepts a team, which arrives, repairs it, and un-latches
    /// the tier. Before #1013 `resolve_repair_target` skipped the 0-HP system
    /// and the team bounced off it on arrival.
    #[test]
    fn destroyed_station_is_dispatched_to_and_repaired_end_to_end() {
        let mut app = test_app();
        start_game(&mut app);

        let destroyed = SystemId("helm-engine-port".into());
        {
            let mut query = app.world_mut().query_filtered::<
                &mut crate::entities::spawner::EntitySystemHull,
                With<crate::server_app::LocalShip>,
            >();
            let mut hull = query
                .single_mut(app.world_mut())
                .expect("test fixture must contain one LocalShip hull");
            hull.0.set_hp(&destroyed, 0.0);
            assert_eq!(hull.0.tier_for(&destroyed), DamageTier::Destroyed);
        }

        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: SystemId(REPAIR_SYSTEM_ID.into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: RepairTarget::Station(StationId("helm".into())),
                },
            },
        );
        tick(&mut app);

        {
            let teams = local_teams(&mut app);
            let TeamSlot::Travelling { system_id, .. } = &teams.0.slots()[0] else {
                panic!(
                    "team 0 must be sent to the Destroyed system, got {:?}",
                    teams.0.slots()[0]
                );
            };
            assert_eq!(
                system_id.as_ref(),
                Some(&destroyed),
                "the worst system on the station is the Destroyed one"
            );
        }

        // 5 s travel at 0.2 s per update, then a few ticks of repair.
        for _ in 0..30 {
            tick(&mut app);
        }

        let mut query = app.world_mut().query_filtered::<
            &crate::entities::spawner::EntitySystemHull,
            With<crate::server_app::LocalShip>,
        >();
        let hull = query
            .single(app.world())
            .expect("test fixture must contain one LocalShip hull");
        assert!(
            hull.0.current_for(&destroyed).unwrap() > 0.0,
            "the arrived team must restore HP to the Destroyed system"
        );
        assert_ne!(
            hull.0.tier_for(&destroyed),
            DamageTier::Destroyed,
            "any positive HP un-latches the Destroyed tier"
        );
    }

    /// The sweep through the real Bevy tick: `tick_repair_teams` hands its
    /// ship's own `ShipConfigComponent` to `RepairTeams::tick`, so a team that
    /// finishes one system moves to the next one its station needs without
    /// going `Returning` in between.
    #[test]
    fn tick_repair_teams_sweeps_the_station_using_the_ship_config() {
        let mut app = test_app();
        start_game(&mut app);

        // `helm-engine-port` and `helm-engine-starboard` are BOTH owned by the
        // `helm` station in the shipped battleship config
        // `ShipConfigComponent::default()` loads, so they share a sweep group.
        // The fixture REPLACES the hull with exactly those two rows: the shipped
        // battleship authors 13 `[[hull.system_hull]]` rows and none of them is
        // named `helm` (a station name is not a hull row), so nothing in this
        // fixture touches the ownerless bucket at all.
        let first = SystemId("helm-engine-port".into());
        let second = SystemId("helm-engine-starboard".into());
        {
            let local_ship = {
                let mut query = app
                    .world_mut()
                    .query_filtered::<Entity, With<crate::server_app::LocalShip>>();
                query.single(app.world()).expect("one LocalShip")
            };
            // Rebuild the hull so it carries both helm engines.
            app.world_mut().entity_mut(local_ship).insert(
                crate::entities::spawner::EntitySystemHull(SystemHull::from_config(&[
                    (first.clone(), 10.0),
                    (second.clone(), 10.0),
                ])),
            );
            let mut hull = app
                .world_mut()
                .get_mut::<crate::entities::spawner::EntitySystemHull>(local_ship)
                .unwrap();
            hull.0.set_hp(&first, 1.0);
            hull.0.set_hp(&second, 0.0);
        }

        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: SystemId(REPAIR_SYSTEM_ID.into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: RepairTarget::Station(StationId("helm".into())),
                },
            },
        );

        // Walk the whole visit, recording every system the team works on and
        // whether it ever heads home mid-way.
        let mut visited: Vec<String> = vec![];
        let mut returned = false;
        for _ in 0..400 {
            tick(&mut app);
            match &local_teams(&mut app).0.slots()[0] {
                TeamSlot::Repairing {
                    system_id: Some(s), ..
                } if visited.last() != Some(&s.0) => {
                    visited.push(s.0.clone());
                }
                TeamSlot::Returning { .. } => {
                    returned = true;
                    break;
                }
                _ => {}
            }
        }

        assert_eq!(
            visited,
            vec![
                "helm-engine-starboard".to_string(),
                "helm-engine-port".to_string()
            ],
            "the team must sweep both helm engines in one visit, worst first"
        );
        assert!(returned, "and only then head home");
    }
}
