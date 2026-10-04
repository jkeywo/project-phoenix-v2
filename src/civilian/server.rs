//! Bevy adapter for civilian routes and orders (issue #1028).
//!
//! Two components, one fixed-tick system, and no steering whatsoever. Every
//! decision lives in the pure sibling [`super::traffic`]; what happens here is
//! the translation of its answer into the vocabulary NPC hulls already fly.
//!
//! # The reuse, seam by seam (AC2)
//!
//! A civilian is an ordinary NPC hull. It carries a `[behaviour]` block like
//! every other, its doctrine is scored into its own blackboard by
//! `ai::server::aggregate_doctrine_blackboards`, the winning directive is flown
//! by `ai::core::plan_helm_travel`, and the resulting thrust and steering reach
//! the hull as ordinary admitted `SetThrust` / `SetSteering` commands. This
//! adapter's entire job is to keep **one** entry in that hull's doctrine list —
//! [`CIVILIAN_ROUTE_OBJECTIVE_ID`] — pointed at whatever the pure module says
//! the civilian is currently trying to do:
//!
//! | [`CivilianTravel`] | installed directive | flown by |
//! |---|---|---|
//! | `Route` | `AiDirective::Patrol` over the route's anchor chain | the existing patrol arm, with the existing [`PatrolCursor`] as its leg pointer |
//! | `Anchor` | `AiDirective::Reach` | the existing reach arm |
//! | `Dock` | `AiDirective::Dock` | the existing navigation-objective → waypoint hand-off, plus the existing docking close manoeuvre |
//! | `Hold` | no helm-relevant directive at all | the existing "no objective ⇒ zero throttle" arm, which brakes the hull to a stop |
//!
//! There is deliberately no per-leg mover, no dock approach controller and no
//! private waypoint write. The one thing this adapter *adds* to the shared
//! vocabulary is `AiDirective::Dock`, and it adds it as a directive rather than
//! as a mover: its destination is resolved by the same
//! `resolve_destroy_target` the `Destroy` arm uses, it becomes an anchored
//! navigation waypoint through the same `operate_navigation_ai` every hull
//! already runs, and the hull flies it through the same Channel-3 clearance.
//!
//! # Per-leg behaviour without a second cursor
//!
//! `advance_objective_cursors` already walks a `Patrol` directive's anchors and
//! keeps a [`PatrolCursor`] per objective id. That cursor **is** the civilian's
//! leg pointer: this system mirrors its index onto the state each tick and reads
//! the authored `speed` of the leg it names straight onto the doctrine entry's
//! `target_speed`. An authored `hold_secs` dwell is the same lever set to zero,
//! which is why a dwelling civilian keeps its directive (and therefore its
//! place on the route) instead of being taken off it.
//!
//! # Where orders come from
//!
//! Both surfaces converge here and neither is privileged (AGENTS.md #6):
//!
//! * a **console** order arrives as an admitted `OrderCivilian` payload on the
//!   `navigation` system, read out of the ordering ship's `AdmittedCommands`;
//! * a **script** order arrives on the `EffectQueue<PendingCivilianOrder>`
//!   resource (issue #1223), pushed by the applier with the target already
//!   resolved to a UUID.
//!
//! A malformed or unaddressable console order is answered with a
//! `ServerMessage::CivilianOrderRejected` carrying a `strings.csv` reason id, on
//! the same response token the command arrived with. A *refusal* is not that:
//! it is the civilian's own answer, arrives seconds later through the compliance
//! state, and is visible on the Navigation blackboard rather than as a bounce.

use bevy::prelude::*;

use crate::authoritative::{DeclareState, StateClass};
use crate::civilian::traffic::{
    CivilianConfig, CivilianOrder, CivilianState, CivilianTravel, ComplianceDisposition,
};
use crate::core::messages::{
    ActionCorrelationId, ActionFeedbackOutcome, AdmittedCommands, ServerMessage,
    SystemControlPayload,
};
use crate::effect_queue::EffectQueue;
use crate::entities::config::DoctrineObjective;
use crate::entities::spawner::{BehaviourSection, EntityUuid, FactionComponent};
use crate::logging::LogFilterConfig;
use crate::world::config::WorldConfig;
use crate::world::server::WorldContentRuntime;

