use bevy::prelude::*;

use crate::command_admission::ai_emit::emit_ai_command;
use crate::core::messages::{
    ActionFeedbackOutcome, AdmittedCommand, AdmittedCommands, CameraView, CaptainBlackboard,
    DeliveryClass, ObjectiveSnapshot, ServerMessage, SystemBlackboard, SystemControlPayload,
    SystemId, ViewMode,
};
use crate::objectives::WorldConditions;
use crate::ship::combat_activity::RecentCombatActivity;
use crate::ship::control_source::ControlSource;
use crate::ship_plugin::ShipSystemControlSources;
use crate::world::server::ObjectiveManagerRes;

/// Per-ship inline stateless Captain AI policy (issue #775).
///
/// Attached at spawn from the ship's authored `[captain_console.ai]` block.
/// Read by [`operate_captain_ai`], which evaluates it over an immutable per-tick
/// fact snapshot to decide the `red_alert` output channel — replacing the
/// retired hardcoded `CaptainAi` combat-window controller.
///
/// Since #885b stage 5d there is no Rust-side synthesised default behind it: a
/// ship without the component takes no Red Alert decisions at all.
#[derive(Component, Clone, Debug, Default)]
pub struct CaptainAiPolicy(pub crate::ai::policy::AiPolicy);

pub struct CaptainPlugin;

impl Plugin for CaptainPlugin {
    fn build(&self, app: &mut App) {
        use crate::command_admission::{ConsumerMatcher, RegisterAdmittedConsumer};
        // Admitted-command consumers (issue #833): `handle_set_red_alert`
        // (red-alert), `handle_set_objective_priority` (captain), and
        // `handle_set_view` (viewscreen SetView).
        app.register_admitted_consumer(ConsumerMatcher::exact(
            crate::ship::system_registry::RED_ALERT_KIND,
            crate::ship::system_registry::RED_ALERT_SYSTEM_ID,
        ))
        .register_admitted_consumer(ConsumerMatcher::exact(
            crate::ship::system_registry::CAPTAIN_KIND,
            crate::ship::system_registry::CAPTAIN_SYSTEM_ID,
        ))
        .register_admitted_consumer(ConsumerMatcher::kind(
            crate::ship::system_registry::VIEWSCREEN_KIND,
        ));
        app.init_resource::<crate::server_app::CaptainPriorityBoost>();
        // The ONE shared AI decision cadence (issues #889, #895).
        crate::ai::cadence::register_ai_cadence(app);
        app.add_systems(
            FixedUpdate,
            (
                // Gated by `run_if`, not by an `Option<Res<_>>` check inside the
                // body (issue #889). The in-body form fell back to evaluating
                // EVERY tick whenever the resource was absent — which is every
                // bare-`App` fixture in the crate — so the shipped cadence was
                // not exercised by a single unit test. The rate is unchanged:
                // the derived slower snapshot cadence, `[global] ai_snapshot_hz`
                // base ticks apart.
                operate_captain_ai
                    .in_set(crate::sim_sets::SimSet::Input)
                    .before(handle_set_red_alert)
                    .run_if(crate::ai::cadence::ai_snapshot_ready),
                backfill_captain_prefers_cinematic_view
                    .in_set(crate::sim_sets::SimSet::Input)
                    .before(handle_set_view)
                    .run_if(crate::ai::cadence::ai_snapshot_ready),
                // Carries `RedAlertApplied`: every `Input` reader that decides
                // authoritative state from the alert level orders itself after
                // this label rather than against this system by name, so the
                // "current alert level" contract survives an unrelated system
                // joining the set (issue #1346's determinism regression).
                handle_set_red_alert
                    .in_set(crate::sim_sets::SimSet::Input)
                    .in_set(crate::sim_sets::RedAlertApplied),
                handle_set_view.in_set(crate::sim_sets::SimSet::Input),
                handle_set_objective_priority.in_set(crate::sim_sets::SimSet::Input),
                crate::ship::combat_activity::update_combat_activity
                    .in_set(crate::sim_sets::SimSet::Broadcast),
                publish_captain_blackboard.in_set(crate::sim_sets::SimSet::Publish),
            ),
        );
    }
}

// ── Input handlers ───────────────────────────────────────────────────────────

