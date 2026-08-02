use bevy::prelude::*;

use crate::command_admission::ai_emit::emit_ai_command;
use crate::core::broadcast::{Audience, Cadence, SimBroadcaster};
use crate::damage::DamageTier;
use crate::messages::ModifierSlot;
use crate::messages::{
    QueueEntryPreview, RepairBlackboard, ServerMessage, SystemBlackboard, SystemHullStatus,
    SystemId, TeamSlot,
};
use crate::modifiers::ShipModifiers;
use crate::repair_teams::RepairTeams;
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
    pub fn push_or_merge(&mut self, entry: RepairQueueEntry) {
        if entry.tier == DamageTier::Destroyed {
            return;
        }
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

    pub fn remove_station(&mut self, station_id: &str) {
        self.entries.retain(|e| e.station_id != station_id);
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
        // The dispatch router registers itself in Physics, pinning its own
        // `.after(operate_repair_ai)` ordering (issue #830). See `super::dispatch`.
        super::dispatch::register_repair_dispatch(app);
        app.add_systems(
            FixedUpdate,
            (
                // AC4 DETERMINISM (issue #785) — pin the remaining intra-Physics
                // edge. `operate_repair_ai` (decide/emit) →
                // `handle_dispatch_repair_team` + `handle_set_repair_priority`
                // (apply) → `tick_repair_teams` (advance) must run in that order
                // every tick. #830 pinned only the first edge, so
                // `tick_repair_teams` stayed ambiguous against the applier even
                // though both mutate `ShipRepairTeams`: Bevy's parallel executor
                // then serialised them run-varyingly, and a dispatch landing
                // BEFORE the tick let a `Returning { remaining }` slot hit
                // `remaining <= 0` and flip straight to `Travelling` instead of
                // staying `Returning`. That — not HashMap order — was the root
                // cause of `all_busy_teams_ignore_further_dispatches` flaking.
                // Production was weaker than its own `npc_repair_app` fixture,
                // which already `.chain()`s the quartet; this closes the gap.
                tick_repair_teams
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(super::dispatch::handle_dispatch_repair_team)
                    .after(super::dispatch::handle_set_repair_priority),
                operate_repair_ai.in_set(crate::sim_sets::SimSet::Physics),
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
pub fn tick_repair_teams(
    time: Res<Time>,
    mut ship_q: Query<
        (
            Option<&mut ShipRepairTeams>,
            &ShipModifiers,
            &mut crate::entity_spawner::EntitySystemHull,
            Option<&crate::entity_spawner::EntityUuid>,
        ),
        With<crate::server_app::Ship>,
    >,
    // Balance telemetry. `Option<ResMut<Messages<_>>>` so bare-`App` fixtures
    // that never registered the message still pass parameter validation.
    mut balance_events: Option<ResMut<bevy::ecs::message::Messages<crate::balance::BalanceEvent>>>,
) {
    let dt = time.delta_secs();

    for (teams_comp, modifiers, mut hull, ship_uuid) in ship_q.iter_mut() {
        let Some(mut teams) = teams_comp else {
            continue;
        };
        let repair_mult = modifiers.get(&ModifierSlot::RepairRate);
        // Capture total hull current before/after the tick so the restored HP
        // can be reported as a per-ship `RepairApplied` delta (issue #841).
        // This is the only path that actually ticks a ship's teams — the global
        // `ShipRepairTeams` resource is publish-only, never ticked.
        let before = hull.0.total_current();
        teams.0.tick(dt * repair_mult, &mut hull.0);
        let restored = hull.0.total_current() - before;
        if restored > 0.0 {
            if let (Some(msgs), Some(uuid)) = (balance_events.as_mut(), ship_uuid) {
                msgs.write(crate::balance::BalanceEvent::RepairApplied {
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
            Option<&crate::entity_spawner::EntitySystemHull>,
            Option<&RepairRequestQueue>,
            &mut crate::server_app::ShipSystemBlackboards,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    for (teams_opt, hull_opt, repair_queue_ref, mut blackboards) in ship_q.iter_mut() {
        let default_teams;
        let teams: &ShipRepairTeams = match teams_opt {
            Some(t) => t,
            None => {
                default_teams = ShipRepairTeams(crate::repair_teams::RepairTeams::default());
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
        let bb = RepairBlackboard {
            teams: team_slots,
            travel_duration_secs: teams.0.timings().travel_duration,
            system_hull,
            damageable_systems,
            // Host-internal copy: unprojected. `system_hull` and `queue_depth` both
            // carry exact per-system detail and are filtered on the wire by
            // `visibility::project_repair_blackboard`, which also fills in the
            // aggregate (issue #737). The repair AI controller reads this copy and
            // needs every system.
            queue_depth,
            aggregate_hull_fraction: None,
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
    hull: &crate::damage::SystemHull,
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

/// The candidate key standing for [`crate::messages::RepairTarget::Core`] — the
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
    let mut facts = crate::world::flags::AiFacts::new();
    facts.set("tier_ordinal", reading.tier_ordinal as f64);
    facts.set("deficit", reading.deficit as f64);
    facts.set("damage_fraction", reading.damage_fraction as f64);
    facts.set(
        "worst_system_damage_fraction",
        reading.worst_system_damage_fraction as f64,
    );
    facts.set("system_count", reading.system_count as f64);
    facts.set("is_core", if reading.is_core { 1.0 } else { 0.0 });
    facts.set(
        "source_repair_request",
        if reading.source_repair_request {
            1.0
        } else {
            0.0
        },
    );
    facts.set(
        "source_core_bucket",
        if reading.is_core { 1.0 } else { 0.0 },
    );
    facts.set("assigned", if reading.assigned { 1.0 } else { 0.0 });
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
    let mut facts = crate::world::flags::AiFacts::new();
    facts.set("free_team_count", free_team_count as f64);
    facts.set(
        "total_hull_health_fraction",
        total_hull_health_fraction as f64,
    );
    facts.set("red_alert", if red_alert { 1.0 } else { 0.0 });
    if let Some(pr) = power_rating {
        facts.set("power_rating", pr as f64);
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
fn committed_station_for_slot(
    slot: &TeamSlot,
    config: &crate::ship_plugin::ShipConfigComponent,
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
    config
        .0
        .system(system_id)
        .and_then(|sc| sc.station.as_ref())
        .map(|s| s.0.clone())
}

/// Aggregate a station's observable hull damage: `(damage_fraction,
/// worst_system_damage_fraction, system_count)`.
fn station_damage_readings(
    station_id: &str,
    hull: &crate::damage::SystemHull,
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
///   1. `RepairRequestQueue.entries`, pruned below the moment a station has no
///      non-Operational / non-Destroyed system — so a completed repair's target
///      disappears (AC4).
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
    // Read-only scenario flag/counter chain (issue #891 stage 2). `Option` so
    // bare-`App` fixtures still pass parameter validation.
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    layers: Option<Res<crate::world::server::WorldLayerMap>>,
    // The per-ship origin-layer stamp (issue #891 review finding 1): an O(1)
    // read replacing the old `WorldLayerMap` scan inside `entity_flag_chain`.
    origin_q: Query<&crate::world::server::EntityOriginLayer>,
    mut ships: Query<
        (
            Entity,
            Option<&crate::entity_spawner::EntityUuid>,
            &ShipSystemControlSources,
            Option<&ShipRepairTeams>,
            Option<&crate::entity_spawner::EntitySystemHull>,
            Option<&mut RepairRequestQueue>,
            Option<&crate::ship_plugin::ShipConfigComponent>,
            Option<&RepairTargetSelector>,
            Option<&crate::ship_state::ShipRedAlert>,
            &mut crate::messages::AdmittedCommands,
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
        mut admitted,
    ) in ships.iter_mut()
    {
        let policy = sources.0.policy_for(&repair_system_id());
        if !policy.operate_ai {
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

        rq.entries.retain(|entry| {
            // The `core` bucket owns NO station in `ShipConfig` — validation
            // actively forbids a station with that id, and `damage_sync` files
            // ownerless systems under it — so the station-owned scan below would
            // find zero systems and prune every core request. Prune it against
            // its own hull entry instead, the same repairable-tier test.
            if entry.station_id == REPAIR_CORE_BUCKET_KEY {
                let t = hull
                    .0
                    .tier_for(&SystemId(REPAIR_CORE_BUCKET_KEY.to_string()));
                return t != DamageTier::Operational && t != DamageTier::Destroyed;
            }
            config
                .0
                .systems
                .iter()
                .filter(|s| {
                    s.station.as_ref().map(|st| st.0.as_str()) == Some(entry.station_id.as_str())
                })
                .any(|s| {
                    let t = hull.0.tier_for(&s.id);
                    t != DamageTier::Operational && t != DamageTier::Destroyed
                })
        });

        // Free team indices, ASCENDING — the deterministic visit order (AC4).
        // Emission does not mutate `teams` this tick (the applier does, later in
        // Physics), so `lowest_free_team()` would return the same idx every
        // time; we draw from a locally-consumed list instead.
        let free_teams: Vec<usize> = teams
            .0
            .slots()
            .iter()
            .enumerate()
            .filter_map(|(i, s)| matches!(s, TeamSlot::Idle).then_some(i))
            .collect();
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
            .filter_map(|slot| committed_station_for_slot(slot, config))
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
        let flag_chain = crate::world::server::entity_flag_chain(
            origin_q.get(ship_entity).ok(),
            runtime.as_deref(),
            layers.as_deref(),
        );

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
                .and_then(|slot| committed_station_for_slot(slot, config));

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
                crate::messages::RepairTarget::Core
            } else {
                crate::messages::RepairTarget::Station(crate::messages::StationId(winner.clone()))
            };
            emit_repair_ai_command(
                entity_uuid,
                crate::messages::SystemControlPayload::DispatchRepairTeam {
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
    entity_uuid: Option<&crate::entity_spawner::EntityUuid>,
    payload: crate::messages::SystemControlPayload,
    sources: &ShipSystemControlSources,
    sessions: &crate::lobby::Sessions,
    config: &crate::ship_plugin::ShipConfigComponent,
    admitted: &mut crate::messages::AdmittedCommands,
) -> bool {
    emit_ai_command(
        entity_uuid,
        crate::system_registry::repair_system_id(),
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
    use crate::damage::SystemHull;
    use crate::lobby::{InboundMessage, LobbyPlugin, OutboundMessage};
    use crate::messages::*;
    use crate::shield::ShieldSystem;
    use crate::ship_plugin::ShipSystemControlSources;
    use crate::simulation::SimOutbox;
    use crate::simulation::{ShipImpulse, ShipShields};

    #[derive(Resource, Default)]
    struct Outbox(Vec<OutboundMessage>);

    fn collect(mut reader: MessageReader<OutboundMessage>, mut box_: ResMut<Outbox>) {
        for m in reader.read() {
            box_.0.push(m.clone());
        }
    }

    fn test_app() -> App {
        let mut app = App::new();
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
            crate::simulation::Ship,
            crate::simulation::LocalShip,
            crate::ship_plugin::ShipConfigComponent::default(),
            crate::ship_plugin::ShipSystemControlSources::default(),
            crate::messages::AdmittedCommands::default(),
            crate::ship_plugin::ActiveStationRatings::default(),
            crate::ship_plugin::CoordinationQueue::default(),
            crate::entity_spawner::EntitySystemHull(SystemHull::from_config(hull_config)),
            crate::server_app::ShipSystemBlackboards::default(),
            ShipShields(ShieldSystem::default(), 0.5),
            ShipImpulse(crate::impulse::ImpulseState::new()),
            crate::modifiers::ShipModifiers::new(),
            RepairRequestQueue::default(),
            // Nested tuple to keep the outer bundle within Bevy's 15-arity limit.
            // Issue #830: the global `ShipRepairTeams` Resource is gone; every
            // ship (including this test's LocalShip) carries its own component.
            (
                crate::ship_plugin::RepairHumanAlerted::default(),
                crate::ship_plugin::LastSystemTiers::default(),
                ShipRepairTeams(crate::repair_teams::RepairTeams::new(2)),
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
            .query_filtered::<&ShipRepairTeams, With<crate::simulation::LocalShip>>();
        q.single(app.world())
            .expect("LocalShip must carry ShipRepairTeams")
            .clone()
    }

    /// Dispatch a team on the LocalShip's own `ShipRepairTeams` component.
    fn dispatch_local(app: &mut App, idx: usize, sid: SystemId, name: &str) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipRepairTeams, With<crate::simulation::LocalShip>>();
        q.single_mut(app.world_mut())
            .expect("LocalShip must carry ShipRepairTeams")
            .0
            .dispatch(idx, sid, name.to_string());
    }

    fn repair_bb(app: &mut App) -> RepairBlackboard {
        let mut q = app
            .world_mut()
            .query_filtered::<&crate::server_app::ShipSystemBlackboards, With<crate::simulation::LocalShip>>();
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
                delivery: crate::messages::DeliveryClass::Reliable,
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
            crate::messages::TeamSlot::Travelling { .. }
        )
    }

    fn team_is_idle(teams: &ShipRepairTeams, idx: usize) -> bool {
        matches!(teams.0.slots()[idx], crate::messages::TeamSlot::Idle)
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
                target: crate::messages::SystemId("repair".into()),
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

        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("repair".into()),
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
                .query_filtered::<Entity, With<crate::simulation::LocalShip>>();
            query
                .single(app.world())
                .expect("test fixture must contain one LocalShip")
        };
        app.world_mut()
            .entity_mut(local_ship)
            .insert(ShipRepairTeams(crate::repair_teams::RepairTeams::default()));

        let damaged_system = SystemId("helm-engine-port".into());
        let hp_before = 10.0;
        {
            let mut query = app.world_mut().query_filtered::<
                &mut crate::entity_spawner::EntitySystemHull,
                With<crate::simulation::LocalShip>,
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
            &crate::entity_spawner::EntitySystemHull,
            With<crate::simulation::LocalShip>,
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

        // Dispatch both teams (default is 2).
        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("repair".into()),
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
                target: crate::messages::SystemId("repair".into()),
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
                target: crate::messages::SystemId("repair".into()),
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
            crate::messages::TeamSlot::Returning { .. }
        ));
        assert!(team_is_travelling(&teams, 1));
    }

    /// RepairState broadcast includes the team slot states.
    #[test]
    fn repair_state_broadcast_includes_team_slots() {
        let mut app = test_app();
        start_game(&mut app);

        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("repair".into()),
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
                teams.iter().any(|t| matches!(t, crate::messages::TeamSlot::Travelling { .. })))
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

        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("repair".into()),
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
                target: crate::messages::SystemId("repair".into()),
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
                target: crate::messages::SystemId("repair".into()),
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
                target: crate::messages::SystemId("repair".into()),
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
        let config =
            crate::entity_includes::load_entity_config("assets/entities/alliance_battleship.toml")
                .expect("alliance_battleship.toml must compose and parse");
        let rc = config
            .repair
            .expect("alliance_battleship must declare [repair]");
        let timings = rc.to_runtime();
        let teams = crate::repair_teams::RepairTeams::new_with_timings(2, timings);
        assert_eq!(teams.timings().travel_duration, rc.travel_duration_secs);
        assert_eq!(
            teams.timings().repair_rate_hp_per_sec,
            rc.repair_rate_hp_per_sec
        );
        // And the runtime defaults still match (until someone intentionally
        // diverges them).
        let baseline = crate::repair_teams::RepairTimings::default();
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
    /// must be evicted by the retain predicate (zombie-entry regression).
    #[test]
    fn queue_entry_evicted_when_all_systems_destroyed() {
        use crate::damage::SystemHull;
        use crate::ship::config::{ShipConfig, SystemInstanceConfig};

        let station_id = "helm";
        let system_id = SystemId("helm".into());

        let config = ShipConfig {
            stations: vec![],
            systems: vec![SystemInstanceConfig {
                id: system_id.clone(),
                kind: "helm".into(),
                station: Some(StationId(station_id.into())),
                ai_only: false,
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
            tier: crate::damage::DamageTier::Disabled,
            deficit: 25.0,
        });
        assert_eq!(rq.entries.len(), 1, "entry must be present before retain");

        hull.set_hp(&system_id, 0.0);
        assert_eq!(
            hull.tier_for(&system_id),
            crate::damage::DamageTier::Destroyed,
            "system must be Destroyed after set_hp(0)"
        );

        rq.entries.retain(|entry| {
            config
                .systems
                .iter()
                .filter(|s| {
                    s.station.as_ref().map(|st| st.0.as_str()) == Some(entry.station_id.as_str())
                })
                .any(|s| {
                    let t = hull.tier_for(&s.id);
                    t != crate::damage::DamageTier::Operational
                        && t != crate::damage::DamageTier::Destroyed
                })
        });

        assert!(
            rq.entries.is_empty(),
            "queue entry must be evicted when all station systems are Destroyed"
        );
    }

    /// Verifies that operate_repair_ai loops over all entities with
    /// ShipSystemControlSources, gating on operate_ai (issue #590 AC).
    #[test]
    fn operate_repair_ai_runs_per_entity_for_ai_controlled_ships() {
        use crate::ship::control_source::{ControlSource, ControlSourceResolver};

        let mut ai_resolver = ControlSourceResolver::new();
        ai_resolver.set(
            crate::system_registry::repair_system_id(),
            ControlSource::Ai,
        );
        let ai_sources = ShipSystemControlSources(ai_resolver);
        let policy = ai_sources
            .0
            .policy_for(&crate::system_registry::repair_system_id());
        assert!(policy.operate_ai, "AI Repair must gate through operate_ai");

        let mut human_resolver = ControlSourceResolver::new();
        human_resolver.set(
            crate::system_registry::repair_system_id(),
            ControlSource::Human,
        );
        let human_sources = ShipSystemControlSources(human_resolver);
        let human_policy = human_sources
            .0
            .policy_for(&crate::system_registry::repair_system_id());
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
    fn clear_admitted_commands(mut q: Query<&mut crate::messages::AdmittedCommands>) {
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
            crate::damage::SystemHull::from_config(&[(SystemId("helm".into()), 100.0_f32)]);
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
                crate::entity_spawner::EntityUuid("npc-repair-1".into()),
                ShipSystemControlSources(resolver),
                ShipRepairTeams(crate::repair_teams::RepairTeams::new(2)),
                crate::entity_spawner::EntitySystemHull(hull),
                crate::modifiers::ShipModifiers::new(),
                queue,
                npc_repair_config(),
                crate::messages::AdmittedCommands::default(),
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
            .get::<crate::entity_spawner::EntitySystemHull>(npc)
            .unwrap()
            .0
            .total_current();

        // 200 iterations comfortably covers the 5 s travel + repair time.
        for _ in 0..200 {
            app.update();
        }

        let hp_after = app
            .world()
            .get::<crate::entity_spawner::EntitySystemHull>(npc)
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
        let mut admitted = crate::messages::AdmittedCommands::default();

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
        // Use the hull with helm at reduced HP so the team doesn't immediately
        // leave when it arrives.
        {
            let mut q = app.world_mut().query_filtered::<
                &mut crate::entity_spawner::EntitySystemHull,
                With<crate::simulation::LocalShip>,
            >();
            if let Ok(mut hull) = q.single_mut(app.world_mut()) {
                hull.0
                    .set_hp(&crate::messages::SystemId("helm".into()), 10.0);
            }
        }

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

        let mut hull = crate::damage::SystemHull::from_config(&[
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
            let tier = hull.tier_for(&SystemId(sid.into()));
            if tier == DamageTier::Operational || tier == DamageTier::Destroyed {
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
                crate::entity_spawner::EntityUuid("npc-repair-2".into()),
                ShipSystemControlSources(resolver),
                ShipRepairTeams(crate::repair_teams::RepairTeams::new(teams)),
                crate::entity_spawner::EntitySystemHull(hull),
                crate::modifiers::ShipModifiers::new(),
                queue,
                two_station_config(),
                crate::messages::AdmittedCommands::default(),
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
            .get_mut::<crate::entity_spawner::EntitySystemHull>(npc)
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
            crate::repair_teams::RepairTeams::new(1);
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
            .get::<crate::entity_spawner::EntitySystemHull>(npc)
            .unwrap()
            .0
            .current_for(&SystemId("bravo-sys".into()))
            .unwrap();
        for _ in 0..200 {
            app.update();
        }
        let after = app
            .world()
            .get::<crate::entity_spawner::EntitySystemHull>(npc)
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
        let mut hull = crate::damage::SystemHull::from_config(&[
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
                crate::entity_spawner::EntityUuid("npc-repair-core".into()),
                ShipSystemControlSources(resolver),
                ShipRepairTeams(crate::repair_teams::RepairTeams::new(1)),
                crate::entity_spawner::EntitySystemHull(hull),
                crate::modifiers::ShipModifiers::new(),
                queue,
                config,
                crate::messages::AdmittedCommands::default(),
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
}