/// The doctrine objective id a civilian's route or order always occupies.
///
/// One id, whatever the civilian is doing, for two reasons. It keeps the
/// entity's authored doctrine (a courier's own `reach-destination`, say)
/// untouched alongside it; and it makes the [`PatrolCursor`] keyed on that id
/// the civilian's leg pointer across a whole mission rather than something that
/// is thrown away and re-created every time an order lands.
///
/// [`PatrolCursor`]: crate::ai::patrol_cursor::PatrolCursor
pub const CIVILIAN_ROUTE_OBJECTIVE_ID: &str = "civilian-route";

/// The `strings.csv` id reported when an order names a civilian this world does
/// not have.
pub const REJECT_UNKNOWN_CIVILIAN: &str = "civilian.order.rejected.unknown_target";

/// The `strings.csv` id reported when an order is malformed — a divert naming
/// both a route and an anchor, or neither.
pub const REJECT_MALFORMED_ORDER: &str = "civilian.order.rejected.malformed";

/// Present when the entity's TOML declared a `[civilian]` table: the authored
/// route assignment, route priority and compliance disposition.
///
/// Authored configuration rather than live state — it never changes after
/// spawn, which is why the state that *does* change lives next door in
/// [`CivilianTraffic`] and is the only half a save has to carry.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct CivilianSection(pub CivilianConfig);

/// One civilian's live route, order and compliance state.
///
/// Authoritative per-entity simulation state: it decides where a hull is going
/// and whether the crew's order is being honoured, and two hosts that disagreed
/// about it would disagree about whether a mission is going well.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct CivilianTraffic(pub CivilianState);

/// An order queued for a civilian, already resolved to the target's UUID.
///
/// The script surface buffers these on `WorldContentRuntime`; the console
/// surface resolves straight to one in the same system that consumes it. Both
/// end up in the same list so that neither path can grow behaviour the other
/// does not have.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingCivilianOrder {
    /// The target civilian's `EntityUuid`.
    pub uuid: String,
    /// What it has been asked to do.
    pub order: CivilianOrder,
}

/// Registers the civilian traffic tick. Added by `WorldPlugin` — the routes,
/// anchors and name table it reads are its resources.
pub struct CivilianPlugin;

impl Plugin for CivilianPlugin {
    fn build(&self, app: &mut App) {
        // The scripted-order queue `tick_civilian_traffic` drains (issue #1223),
        // registered and declared at this owning site. A transient inter-system
        // queue — drained in full every tick, empty at every fold/snapshot
        // boundary — so `ClearedAtFold`.
        app.init_resource::<EffectQueue<PendingCivilianOrder>>()
            .declare_state::<EffectQueue<PendingCivilianOrder>>(
                StateClass::ClearedAtFold,
                "digest-exclusion-classes",
            );
        app.add_systems(
            FixedUpdate,
            tick_civilian_traffic
                .in_set(crate::sim_sets::FixedStep::TickCivilianTraffic)
                .in_set(crate::sim_sets::SimSet::Input),
        );
    }
}