fn finish_action_feedback(
    cmd: &AdmittedCommand,
    outbound: &mut Option<
        ResMut<bevy::ecs::message::Messages<crate::lobby::server::OutboundMessage>>,
    >,
    outcome: ActionFeedbackOutcome,
) {
    let (Some(correlation), Some(token), Some(messages)) = (
        cmd.feedback_correlation.as_ref(),
        cmd.response_token.as_ref(),
        outbound.as_deref_mut(),
    ) else {
        return;
    };
    messages.write(crate::lobby::server::OutboundMessage {
        target: crate::lobby::Target::Token(token.clone()),
        msg: ServerMessage::ActionFeedback {
            correlation: correlation.clone(),
            outcome,
        },
        delivery: DeliveryClass::Reliable,
    });
}

/// Applies `SetRedAlert { active }` commands from every ship's own
/// `AdmittedCommands` to that ship's own `ShipRedAlert` (issue #748).
///
/// The command carries the desired end state, so the handler **assigns**
/// `ra.0 = active` rather than inverting. Retried, duplicated, or stale-UI
/// commands are therefore idempotent: setting `active: true` twice leaves the
/// ship at true; a stale `active: false` when already false is a no-op.
///
/// Iterates every ship (player + NPC) because `operate_captain_ai` writes
/// `SetRedAlert` into each ship's own `AdmittedCommands` when its Captain
/// system is AI-controlled. Without per-entity dispatch, NPC captain-AI
/// red-alert changes would be silently dropped.
pub(crate) fn handle_set_red_alert(
    mut ship_query: Query<
        (
            &AdmittedCommands,
            &mut crate::ship::state::ShipRedAlert,
            Option<&crate::entities::spawner::EntityUuid>,
        ),
        With<crate::server_app::Ship>,
    >,
    // Balance telemetry. `Option<ResMut<Messages<_>>>` so bare-`App` fixtures
    // that never registered the message still pass parameter validation.
    mut balance_events: Option<
        ResMut<bevy::ecs::message::Messages<crate::core::balance::BalanceEvent>>,
    >,
    mut outbound: Option<
        ResMut<bevy::ecs::message::Messages<crate::lobby::server::OutboundMessage>>,
    >,
) {
    for (admitted, mut ra, ship_uuid) in ship_query.iter_mut() {
        for cmd in admitted.for_target(crate::ship::system_registry::RED_ALERT_SYSTEM_ID) {
            if let SystemControlPayload::SetRedAlert { active } = cmd.payload {
                // Assign, don't invert — the whole point of the set command
                // (issue #748). Only emit the balance tracer when the value
                // actually changes so idempotent retries don't spam telemetry.
                if ra.0 != active {
                    ra.0 = active;
                    // Balance tracer: every red-alert change, human or AI (both
                    // route through this same command), on every ship. Skipped
                    // for a ship with no uuid to key it on.
                    if let (Some(msgs), Some(uuid)) = (balance_events.as_mut(), ship_uuid) {
                        msgs.write(crate::core::balance::BalanceEvent::RedAlertChanged {
                            ship: uuid.0.clone(),
                            on: ra.0,
                        });
                    }
                }
                // Applied means this owning consumer actually consumed the due
                // command, including an idempotent same-value assignment.  The
                // normal Captain blackboard remains the only gameplay-state
                // response and may arrive separately.
                finish_action_feedback(cmd, &mut outbound, ActionFeedbackOutcome::Applied);
            }
        }
    }
}