/// Advance every civilian's compliance clock by one logical tick and install the
/// directive it implies.
///
/// `SimSet::Input`, so a console order admitted this tick is acted on this tick
/// and the doctrine entry it produces is already in place when
/// `aggregate_doctrine_blackboards` scores the pool in `SimSet::PublishAggregate`.
/// A *scripted* order is one tick later by construction — the applier that
/// resolves it runs in `SimSet::Physics` — which is the same one-tick bridge
/// every other world-event consumer rides.
///
/// UUID order, not query order: Bevy's archetype iteration order is not part of
/// the simulation's contract, and two civilians racing for the same order would
/// otherwise resolve differently on two hosts. Same rule
/// [`crate::sim_digest`] applies to its own walks.
#[allow(clippy::too_many_arguments)]
pub fn tick_civilian_traffic(
    runtime: Option<ResMut<WorldContentRuntime>>,
    // The scripted civilian-order queue, extracted off `WorldContentRuntime`
    // (issue #1223). This is its owning drain; console orders still arrive on the
    // `order_sources` query below.
    mut civilian_orders_queue: ResMut<EffectQueue<PendingCivilianOrder>>,
    world_config: Option<Res<WorldConfig>>,
    factions: Option<Res<crate::entities::config_cache::FactionRegistryResource>>,
    sim_tick: Option<Res<crate::sim_tick::SimTick>>,
    mut outbox: Option<ResMut<crate::server_app::SimOutbox>>,
    order_sources: Query<&AdmittedCommands>,
    mut civilians: Query<(
        Entity,
        &EntityUuid,
        Option<&FactionComponent>,
        &CivilianSection,
        &mut CivilianTraffic,
        &mut BehaviourSection,
        Option<&mut crate::ai::server::ObjectiveCursors>,
    )>,
    log: Option<Res<LogFilterConfig>>,
) {
    let Some(runtime) = runtime else {
        return;
    };
    let now = sim_tick.map(|t| t.0).unwrap_or(0);
    let tick_hz = world_config
        .as_deref()
        .map(|wc| wc.global.sim_tick_hz)
        .unwrap_or_else(|| crate::entities::config::GlobalConfig::default().sim_tick_hz);

    // Who is actually addressable. An order that resolves to a real entity
    // which is not traffic — a rock, a starbase, the player's own hull — is
    // rejected rather than queued, because a queue nobody drains is a silent
    // drop and the operator would be left watching for an answer that cannot
    // come.
    let addressable: std::collections::HashSet<String> = civilians
        .iter()
        .map(|(_, uuid, ..)| uuid.0.clone())
        .collect();

    // Every order addressed this tick, script and console alike, resolved to a
    // target UUID. Script orders are taken first so a scenario and a crew
    // issuing on the same tick resolve in a fixed order — and the crew wins,
    // because the later `receive_order` replaces the earlier.
    let mut queued: Vec<PendingCivilianOrder> = std::mem::take(&mut civilian_orders_queue.0);
    let mut rejections: Vec<(String, String, String, Option<ActionCorrelationId>)> = Vec::new();
    let mut accepted_feedback: Vec<(String, String, ActionCorrelationId)> = Vec::new();
    for admitted in order_sources.iter() {
        for cmd in admitted.for_target(crate::ship::system_registry::NAVIGATION_SYSTEM_ID) {
            let SystemControlPayload::OrderCivilian { target, order } = &cmd.payload else {
                continue;
            };
            let token = cmd.response_token.clone().unwrap_or_default();
            if order.validate().is_err() {
                rejections.push((
                    token,
                    target.clone(),
                    REJECT_MALFORMED_ORDER.to_string(),
                    cmd.feedback_correlation.clone(),
                ));
                continue;
            }
            match resolve_civilian(&runtime, target).filter(|uuid| addressable.contains(uuid)) {
                Some(uuid) => {
                    queued.push(PendingCivilianOrder {
                        uuid: uuid.clone(),
                        order: order.clone(),
                    });
                    if let Some(correlation) = cmd
                        .feedback_correlation
                        .clone()
                        .filter(|_| !token.is_empty())
                    {
                        accepted_feedback.push((uuid, token, correlation));
                    }
                }
                None => rejections.push((
                    token,
                    target.clone(),
                    REJECT_UNKNOWN_CIVILIAN.to_string(),
                    cmd.feedback_correlation.clone(),
                )),
            }
        }
    }
    // A scripted order to something that is not traffic gets the same warning
    // and the same drop, minus the bounce: a script has no console to flash.
    for pending in queued.iter().filter(|p| !addressable.contains(&p.uuid)) {
        crate::pwarn!(
            log,
            crate::logging::LogCat::Nav,
            "civilian order for uuid '{}' names no craft carrying traffic state — ignoring",
            pending.uuid
        );
    }
    for (token, target, reason, correlation) in rejections {
        crate::pwarn!(
            log,
            crate::logging::LogCat::Nav,
            "civilian order for '{target}' rejected: {reason}"
        );
        if token.is_empty() {
            continue;
        }
        if let Some(outbox) = outbox.as_deref_mut() {
            outbox.push_reliable((
                crate::lobby::Target::Token(token.clone()),
                ServerMessage::CivilianOrderRejected { target, reason },
            ));
            if let Some(correlation) = correlation {
                let feedback = crate::command_admission::feedback::action_feedback(
                    &token,
                    &correlation,
                    ActionFeedbackOutcome::Refused,
                );
                outbox.push_reliable((feedback.target, feedback.msg));
            }
        }
    }

    let mut rows: Vec<(String, Entity)> = civilians
        .iter()
        .map(|(entity, uuid, ..)| (uuid.0.clone(), entity))
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.index().cmp(&b.1.index())));

    for (uuid, entity) in rows {
        let Ok((_, _, faction, section, mut traffic, mut behaviour, cursors)) =
            civilians.get_mut(entity)
        else {
            continue;
        };
        let disposition = resolve_disposition(&section.0, faction, factions.as_deref());

        // 1. Take whatever this civilian was told.
        for pending in queued.iter().filter(|q| q.uuid == uuid) {
            if let Some(t) =
                traffic
                    .0
                    .receive_order(pending.order.clone(), &disposition, now, tick_hz)
            {
                crate::pdebug!(
                    log,
                    crate::logging::LogCat::Nav,
                    entity = entity,
                    "civilian order {:?}: {} -> {}",
                    pending.order,
                    t.from.as_str(),
                    t.to.as_str()
                );
            }
        }
        // A console order is Applied when this owning consumer has delivered
        // it into the civilian's compliance machine. The craft may still
        // refuse on its authored clock later; that is authoritative traffic
        // state, not a transport-level command refusal.
        if let Some(outbox) = outbox.as_deref_mut() {
            for (_, token, correlation) in accepted_feedback.iter().filter(|(id, ..)| id == &uuid) {
                let feedback = crate::command_admission::feedback::action_feedback(
                    token,
                    correlation,
                    ActionFeedbackOutcome::Applied,
                );
                outbox.push_reliable((feedback.target, feedback.msg));
            }
        }

        // 2. Advance the compliance clock against a live answer to "can this
        //    still be carried out?".
        let resolves = destination_resolves(traffic.0.order(), &runtime, world_config.as_deref());
        if let Some(t) = traffic.0.advance(now, resolves, &disposition, tick_hz) {
            crate::pdebug!(
                log,
                crate::logging::LogCat::Nav,
                entity = entity,
                "civilian compliance: {} -> {}{}",
                t.from.as_str(),
                t.to.as_str(),
                t.reason
                    .as_ref()
                    .map(|r| format!(" ({r})"))
                    .unwrap_or_default()
            );
        }

        // 3. The cursor is the leg pointer; mirror it and let an authored dwell
        //    start when the civilian leaves a leg that asked for one.
        let route = traffic
            .0
            .route()
            .and_then(|id| world_config.as_deref().and_then(|wc| wc.route(id)));
        if let Some(index) = cursors.as_deref().and_then(|c| {
            c.0.iter()
                .find(|cursor| cursor.objective_id == CIVILIAN_ROUTE_OBJECTIVE_ID)
                .map(|cursor| cursor.index())
        }) {
            traffic.0.observe_leg(index, route, now, tick_hz);
        }

        // 4. Install what it is trying to do, as an ordinary doctrine objective.
        let travel = traffic.0.travel();
        let speed = traffic.0.cruise_speed(route, now);
        let desired = directive_for(&travel, &section.0, route, speed);
        if install_directive(&mut behaviour.0.doctrine, desired) {
            // The chain changed under the cursor, so the old index means nothing
            // — drop it and let `advance_objective_cursors` mint a fresh one.
            if let Some(mut cursors) = cursors {
                cursors
                    .0
                    .retain(|c| c.objective_id != CIVILIAN_ROUTE_OBJECTIVE_ID);
            }
        }
    }
}