fn view_request_from_admitted(
    cmd: &crate::core::messages::AdmittedCommand,
    ship_config: Option<&crate::ship_plugin::ShipConfigComponent>,
) -> Option<(SystemId, ViewMode)> {
    /// Map a "cinematic" marker name to the Cinematic view mode.
    fn resolve(mode: &ViewMode) -> ViewMode {
        match mode {
            ViewMode::Camera(cv) if cv.marker_name == "cinematic" => ViewMode::Cinematic,
            _ => mode.clone(),
        }
    }
    let config = ship_config.map(|config| &config.0);
    let has_authored_viewscreen = config.is_some_and(|config| {
        config
            .systems
            .iter()
            .any(|system| system.kind == crate::ship::system_registry::VIEWSCREEN_KIND)
    });
    let target_is_viewscreen = config
        .and_then(|config| config.system(&cmd.target))
        .is_some_and(|system| system.kind == crate::ship::system_registry::VIEWSCREEN_KIND)
        || (!has_authored_viewscreen
            && cmd.target.0 == crate::ship::system_registry::VIEWSCREEN_SYSTEM_ID);
    let SystemControlPayload::SetView { mode } = &cmd.payload else {
        return None;
    };
    if !target_is_viewscreen && cmd.target.0 != crate::ship::system_registry::HELM_STATION_ID {
        return None;
    }

    let source_kind = match mode {
        ViewMode::Camera(_) | ViewMode::Cinematic => crate::ship::system_registry::CAPTAIN_KIND,
        ViewMode::Radar => crate::ship::system_registry::HELM_RADAR_KIND,
        ViewMode::ScienceRadar | ViewMode::SensorsRadar => {
            crate::ship::system_registry::SENSORS_KIND
        }
        ViewMode::SystemChart | ViewMode::NavigationChart => {
            crate::ship::system_registry::NAVIGATION_KIND
        }
        ViewMode::Comms => crate::ship::system_registry::COMMS_KIND,
    };
    let source = config
        .and_then(|config| {
            config
                .systems
                .iter()
                .find(|system| system.kind == source_kind)
        })
        .map(|system| system.id.clone())
        .unwrap_or_else(|| crate::ship::viewscreen::source_system_for_view_mode(mode));
    Some((source, resolve(mode)))
}

/// Apply admitted viewscreen `SetView` requests to the local ship's
/// `ShipViewMode` under the latest-valid-command-wins policy (issue #769).
///
/// Runs in `SimSet::Input`. Comms' `handle_show_on_screen` is explicitly
/// ordered `.after` this system (see `CommsWorldPlugin`) so that when a
/// `SetView` and a `ShowOnScreen` land in the SAME tick the two requests are
/// applied in a deterministic order — `SetView` first, `ShowOnScreen` last —
/// making the monotonic arbiter `sequence` an authoritative total order rather
/// than depending on Bevy's ambiguous system-execution order.
pub(crate) fn handle_set_view(
    ship_query: Query<
        (
            &AdmittedCommands,
            Option<&crate::ship_plugin::ShipConfigComponent>,
        ),
        With<crate::server_app::LocalShip>,
    >,
    mut view_mode_q: Query<
        &mut crate::ship::state::ShipViewMode,
        With<crate::server_app::LocalShip>,
    >,
    mut outbound: Option<
        ResMut<bevy::ecs::message::Messages<crate::lobby::server::OutboundMessage>>,
    >,
) {
    let Some((admitted, ship_config)) = ship_query.iter().next() else {
        return;
    };
    let mut vm = view_mode_q.iter_mut().next();
    for cmd in admitted.0.iter() {
        if let Some((source, mode)) = view_request_from_admitted(cmd, ship_config) {
            let outcome = if let Some(view_mode) = vm.as_deref_mut() {
                view_mode.request_view_mode_from(source, mode);
                ActionFeedbackOutcome::Applied
            } else {
                ActionFeedbackOutcome::Refused
            };
            finish_action_feedback(cmd, &mut outbound, outcome);
        }
    }
}