/// Resolve an order's target: an authored world entity name, or a UUID as-is.
///
/// Both, because a console tapping a contact on the nav map holds a UUID while a
/// scenario writing an order in a script holds the name it authored.
fn resolve_civilian(runtime: &WorldContentRuntime, target: &str) -> Option<String> {
    runtime.name_to_uuid.get(target).cloned().or_else(|| {
        uuid::Uuid::parse_str(target)
            .ok()
            .map(|_| target.to_string())
    })
}

/// This civilian's disposition: its own authored table, else its faction's, else
/// the cooperative default.
///
/// Two levels rather than one because a scenario tunes traffic in both shapes —
/// "the Kestrel Combine never diverts" is a faction fact, "this one hauler is
/// having a bad day" is an entity fact — and neither is a threshold the code
/// invented (AGENTS.md #11).
fn resolve_disposition(
    config: &CivilianConfig,
    faction: Option<&FactionComponent>,
    registry: Option<&crate::entities::config_cache::FactionRegistryResource>,
) -> ComplianceDisposition {
    if let Some(own) = config.compliance.as_ref() {
        return own.clone();
    }
    faction
        .zip(registry)
        .and_then(|(f, r)| r.0.get(&f.0))
        .and_then(|f| f.compliance.clone())
        .unwrap_or_default()
}