/// When an AI captain takes over the ship the player is watching — a
/// "backfilled" captain, i.e. the Captain seat's own Control Source has gone
/// AI — it prefers the Cinematic camera over whatever view mode the ship
/// happened to be showing at the moment of takeover.
///
/// Nothing in the AI Captain doctrine emitted a `SetView` before this: the
/// authored `[captain_console.ai]` policy only ever declares the `red_alert`
/// channel (see `fragments/ai/captain_alliance.toml`), so a backfilled hull
/// simply kept the human's last view mode forever, cinematic or not. This is
/// a direct emission rather than a policy channel because the decision has no
/// tuning surface — "backfilled ⇒ cinematic" is the whole rule.
///
/// Scoped to `LocalShip`: view mode is a spectator concern, and no NPC's
/// `ShipViewMode` is read by anything (`handle_set_view` above carries the
/// same `With<LocalShip>` filter).
fn backfill_captain_prefers_cinematic_view(
    sessions: Res<crate::lobby::Sessions>,
    mut ship_query: Query<
        (
            &mut AdmittedCommands,
            &ShipSystemControlSources,
            &crate::ship::state::ShipViewMode,
            Option<&crate::entities::spawner::EntityUuid>,
            Option<&crate::ship_plugin::ShipConfigComponent>,
        ),
        With<crate::server_app::LocalShip>,
    >,
) {
    for (mut admitted, control_sources, view_mode, entity_uuid, ship_config) in
        ship_query.iter_mut()
    {
        // Both `Camera` and `Cinematic` `SetView` authorize off the CAPTAIN
        // system, not the viewscreen (`source_system_for_view_mode`,
        // `is_command_authorized`'s `effective_target` remap) — the viewscreen
        // itself has no seat to be human- or AI-operated, the Captain does.
        let captain_system_id = ship_config
            .and_then(|config| {
                config
                    .0
                    .systems
                    .iter()
                    .find(|system| system.kind == crate::ship::system_registry::CAPTAIN_KIND)
            })
            .map(|system| system.id.clone())
            .unwrap_or_else(crate::ship::system_registry::captain_system_id);
        let policy = control_sources.0.policy_for(&captain_system_id);
        if !policy.operate_ai {
            continue;
        }
        if view_mode.view_mode == ViewMode::Cinematic {
            // Already there — an explicit human `SetView` back to Cinematic,
            // or a previous tick of this same system. Either way, no-op so
            // admission is not spammed every tick.
            continue;
        }
        let viewscreen_system_id = ship_config
            .and_then(|config| {
                config
                    .0
                    .systems
                    .iter()
                    .find(|system| system.kind == crate::ship::system_registry::VIEWSCREEN_KIND)
            })
            .map(|system| system.id.clone())
            .unwrap_or_else(crate::ship::system_registry::viewscreen_system_id);
        emit_ai_command(
            entity_uuid,
            viewscreen_system_id,
            SystemControlPayload::SetView {
                mode: ViewMode::Cinematic,
            },
            control_sources,
            &sessions,
            ship_config,
            &mut admitted,
        );
    }
}

/// Toggle the captain's priority boost for a doctrine objective.
/// Sending the same id twice clears the boost.
fn handle_set_objective_priority(
    ship_query: Query<
        (
            &AdmittedCommands,
            Option<&crate::entities::spawner::EntityUuid>,
        ),
        With<crate::server_app::LocalShip>,
    >,
    mut boost: ResMut<crate::server_app::CaptainPriorityBoost>,
    mut outbound: Option<
        ResMut<bevy::ecs::message::Messages<crate::lobby::server::OutboundMessage>>,
    >,
) {
    let Some((admitted, uuid)) = ship_query.iter().next() else {
        return;
    };
    // Scope the boost to this captain's own ship (issue #752): the toggle
    // writes into the local ship's scope only, never a session-global slot.
    let scope =
        crate::server_app::CaptainPriorityBoost::scope_key(uuid.map(|u| u.0.as_str())).to_string();
    for cmd in admitted.for_target(crate::ship::system_registry::CAPTAIN_SYSTEM_ID) {
        if let SystemControlPayload::SetObjectivePriority { id } = &cmd.payload {
            boost.toggle(&scope, id);
            finish_action_feedback(cmd, &mut outbound, ActionFeedbackOutcome::Applied);
        }
    }
}

/// AI system: if the red-alert system is AI-controlled, evaluate this ship's
/// inline stateless [`CaptainAiPolicy`] over an immutable per-tick fact
/// snapshot (issue #775) and emit `SetRedAlert { active }` into
/// `AdmittedCommands` with the resolved state (issue #748). Runs before
/// `handle_set_red_alert` so the command is visible to the handler in the same
/// tick.
///
/// The emit is guarded on a state change purely to avoid admission spam — the
/// set command is idempotent, so a re-emit every tick would be harmless but
/// wasteful. Correctness does not depend on the guard.
///
/// After PRD #597 PR 10: reads combat timers from each ship's own
/// per-entity `RecentCombatActivity` component — no global resource. Loops over
/// all ship entities (player and NPC) where the Captain system is
/// `ControlSource::Ai`.
fn operate_captain_ai(
    time: Res<Time>,
    // `ai`-category decision-trace instrumentation (issue #1146). `Option<Res>`
    // for the bare-`App` reason the macro docs give; with `ai` logging off the
    // trace is skipped and the red-alert decision is unchanged.
    log: Option<Res<crate::logging::LogFilterConfig>>,
    sim_tick: Option<Res<crate::sim_tick::SimTick>>,
    sessions: Res<crate::lobby::Sessions>,
    // Issue #912: the SHARED per-tick world frame, not a scan of the captain's
    // own. `Option<Res<_>>` because bare-`App` fixtures never register
    // `AiPlugin`/the config cache; an absent resource seeds "no contact", which
    // is the safe reading (see `nearest_hostile_range`).
    world_snapshot: Option<Res<crate::ai::server::WorldSnapshot>>,
    faction_registry: Option<Res<crate::entities::config_cache::FactionRegistryResource>>,
    // The read-only AI-host world context — flag chain, sessions, and origin
    // stamps — behind one bare-`Res` system param (issue #1207). A fixture that
    // runs this host must register it (`register_ai_host_env`) or fail loudly at
    // schedule build, so a bare `App` cannot silently diverge from production.
    ai_env: crate::ai::host::AiHostEnv,
    mut ship_query: Query<(
        Entity,
        &mut AdmittedCommands,
        &ShipSystemControlSources,
        &RecentCombatActivity,
        Option<&crate::ship::state::ShipRedAlert>,
        Option<&crate::entities::spawner::EntityUuid>,
        Option<&crate::ship_plugin::ShipConfigComponent>,
        Option<&CaptainAiPolicy>,
        Option<&crate::ship::state::ShipPhysics>,
        Option<&crate::entities::spawner::FactionComponent>,
        // Display name for the `ai` decision trace's `ship` field (issue #1146).
        Option<&crate::entities::spawner::EntityName>,
    )>,
) {
    let now = time.elapsed_secs();
    let registry = faction_registry.as_deref().map(|r| &r.0);

    for (
        ship_entity,
        mut admitted,
        control_sources,
        activity,
        red_alert_opt,
        entity_uuid,
        ship_config,
        ship_policy,
        physics,
        faction,
        name_opt,
    ) in ship_query.iter_mut()
    {
        // Build the immutable typed-fact snapshot for this evaluation from this
        // ship's own combat activity. `secs_since_combat` is absent when the
        // ship has no combat history at all — the policy then reads "not in
        // combat" via the absent-fact rule.
        let last_combat = most_recent(
            most_recent(activity.last_damage_taken, activity.last_hostile_fire_taken),
            activity.last_weapon_fired,
        );
        let mut facts = crate::world::flags::AiFacts::new();
        if let Some(s) = last_combat {
            facts.set_fact(
                crate::entities::ai_flag_hosts::SECS_SINCE_COMBAT,
                (now - s) as f64,
            );
        }

        // First-contact readings (issue #912). ALWAYS seeded, both of them, so
        // an authored guard reads "clear" rather than "absent" — an absent fact
        // makes every comparison false, so a conditionally-seeded presence fact
        // is a dead guard with no error anywhere.
        let hostile_range = nearest_hostile_range(
            world_snapshot.as_deref(),
            registry,
            physics,
            faction,
            entity_uuid,
        );
        facts.set(
            crate::entities::config::CAPTAIN_HOSTILE_CONTACT_FACT,
            if hostile_range.is_some() { 1.0 } else { 0.0 },
        );
        facts.set(
            crate::entities::config::CAPTAIN_HOSTILE_RANGE_FACT,
            hostile_range.unwrap_or(0.0) as f64,
        );

        // Gate → declare → resolve the `red_alert` channel through the shared AI
        // host spine (issue #1208): the Control-Source gate, the strict
        // AI-declaration check (an absent `[captain_console.ai]` ⇒ `Undeclared`
        // ⇒ no automation, PRD #774 US7) and the channel resolution all live in
        // `decide`. The scenario flag chain is anchored at the layer that spawned
        // this ship (issue #891 stage 2). The Captain drives only `red_alert`, so
        // any verb other than `SetRedAlert` — and every non-acting outcome —
        // means "no Red Alert decision this tick".
        let flag_chain = ai_env.flag_chain(ship_entity);
        let tick = crate::ai::host::HostTick {
            system: crate::ship::system_registry::red_alert_system_id(),
            channel: crate::entities::config::CAPTAIN_RED_ALERT_CHANNEL,
            facts: &facts,
            flags: &flag_chain,
            state: None,
        };
        let should_be_red_alert =
            match crate::ai::host::decide(&control_sources.0, ship_policy.map(|p| &p.0), &tick) {
                crate::ai::host::HostOutcome::Act(
                    crate::ai::policy::AiPolicyVerb::SetRedAlert(b),
                ) => Some(*b),
                _ => None,
            };

        if let Some(should_be_red_alert) = should_be_red_alert {
            let current_red_alert = red_alert_opt.map(|ra| ra.0).unwrap_or(false);
            if should_be_red_alert != current_red_alert {
                // Console-AI decision trace (issue #1146): the Captain host is a
                // fine-system AI host routed through the `ai::host` spine, and its
                // one decision is the Red Alert lever. Emit the change as a
                // STRUCTURED `ai`-category event (tick/ship/prev/new) so it joins
                // the doctrine and target timelines in one filterable stream. The
                // `pinfo!` gate does the level + `--log-entity` check; when `ai`
                // logging is off this is a couple of reads and no formatting.
                crate::pinfo!(
                    log,
                    crate::logging::LogCat::Ai,
                    entity = ship_entity,
                    ai_event = "red_alert_change",
                    tick = sim_tick.as_deref().map(|t| t.0).unwrap_or(0),
                    ship = name_opt.map(|n| n.0.as_str()).unwrap_or("<unnamed>"),
                    prev = current_red_alert,
                    new = should_be_red_alert,
                    "red alert {current_red_alert} -> {should_be_red_alert}"
                );
                // Route through the shared admission seam with this ship's own
                // `ai:<uuid>` token (issue #830) rather than pushing straight
                // into `AdmittedCommands` — true AI/human symmetry, binding the
                // red-alert `SystemId` at this call site. Emits the same idempotent
                // `SetRedAlert` command a human captain sends (issue #748); the
                // on-change guard only avoids admission spam.
                emit_ai_command(
                    entity_uuid,
                    crate::ship::system_registry::red_alert_system_id(),
                    SystemControlPayload::SetRedAlert {
                        active: should_be_red_alert,
                    },
                    control_sources,
                    &sessions,
                    ship_config,
                    &mut admitted,
                );
            }
        }
    }
}