/// Whether the standing order can still be carried out.
///
/// The live half of the compliance machine's `destination_resolves` input: a
/// dock target this world does not have, a diverted-to route nobody declared, or
/// an anchor that resolves nowhere all mean the civilian agreed to something it
/// cannot do — which is [`ComplianceState::NonCompliant`], not a silent stall.
///
/// [`ComplianceState::NonCompliant`]: crate::civilian::ComplianceState::NonCompliant
fn destination_resolves(
    order: Option<&CivilianOrder>,
    runtime: &WorldContentRuntime,
    world_config: Option<&WorldConfig>,
) -> bool {
    match order {
        None | Some(CivilianOrder::Hold) => true,
        Some(CivilianOrder::Dock { structure }) => runtime.name_to_uuid.contains_key(structure),
        Some(CivilianOrder::Divert {
            route: Some(id), ..
        }) => world_config.and_then(|wc| wc.route(id)).is_some_and(|r| {
            r.legs
                .iter()
                .all(|l| wc_has_anchor(world_config, &l.anchor))
        }),
        Some(CivilianOrder::Divert {
            anchor: Some(name), ..
        }) => wc_has_anchor(world_config, name),
        // A divert naming neither is refused at both order surfaces, so it can
        // only reach here through a hand-built state; treat it as unresolvable
        // rather than as quietly fine.
        Some(CivilianOrder::Divert { .. }) => false,
    }
}

/// Whether the world declares this anchor.
fn wc_has_anchor(world_config: Option<&WorldConfig>, anchor: &str) -> bool {
    world_config.is_some_and(|wc| wc.anchors.contains_key(anchor))
}

/// The doctrine entry a civilian's current travel intent implies, or `None` when
/// it is holding station and should have no helm-relevant directive at all.
fn directive_for(
    travel: &CivilianTravel,
    config: &CivilianConfig,
    route: Option<&crate::civilian::traffic::RouteConfig>,
    speed: f32,
) -> Option<DoctrineObjective> {
    let base = DoctrineObjective {
        id: CIVILIAN_ROUTE_OBJECTIVE_ID.to_string(),
        base_priority: config.route_priority,
        target_speed: speed,
        // A civilian under way runs its drive: this is ambient traffic crossing
        // a system, not a warship holding a firing position.
        use_impulse: Some(config.use_impulse.unwrap_or(true)),
        ..DoctrineObjective::default()
    };
    match travel {
        CivilianTravel::Hold => None,
        CivilianTravel::Route { .. } => {
            let route = route?;
            Some(DoctrineObjective {
                directive_kind: Some("Patrol".to_string()),
                directive_anchors: route.anchor_chain(),
                directive_loop: route.loops(),
                ..base
            })
        }
        CivilianTravel::Anchor { name } => Some(DoctrineObjective {
            directive_kind: Some("Reach".to_string()),
            directive_anchor: Some(name.clone()),
            ..base
        }),
        CivilianTravel::Dock { structure } => Some(DoctrineObjective {
            directive_kind: Some("Dock".to_string()),
            directive_dock_target: Some(structure.clone()),
            ..base
        }),
    }
}

/// Keep the civilian's one doctrine entry in step with `desired`.
///
/// Returns whether the entry's *destination* changed, which is the only thing
/// that invalidates the route cursor — a leg's speed moving as the cursor
/// advances must not reset the very cursor that moved it.
///
/// A hold counts as a change, so a craft released from one rejoins its lane at
/// the first leg rather than where it stopped. That is the honest reading of
/// "no directive at all": the cursor belongs to an objective, and while the
/// craft has none there is nothing for it to be a pointer into. Preserving it
/// would need the adapter to remember which lane the dropped cursor belonged
/// to, which is a second copy of the thing the doctrine entry already says.
fn install_directive(
    doctrine: &mut Vec<DoctrineObjective>,
    desired: Option<DoctrineObjective>,
) -> bool {
    let existing = doctrine
        .iter()
        .position(|d| d.id == CIVILIAN_ROUTE_OBJECTIVE_ID);
    match (existing, desired) {
        (None, None) => false,
        (Some(i), None) => {
            doctrine.remove(i);
            true
        }
        (None, Some(entry)) => {
            doctrine.push(entry);
            true
        }
        (Some(i), Some(entry)) => {
            let moved = !same_destination(&doctrine[i], &entry);
            doctrine[i] = entry;
            moved
        }
    }
}

/// Whether two doctrine entries name the same place to go.
fn same_destination(a: &DoctrineObjective, b: &DoctrineObjective) -> bool {
    a.directive_kind == b.directive_kind
        && a.directive_anchors == b.directive_anchors
        && a.directive_loop == b.directive_loop
        && a.directive_anchor == b.directive_anchor
        && a.directive_dock_target == b.directive_dock_target
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