/// Planar range to this ship's nearest faction-hostile contact, or `None` when
/// it has no hostile contact at all (issue #912).
///
/// # It reads the shared frame — it does not scan
///
/// The contacts come from the one per-tick [`crate::ai::server::WorldSnapshot`]
/// that `build_world_snapshot` publishes, the same producer the Helm's world
/// view and Tactical's nearest-hostile tier already read, and the hostile
/// verdict and the geometry are delegated to the same pure helpers those two
/// use ([`crate::ai::find_nearest_hostile`], [`crate::ai::target_relative_motion`]).
/// So the captain's answer to "is there an enemy out there, and how far" agrees
/// with the two consoles that act on it by construction, rather than by two
/// scans being kept in step by hand.
///
/// # Ordering: this is LAST tick's snapshot, deliberately
///
/// `operate_captain_ai` runs in `SimSet::Input`; `build_world_snapshot` runs in
/// `SimSet::Physics`. The captain therefore reads the PREVIOUS tick's frame.
/// That is the project's frozen-snapshot doctrine (`src/sim_sets.rs`) working as
/// intended — every AI consumer decides against one immutable frame instead of
/// racing the producer — and costs a one-tick lag on a decision whose authored
/// threshold is measured in tens of world units. It is not a missing ordering
/// edge, and adding one would put the captain inside the frame it reads.
///
/// # No radar gate here, on purpose
///
/// The Captain console owns no radar, and inventing a reach for it in Rust would
/// pin a gameplay distance in code (AGENTS.md rule #11). The horizon is the
/// authored `param(...)` the hull's guard compares this range against, so a
/// designer decides at what range a contact becomes an alert — and can author a
/// cautious hull as easily as an aggressive one.
///
/// `None` for a ship with no position, no faction, or no readable snapshot: all
/// three mean "this host cannot see anything", which seeds the safe reading.
fn nearest_hostile_range(
    snapshot: Option<&crate::ai::server::WorldSnapshot>,
    registry: Option<&crate::ai::faction::FactionRegistry>,
    physics: Option<&crate::ship::state::ShipPhysics>,
    faction: Option<&crate::entities::spawner::FactionComponent>,
    entity_uuid: Option<&crate::entities::spawner::EntityUuid>,
) -> Option<f32> {
    let snapshot = snapshot?;
    let registry = registry?;
    let physics = physics?;
    let faction = faction?;

    let self_uuid = entity_uuid.map(|u| u.0.as_str()).unwrap_or("");
    let entities: Vec<crate::ai::AiWorldEntity> = snapshot
        .entities
        .iter()
        .filter(|e| e.uuid.to_string() != self_uuid)
        .cloned()
        .collect();
    let world_view = crate::ai::WorldView {
        entity_pos: [physics.x, 0.0, physics.z],
        entity_yaw: physics.yaw,
        entities,
        self_faction: Some(faction.0),
        ..crate::ai::WorldView::default()
    };

    let nearest = crate::ai::find_nearest_hostile(&world_view, registry)?;
    let target = world_view.entities.iter().find(|e| e.uuid == nearest)?;
    Some(
        crate::ai::target_relative_motion(
            world_view.entity_pos,
            physics.yaw,
            physics.forward_speed,
            target.position,
            target.yaw,
            target.forward_speed,
        )
        .range,
    )
}

fn most_recent(a: Option<f32>, b: Option<f32>) -> Option<f32> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

// ── Blackboard publish ───────────────────────────────────────────────────────

/// Per-`Ship` publisher (issue #830). Ship-wide fields (red_alert, auto flags,
/// hull integrity, game_status) are computed for every ship from its own
/// per-entity `ShipRedAlert` + `ShipSystemControlSources` + `EntitySystemHull`.
/// Player-only fields — camera views (from the local `ModelMarkers` /
/// `CinematicCameraSection`), view direction/mode (from the local
/// `ShipViewMode`), and the objectives list + boost (from `ObjectiveManagerRes`
/// / `CaptainPriorityBoost`) — are gated on `Has<LocalShip>`; NPCs get the
/// empty/default equivalents (nothing reads an NPC captain blackboard, and the
/// wire broadcaster is `LocalShip`-filtered).
fn publish_captain_blackboard(
    objectives: Option<Res<ObjectiveManagerRes>>,
    // The named-deadline readout (issue #1024). Both `Option` so a bare-`App`
    // fixture and a world with no content runtime keep working; both read-only,
    // so this system stays a pure publisher.
    world_content: Option<Res<crate::world::server::WorldContentRuntime>>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    sim_tick: Option<Res<crate::sim_tick::SimTick>>,
    boost: Res<crate::server_app::CaptainPriorityBoost>,
    markers_q: Query<&crate::entities::model_rig::ModelMarkers, With<crate::server_app::LocalShip>>,
    cinematic_q: Query<
        Option<&crate::entities::spawner::CinematicCameraSection>,
        With<crate::server_app::LocalShip>,
    >,
    mut ship_query: Query<
        (
            &ShipSystemControlSources,
            Option<&crate::ship::state::ShipRedAlert>,
            Option<&crate::ship::state::ShipViewMode>,
            Option<&crate::entities::spawner::EntitySystemHull>,
            Option<&crate::entities::spawner::EntityUuid>,
            Option<&crate::ship_plugin::ShipConfigComponent>,
            bevy::ecs::query::Has<crate::server_app::LocalShip>,
            &mut crate::server_app::ShipSystemBlackboards,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    for (
        control_sources,
        red_alert_comp,
        view_mode_comp,
        hull_opt,
        uuid_opt,
        ship_config,
        is_local,
        mut bbs,
    ) in ship_query.iter_mut()
    {
        let red_alert = red_alert_comp.map(|ra| ra.0).unwrap_or(false);

        let (hull_fraction, hull_integrity_pct) = hull_opt
            .map(|h| {
                let max = h.0.total_max();
                if max > 0.0 {
                    let frac = h.0.total_current() / max;
                    (frac, (frac * 100.0).clamp(0.0, 100.0))
                } else {
                    (1.0, 100.0)
                }
            })
            .unwrap_or((1.0, 100.0));

        let red_alert_auto = control_sources
            .0
            .source_for(&crate::ship::system_registry::red_alert_system_id())
            == ControlSource::Ai;
        let viewscreen_system_id = ship_config
            .and_then(|config| {
                config
                    .0
                    .systems
                    .iter()
                    .find(|system| system.kind == crate::ship::system_registry::VIEWSCREEN_KIND)
            })
            .map(|system| system.id.clone())
            .unwrap_or_else(crate::ship::system_registry::viewscreen_system_id);
        let viewscreen_auto =
            control_sources.0.source_for(&viewscreen_system_id) == ControlSource::Ai;

        // ── Player-only fields (LocalShip) ────────────────────────────────────
        // View mode / camera list / objectives are player camera + doctrine
        // surfaces. NPCs get the same defaults the pre-#830 `.single()` error
        // arms produced for a ship missing the component.
        let view_mode = if is_local {
            view_mode_comp
                .map(|vm| vm.view_mode.clone())
                .unwrap_or(ViewMode::Camera(CameraView::default()))
        } else {
            ViewMode::Camera(CameraView::default())
        };
        let view_direction = match &view_mode {
            ViewMode::Camera(cv) => cv.marker_name.clone(),
            ViewMode::Cinematic => "cinematic".to_string(),
            _ => String::new(),
        };

        let mut camera_views: Vec<String> = Vec::new();
        let mut objectives_snap: Vec<ObjectiveSnapshot> = Vec::new();
        let mut boosted_objective_id: Option<String> = None;
        let mut deadlines: Vec<crate::core::messages::DeadlineSnapshot> = Vec::new();
        if is_local {
            camera_views = markers_q
                .single()
                .ok()
                .map(|mm| {
                    mm.marker_names()
                        .filter(|n| n.starts_with("camera_"))
                        .map(|n| n.to_string())
                        .collect()
                })
                .unwrap_or_default();
            // `marker_names` walks a `HashMap`, whose order is process-local; this
            // list rides the captain blackboard into the #862 snapshot, so sort it
            // or two same-seed peers serialize the same cameras to different bytes.
            // "cinematic" is appended after, keeping it last regardless.
            camera_views.sort();
            let has_cinematic = cinematic_q.single().ok().is_some_and(|c| c.is_some());
            if has_cinematic {
                camera_views.push("cinematic".to_string());
            }

            let conditions = WorldConditions {
                red_alert,
                hull_fraction,
                attacked: false,
            };
            // Scope the boost to this ship (issue #752): a captain's priority
            // pick only reorders its own ship's objective consumers.
            let scope =
                crate::server_app::CaptainPriorityBoost::scope_key(uuid_opt.map(|u| u.0.as_str()));
            let captain_boost = boost.boost_arg(scope);
            objectives_snap = objectives
                .as_ref()
                .map(|obj| {
                    let scored = obj.0.scored_pool_with_boost(&conditions, captain_boost);
                    scored
                        .into_iter()
                        .filter(crate::objectives::is_visible_objective)
                        .map(|o| o.snapshot)
                        .collect()
                })
                .unwrap_or_default();
            boosted_objective_id = boost.boosted_for(scope).map(str::to_string);

            // Visible deadlines only: `visible = false` is a mission keeping a
            // clock to itself, and the wire is the boundary that keeps it there.
            // `remaining_secs` is computed HERE, against the authoritative
            // `SimTick`, so the console renders a number rather than guessing at
            // one (issue #1024).
            if let Some(content) = world_content.as_ref() {
                let now_tick = sim_tick.as_ref().map(|t| t.0).unwrap_or(0);
                let tick_hz = world_config
                    .as_ref()
                    .map(|wc| wc.global.sim_tick_hz)
                    .unwrap_or(crate::world::script::schedule::SchedClock::ZERO.tick_hz);
                deadlines = content
                    .deadlines
                    .records
                    .iter()
                    .filter(|record| record.visible)
                    .map(|record| crate::core::messages::DeadlineSnapshot {
                        id: record.presentation_id(),
                        label: record.label.clone(),
                        remaining_secs: content.deadlines.remaining_secs_scoped(
                            record.origin_layer.as_deref(),
                            &record.id,
                            now_tick,
                            tick_hz,
                        ),
                        state: record.state.as_str().to_string(),
                    })
                    .collect();
            }
        }

        let game_status = if red_alert {
            "RED ALERT — All hands to battlestations."
        } else {
            "Standing by. All systems nominal."
        }
        .to_string();

        let bb = CaptainBlackboard {
            red_alert,
            red_alert_system_id: crate::ship::system_registry::red_alert_system_id(),
            red_alert_auto,
            viewscreen_system_id,
            viewscreen_auto,
            view_direction,
            view_mode,
            camera_views,
            objectives: objectives_snap,
            hull_integrity_pct,
            game_status,
            boosted_objective_id,
            deadlines,
        };

        bbs.0.insert(
            SystemId(crate::ship::system_registry::CAPTAIN_SYSTEM_ID.to_string()),
            SystemBlackboard::Captain(bb),
        );
    }
}

#[cfg(test)]
// Fixture ids only (issue #907): a test that needs "some distinct id" has no
// run to reproduce. Production identity is minted by `crate::world_id`, and
// clippy.toml bans `Uuid::new_v4` outside scopes like this one.
#[allow(clippy::disallowed_methods)]
#[path = "server_tests.rs"]
mod tests;
