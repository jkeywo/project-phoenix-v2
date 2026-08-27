use super::*;
use crate::core::messages::{ClientMessage, InterSystemPayload, InterSystemQueue};
use crate::server_app::Ship;
use crate::ship::components::ShipConfigComponent;
use crate::ship::components::HELM_AI_MAX_DT_SECS;
use crate::ship::control_source::{ControlSource, ControlSourceResolver};
use crate::ship::physics::ShipPhysicsConfig;
use crate::ship::test_support::*;
use crate::simmath;

// ── Table-driven per-axis wiring guard (issue #1208) ──────────────────────
//
// Every `HelmAxisHost` must resolve its OWN accepted verb through the shared
// `decide` spine when its fine system is AI-operated and a policy fires, and
// must stand DOWN — the spine's Control-Source gate returning
// `NotAiOperated` — under a human Control Source. A new axis cannot be added
// without a row below, because each call names its impl type: the six
// `assert_axis_wiring::<_>` lines are the registry, and an unwired seventh
// axis has no entry to prove it gates and fires. The per-axis Bevy tests
// elsewhere in this module carry the second half — that the resolved verb's
// payload lands in `AdmittedCommands` and that a human emits nothing — end to
// end through the real systems.

/// A one-rule policy that fires `verb` on `channel` unconditionally.
fn wiring_firing_policy(
    channel: &str,
    verb: crate::ai::policy::AiPolicyVerb,
) -> crate::ai::policy::AiPolicy {
    crate::ai::policy::AiPolicy {
        params: crate::world::flags::AiParams::new(),
        rules: vec![crate::ai::policy::AiPolicyRule {
            priority: 0,
            channel: channel.into(),
            when: crate::world::flags::parse_predicate("true").unwrap(),
            verb,
        }],
        idle: false,
        machine: None,
    }
}

/// AI on `system`, human on everything else.
fn wiring_ai_sources(system: crate::core::messages::SystemId) -> ShipSystemControlSources {
    let mut resolver = ControlSourceResolver::new();
    resolver.set(system, ControlSource::Ai);
    ShipSystemControlSources(resolver)
}

fn assert_axis_wiring<H: HelmAxisHost>(fire_verb: crate::ai::policy::AiPolicyVerb) {
    let policy = wiring_firing_policy(H::CHANNEL, fire_verb);
    let facts = crate::world::flags::AiFacts::new();
    let flags: [&crate::world::flags::FlagStore; 0] = [];

    // AI-operated on this axis + a policy that fires ⇒ the spine resolves the
    // axis's own accepted verb.
    let ai = wiring_ai_sources(H::system_id());
    match helm_axis_outcome::<H>(&ai, Some(&policy), None, &facts, 0.0, &flags) {
        crate::ai::host::HostOutcome::Act(v) => assert!(
            H::accepts(v),
            "axis on channel '{}' resolved a verb it does not accept",
            H::CHANNEL
        ),
        other => panic!(
            "axis on channel '{}' did not fire under AI + a firing policy: {other:?}",
            H::CHANNEL
        ),
    }

    // Human Control Source ⇒ the gate stands the axis down before it resolves
    // anything — no verb, no payload.
    let human = ShipSystemControlSources(ControlSourceResolver::new());
    assert_eq!(
        helm_axis_outcome::<H>(&human, Some(&policy), None, &facts, 0.0, &flags),
        crate::ai::host::HostOutcome::NotAiOperated,
        "axis on channel '{}' resolved under a human Control Source",
        H::CHANNEL
    );
}

#[test]
fn every_helm_axis_gates_on_ai_and_resolves_its_verb() {
    assert_axis_wiring::<EnginesAxis>(crate::ai::policy::AiPolicyVerb::ActuateDesiredTravel);
    assert_axis_wiring::<SteeringAxis>(crate::ai::policy::AiPolicyVerb::ActuateDesiredFacing);
    assert_axis_wiring::<ImpulseAxis>(crate::ai::policy::AiPolicyVerb::EngageImpulse);
    assert_axis_wiring::<LateralAxis>(crate::ai::policy::AiPolicyVerb::ActuateLateralThrust);
    assert_axis_wiring::<VerticalAxis>(crate::ai::policy::AiPolicyVerb::ActuateVerticalThrust);
    assert_axis_wiring::<BoostAxis>(crate::ai::policy::AiPolicyVerb::EngageBoost);
}

/// Lock this ship's Tactical surface onto `uuid` (issue #702).
///
/// The helm pursues `TacticalRadarSelection`; it no longer resolves a `Destroy`
/// directive's authored name itself. In production `ai_target_selection`
/// does that resolution (tier 1) and publishes the result here, so a test
/// that poses a Destroy objective and expects pursuit must supply the lock
/// that system would have written.
fn set_ship_weapons_target(app: &mut App, uuid: &str) {
    let ship = find_ship_entity(app);
    let mut entity = app.world_mut().entity_mut(ship);
    let mut target = entity
        .get_mut::<crate::console::weapons::TacticalRadarSelection>()
        .expect("ship must carry TacticalRadarSelection");
    target.0 = Some(uuid.to_string());
}

/// Give this ship a Navigation waypoint *and* the Channel-3 clearance to
/// fly it, as `operate_navigation_ai` → the lag router → Helm's receiver would
/// once the order came due (issue #702). Returns the waypoint's generation.
use crate::console::navigation::WaypointMode;

fn set_cleared_nav_waypoint(app: &mut App, x: f32, z: f32) -> u64 {
    let ship = find_ship_entity(app);
    let generation = {
        let mut entity = app.world_mut().entity_mut(ship);
        let mut waypoint = entity
            .get_mut::<crate::console::navigation::NavigationWaypoint>()
            .expect("ship must carry NavigationWaypoint");
        waypoint.set(WaypointMode::Free { x, z });
        waypoint.generation()
    };
    let mut entity = app.world_mut().entity_mut(ship);
    let mut clearance = entity
        .get_mut::<HelmWaypointClearance>()
        .expect("ship must carry HelmWaypointClearance");
    clearance.0 = Some(generation);
    generation
}

// ── #575: player ship AI helm navigation ──────────────────────────────────

// ── Per-axis helm AI (issue #701) ──────────────────────────────────────

fn get_impulse_command(app: &mut App) -> crate::ship::impulse::ImpulsePhase {
    app.world_mut()
        .query_filtered::<&ImpulseCommand, With<Ship>>()
        .single(app.world())
        .expect("expected Ship with ImpulseCommand")
        .0
}

fn set_impulse_command(app: &mut App, phase: crate::ship::impulse::ImpulsePhase) {
    let ship = find_ship_entity(app);
    app.world_mut()
        .entity_mut(ship)
        .get_mut::<ImpulseCommand>()
        .expect("expected ImpulseCommand")
        .0 = phase;
}

/// A `[behaviour]` section whose one doctrine entry matches `objective_id`
/// and permits impulse.
///
/// `use_impulse` is authored explicitly rather than left to
/// `effective_use_impulse`'s directive-kind default, so these tests pin the
/// impulse *system* and not that default (which says `false` for Patrol —
/// the directive some of them use).
///
/// `target_speed`/`maintain_range` are restated because `DoctrineObjective`
/// derives `Default`, which zeroes them rather than applying their serde
/// `default =` values; a zero `target_speed` would silently pin the helm's
/// throttle at 0 alongside whatever the test meant to measure.
fn impulse_doctrine(objective_id: &str) -> crate::entities::config::BehaviourConfig {
    crate::entities::config::BehaviourConfig {
        doctrine: vec![crate::entities::config::DoctrineObjective {
            id: objective_id.into(),
            use_impulse: Some(true),
            target_speed: 0.8,
            maintain_range: 25.0,
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// A ship set up for `ai_helm_impulse`: a per-hull impulse config (the
/// system no-ops without one) and helm-impulse on AI. The coarse helm is
/// left Human — which until #704 was what kept `operate_helm_ai` from being
/// the writer of the `ImpulseCommand` these tests measure, and now simply
/// isolates the axis. The shipped-hull test below exercises the everything-AI
/// case.
fn impulse_ai_app(objective: crate::core::messages::ScoredObjective) -> App {
    let mut app = test_app();
    let objective_id = objective.id.clone();
    set_ship_blackboard_objectives(&mut app, vec![objective]);
    set_behaviour_section(&mut app, impulse_doctrine(&objective_id));
    let ship = find_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(ship)
        .insert(ImpulseConfigResource::default());
    set_helm_control_source(&mut app, ControlSource::Human);
    set_fine_control_source(
        &mut app,
        crate::ship::system_registry::helm_impulse_system_id(),
        ControlSource::Ai,
    );
    app
}

/// Build a `ControlSourceResolver` from a shipped hull's TOML the way the
/// game does when nobody is driving: parse the file, then set every
/// *declared* system to `ControlSource::Ai`. That is literally what the NPC
/// spawn path (`crate::entities::spawner`) does, and what the `Backfill`
/// rating does to a player hull whose station goes unmanned — the two hull
/// families reach the same end state, so the same helper serves both.
///
/// Nothing is hand-set, so the resolver reflects exactly what the hull
/// declares — which is the point of the tests that use it.
/// Takes a hull STEM rather than baked text (issue #875). `include_str!`
/// bakes bytes at compile time, so a baked site can never see include
/// resolution — and `alliance_destroyer` is a COMPOSED hull since #875, so
/// its baked bytes are no longer the document the game loads. Going through
/// `load_entity_config` keeps the claim "reads the shipped TOMLs through the
/// same resolver the game builds" literally true for composed and
/// uncomposed hulls alike.
fn resolver_from_shipped_hull(stem: &str) -> ControlSourceResolver {
    let path = format!("assets/entities/{stem}.toml");
    let config = crate::entities::include_resolve::load_entity_config(&path)
        .unwrap_or_else(|e| panic!("shipped hull {stem} must compose and parse: {e}"));
    let ship_config = config
        .ship_config
        .expect("shipped hull must declare [[system]] blocks");
    let mut resolver = ControlSourceResolver::new();
    for system in &ship_config.systems {
        resolver.set(system.id.clone(), ControlSource::Ai);
    }
    resolver
}

/// #704's precondition, pinned against every hull the game ships.
///
/// The delete is only behaviour-preserving if every hull declares every axis.
/// `ControlSource::default()` is `Human` (`operate_ai == false`), so an
/// *undeclared* axis resolves to "human-held" and its per-axis system stands
/// down — and until #704 the `operate_helm_ai` monolith silently covered that
/// case, because it stood down only from axes that were declared *and* AI.
/// Undeclare an axis after the delete and nothing writes that component at
/// all: the ship loses the behaviour, quietly, with every test still green.
///
/// That is not hypothetical. When #704 went to delete the monolith, five NPC
/// hulls declared neither `helm-impulse` nor `helm-lateral-thrust`, and
/// `alliance_battleship` declared no `helm-lateral-thrust` — so the monolith
/// was still driving impulse and the avoidance dodge on their behalf, and
/// deleting it would have removed both. #704 declares them; this test is what
/// stops the gap re-opening, and it is deliberately a *table over every hull*
/// rather than one hull per axis, because the previous shipped-hull tests
/// (`shipped_hull_config_drives_the_per_axis_helm_systems` on the enemy destroyer,
/// `shipped_hull_config_drives_ai_helm_lateral_thrust` on `alliance_cruiser`)
/// each pinned one hull and one axis pair, which is exactly how six hulls
/// drifted without anything going red.
///
/// Reads the shipped TOMLs through the same resolver the game builds, so it
/// fails on the declaration a hull is actually missing rather than on a
/// hand-built fixture's idea of one.
#[test]
fn every_shipped_hull_declares_every_helm_axis() {
    // (#892) `pirate_raider` + `pirate_raider_reinforcement` were retired as
    // duplicates of `ship_harrow_destroyer`'s display name; the surviving
    // hull takes their two rows.
    let hulls: [&str; 8] = [
        "alliance_battleship",
        "alliance_courier",
        "alliance_cruiser",
        "alliance_destroyer",
        "ship_harrow_destroyer",
        "ship_harrow_patrol",
        "ship_harrow_warhawk",
        "ship_requiem_courier",
    ];

    let axes: [(&str, crate::core::messages::SystemId); 5] = [
        (
            "helm-thrust",
            crate::ship::system_registry::helm_thrust_system_id(),
        ),
        (
            "helm-steering",
            crate::ship::system_registry::helm_steering_system_id(),
        ),
        (
            "helm-impulse",
            crate::ship::system_registry::helm_impulse_system_id(),
        ),
        (
            "helm-lateral-thrust",
            crate::ship::system_registry::lateral_thrust_system_id(),
        ),
        (
            "helm-boost",
            crate::ship::system_registry::helm_boost_system_id(),
        ),
    ];

    for hull in hulls {
        let resolver = resolver_from_shipped_hull(hull);

        // Sanity (#801): the coarse `helm` system is deleted from every
        // shipped hull — a TOML that still declared it would fail parse
        // (the kind is unregistered), but pin the resolver view too.
        assert!(
            !resolver
                .policy_for(&crate::core::messages::SystemId(
                    crate::ship::system_registry::HELM_STATION_ID.to_string()
                ))
                .operate_ai,
            "{hull} must NOT declare a coarse `helm` system (#801)"
        );

        for (axis_name, axis_id) in &axes {
            assert!(
                resolver.policy_for(axis_id).operate_ai,
                "{hull} does not declare `{axis_name}`. Since #704 deleted \
                 operate_helm_ai there is no coarse fallback: an undeclared axis \
                 resolves to ControlSource::Human, its per-axis system stands down, \
                 and nothing writes that intent component at all — the hull silently \
                 loses the behaviour. Declare it in the hull TOML with the same owner \
                 as the coarse `helm`"
            );
        }
    }
}

/// Install a resolver verbatim on every ship, replacing whatever the test
/// harness set up.
fn install_control_sources(app: &mut App, resolver: &ControlSourceResolver) {
    let mut q = app
        .world_mut()
        .query_filtered::<&mut ShipSystemControlSources, With<Ship>>();
    for mut cs in q.iter_mut(app.world_mut()) {
        cs.0 = resolver.clone();
    }
}

/// AC5 (issue #800), and the coverage gap that let the dormancy ship.
///
/// Every other per-axis test hand-builds its control sources, so all of
/// them passed while `helm-thrust` / `helm-steering` were declared in
/// **zero** shipped TOMLs — their policy defaulted to Human, the per-axis
/// systems never fired in shipped content, and `operate_helm_ai` quietly did
/// all the work. This test refuses to hand-build: the sources come from a
/// real shipped hull.
///
/// That the *per-axis* systems produced the intent needed proving while the
/// monolith was alive, and the proof was its stand-down: this hull declares
/// every axis and the NPC spawn path backfills each to AI, so
/// `operate_helm_ai` skipped both writes. Since #704 deleted it the point is
/// simply structural — a non-zero intent has no other possible writer.
#[test]
fn shipped_hull_config_drives_the_per_axis_helm_systems() {
    let resolver = resolver_from_shipped_hull("ship_harrow_destroyer");

    // The declaration itself — what #800 adds, and what was missing.
    assert!(
        resolver
            .policy_for(&crate::ship::system_registry::helm_thrust_system_id())
            .operate_ai,
        "the shipped hull must declare helm-thrust, or ai_helm_thrust is dormant \
         in shipped content"
    );
    assert!(
        resolver
            .policy_for(&crate::ship::system_registry::helm_steering_system_id())
            .operate_ai,
        "the shipped hull must declare helm-steering, or ai_helm_steering is dormant \
         in shipped content"
    );
    // #801: the shipped hull no longer declares a coarse helm at all —
    // the per-axis declarations above are the whole story.

    let mut app = test_app();
    let anchor = "station-alpha";
    set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
    app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
    install_control_sources(&mut app, &resolver);

    tick(&mut app);

    assert!(
        get_thrust_input(&mut app) > 0.0,
        "ai_helm_thrust must drive a shipped hull's throttle toward a Reach anchor \
         (since #704 it is the thrust axis's only AI writer)"
    );
    assert!(
        get_steering_input(&mut app).abs() > 0.0,
        "ai_helm_steering must drive a shipped hull's yaw toward a Reach anchor \
         (since #704 it is the steering axis's only AI writer)"
    );
}

/// Ported in #704 from `shipped_hull_per_axis_intent_matches_the_coarse_path`,
/// which pinned the #800 migration on a real hull: the per-axis path had to
/// reproduce the monolith's intent exactly, so `run(&shipped)` had to equal
/// `run(&pre_800)`. `pre_800` *is* the monolith path, so the delete removes
/// the right-hand side of that equality outright.
///
/// Kept, with both terms retained and the question changed from "do these
/// agree?" to "which of these still drives the ship?". That is the honest
/// successor: the old test's whole point was that the two paths were
/// interchangeable on shipped content, and #704's point is that only one of
/// them exists. Same hull, same resolver, same objective, same measurement.
///
/// The `pre_800` arm is what makes this more than a restatement of
/// `shipped_hull_config_drives_the_per_axis_helm_systems`: it pins that the
/// hull's *declarations* are load-bearing. Strip `helm-thrust`/`helm-steering`
/// back out of `ship_harrow_destroyer.toml` and the shipped arm keeps passing on a
/// coarse fallback if one ever returns — this arm would not.
#[test]
fn shipped_hull_helm_is_driven_by_the_per_axis_declarations_alone() {
    let anchor = "station-alpha";

    let run = |resolver: &ControlSourceResolver| {
        let mut app = test_app();
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        install_control_sources(&mut app, resolver);
        tick(&mut app);
        (get_thrust_input(&mut app), get_steering_input(&mut app))
    };

    let shipped = resolver_from_shipped_hull("ship_harrow_destroyer");

    // The same hull as it behaved before #800: coarse helm on AI, the two
    // axes undeclared and therefore Human by default.
    let mut pre_800 = shipped.clone();
    pre_800.set(
        crate::ship::system_registry::helm_thrust_system_id(),
        ControlSource::Human,
    );
    pre_800.set(
        crate::ship::system_registry::helm_steering_system_id(),
        ControlSource::Human,
    );

    let shipped_intent = run(&shipped);
    assert!(
        shipped_intent.0 > 0.0 && shipped_intent.1.abs() > 0.0,
        "a shipped hull's declared per-axis systems must drive it toward a Reach \
         anchor (got {shipped_intent:?})"
    );
    assert_eq!(
        run(&pre_800),
        (0.0, 0.0),
        "with helm-thrust/helm-steering undeclared the hull's coarse helm is on AI \
         and nothing else is — the shape operate_helm_ai used to serve. Since #704 \
         deleted it that ship must not move: the axis declarations, not the coarse \
         system, are what fly it"
    );
}

/// AC3 on the shipped-hull shape: the Weapons->Helm arc-bearing bias (#677)
/// must survive the move to the per-axis path. Before #800 the bias reached
/// shipped hulls via `operate_helm_ai`; now `ai_helm_steering` owns steering
/// there and has to fold it in instead. Nothing else pins that on a real
/// hull's control sources.
///
/// Note this does *not* pin the monolith's arc-bearing stand-down: both
/// systems compute the same bias from the same inputs, so calling it twice
/// is currently unobservable. See the comment at that call site.
#[test]
fn shipped_hull_helm_ai_folds_pending_arc_bearing_request_into_steering() {
    let mut app = test_app();
    // Destroy target directly ahead and far away, so the baseline pursuit
    // steering (before any arc-bearing bias) is ~0.
    let destroy_uuid = uuid::Uuid::new_v4().to_string();
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(destroy_uuid.clone()),
        crate::entities::spawner::EntityName("Harrow Destroyer".into()),
        Transform::from_xyz(0.0, 0.0, -1000.0),
    ));
    let mut runtime = crate::world::server::WorldContentRuntime::default();
    runtime.name_to_uuid.insert("wave_1".into(), destroy_uuid);
    app.insert_resource(runtime);
    app.insert_resource(crate::lobby::server::ShipClientConfigResource(
        crate::core::messages::ShipClientConfig {
            helm_radar_range: 5000.0,
            ..Default::default()
        },
    ));
    set_ship_blackboard_objectives(&mut app, vec![destroy_scored_objective("wave_1", 80.0)]);

    // A separate hostile well off to starboard is the arc-bearing request
    // target — distinct from the Destroy pursuit target, so any steering
    // bias can only be attributed to the pending request.
    let bearing_uuid = uuid::Uuid::new_v4();
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(bearing_uuid.to_string()),
        crate::entities::spawner::EntityName("Bearing Contact".into()),
        Transform::from_xyz(200.0, 0.0, -1.0),
    ));
    let ship = find_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(ship)
        .insert(PendingArcBearingRequest {
            target: Some(bearing_uuid),
            // Fore bank, narrow arc, ample range: the far-starboard target is
            // in reach but well out of arc, so steering must bias to bear.
            arcs: vec![crate::core::messages::WeaponEmitterArc {
                facing_deg: 0.0,
                arc_deg: 30.0,
                range: 5000.0,
            }],
        });

    // Shipped-hull sources: coarse + both axes on AI.
    let resolver = resolver_from_shipped_hull("ship_harrow_destroyer");
    install_control_sources(&mut app, &resolver);

    tick(&mut app);

    let last = get_last_helm_input(&mut app);
    assert!(
        last.steering.abs() > 0.01,
        "ai_helm_steering owns steering on a shipped hull, so it must be the one to \
         fold in the pending arc-bearing request; operate_helm_ai must not consume \
         the request out from under it. got {last:?}"
    );
}

/// AC1: `ai_helm_thrust` writes `ThrustInput` when its own system is
/// AI-operated and the coarse helm is not.
#[test]
fn ai_helm_thrust_writes_thrust_intent() {
    let mut app = test_app();
    let anchor = "station-alpha";
    set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
    app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
    set_per_axis_helm_ai(&mut app);

    tick(&mut app);

    assert!(
        get_thrust_input(&mut app) > 0.0,
        "ai_helm_thrust must throttle up toward a Reach anchor"
    );
}

/// AC1: the fine gate is real — helm-thrust left Human means no AI write,
/// even with a live Helm objective on the blackboard.
#[test]
fn ai_helm_thrust_does_not_write_when_its_system_is_human() {
    let mut app = test_app();
    let anchor = "station-alpha";
    set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
    app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
    // Coarse helm human, helm-thrust left at its Human default.
    set_helm_control_source(&mut app, ControlSource::Human);

    tick(&mut app);

    assert_eq!(
        get_thrust_input(&mut app),
        0.0,
        "helm-thrust under human control must not be written by ai_helm_thrust"
    );
}

/// AC2: `ai_helm_steering` writes `SteeringInput`, steering toward the
/// selected waypoint. The anchor sits to the right of a ship at the origin
/// facing yaw 0, so steering must be positive.
#[test]
fn ai_helm_steering_writes_steering_intent_toward_waypoint() {
    let mut app = test_app();
    let anchor = "station-alpha";
    set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
    app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
    set_per_axis_helm_ai(&mut app);

    tick(&mut app);

    assert!(
        get_steering_input(&mut app) > 0.0,
        "ai_helm_steering must steer toward an anchor off the starboard bow"
    );
}

/// AC2: the fine gate is real for the steering axis too.
#[test]
fn ai_helm_steering_does_not_write_when_its_system_is_human() {
    let mut app = test_app();
    let anchor = "station-alpha";
    set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
    app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
    set_helm_control_source(&mut app, ControlSource::Human);

    tick(&mut app);

    assert_eq!(
        get_steering_input(&mut app),
        0.0,
        "helm-steering under human control must not be written by ai_helm_steering"
    );
}

/// The axes are genuinely independent: automating only the throttle must
/// leave steering alone, which is the whole point of the per-axis split.
#[test]
fn per_axis_gates_are_independent() {
    let mut app = test_app();
    let anchor = "station-alpha";
    set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
    app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
    set_helm_control_source(&mut app, ControlSource::Human);
    set_fine_control_source(
        &mut app,
        crate::ship::system_registry::helm_thrust_system_id(),
        ControlSource::Ai,
    );
    tick(&mut app);

    assert!(
        get_thrust_input(&mut app) > 0.0,
        "throttle axis is AI-operated → must be written"
    );
    assert_eq!(
        get_steering_input(&mut app),
        0.0,
        "steering axis is still human → must be untouched"
    );
    // The third assertion here used to be a `nav_goal` probe for "did the
    // AiMemory mutation get committed?" — the #701 commit rule's half of
    // this test. #702 made `operate_helm` pure, so there is no commit to
    // observe and no half-dead-AI failure mode to guard: a system that runs
    // computes its axis from the shared surfaces and writes it, full stop.
}

// ── #779: data-authored Engines/Steering policy spine ────────────────────

/// Merge one axis's authored policy into the ship's ONE keyed
/// [`FineSystemAiPolicies`] map (issue #1209), preserving any axes already
/// set — the test-side mirror of the single map the spawner builds. Inserting
/// a fresh single-key map instead would clobber a sibling axis a prior call
/// attached.
fn set_fine_policy(
    app: &mut App,
    ship: Entity,
    system: crate::core::messages::SystemId,
    policy: crate::ai::policy::AiPolicy,
) {
    let mut map = app
        .world()
        .get::<FineSystemAiPolicies>(ship)
        .map(|p| p.0.clone())
        .unwrap_or_default();
    map.insert(system, policy);
    app.world_mut()
        .entity_mut(ship)
        .insert(FineSystemAiPolicies(map));
}

/// Attach an authored Engines policy to the ship (overriding the spawn
/// default the hosts fall back to).
fn attach_engines_policy(app: &mut App, cfg: crate::entities::config::FineSystemAiConfigToml) {
    let ship = find_ship_entity(app);
    let policy = cfg.to_policy().expect("engines policy resolves");
    set_fine_policy(
        app,
        ship,
        crate::ship::system_registry::helm_thrust_system_id(),
        policy,
    );
}

/// Attach an authored Steering policy to the ship.
fn attach_steering_policy(app: &mut App, cfg: crate::entities::config::FineSystemAiConfigToml) {
    let ship = find_ship_entity(app);
    let policy = cfg.to_policy().expect("steering policy resolves");
    set_fine_policy(
        app,
        ship,
        crate::ship::system_registry::helm_steering_system_id(),
        policy,
    );
}

/// AC1/AC3/AC4: with the canonical default Engines *and* Steering policies
/// explicitly attached — the same policies spawn synthesises — a Reach
/// objective produces both actuator inputs and drives the ship toward its
/// destination. The DECISION to actuate now flows through the resolved mode
/// verb; the continuous magnitude still comes from the planner.
#[test]
fn authored_default_policy_actuates_travel_toward_reach_anchor() {
    let mut app = test_app();
    let anchor = "station-alpha";
    set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
    // Anchor off the starboard bow so both axes must engage.
    app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
    set_per_axis_helm_ai(&mut app);
    attach_engines_policy(
        &mut app,
        crate::entities::authored_ai_pins::shipped_policy_toml("engines"),
    );
    attach_steering_policy(
        &mut app,
        crate::entities::authored_ai_pins::shipped_policy_toml("steering"),
    );

    tick(&mut app);

    assert!(
        get_thrust_input(&mut app) > 0.0,
        "the authored Engines policy resolving `actuate_desired_travel` must emit forward SetThrust"
    );
    assert!(
        get_steering_input(&mut app) > 0.0,
        "the authored Steering policy resolving `actuate_desired_facing` must emit SetSteering toward the starboard anchor"
    );

    // AC4: the ship actually closes on its destination — several ticks of
    // forward travel build speed and move it downrange through the shared
    // actuator path (not a coarse direct write).
    let start = get_ship_physics(&mut app);
    for _ in 0..30 {
        tick(&mut app);
    }
    let end = get_ship_physics(&mut app);
    assert!(
        end.forward_speed > 0.0,
        "the ship must build forward speed under the authored policy; got {end:?}"
    );
    let moved = ((end.x - start.x).powi(2) + (end.z - start.z).powi(2)).sqrt();
    assert!(
        moved > 0.0,
        "the ship must make positional progress toward its Reach destination; \
         start=({},{}) end=({},{})",
        start.x,
        start.z,
        end.x,
        end.z
    );
}

/// AC1: the policy is a real gate, not decoration. An Engines policy whose
/// only rule never fires (`when = false`) resolves to "hold" on the
/// `longitudinal` channel, so `ai_helm_thrust` emits nothing even though the
/// Reach objective and the planner both want forward travel — while an
/// unchanged default Steering policy still turns the ship. This is the seam
/// #794 will exploit to retire the hardcoded planner branch.
#[test]
fn engines_policy_that_never_fires_holds_thrust_but_not_steering() {
    let mut app = test_app();
    let anchor = "station-alpha";
    set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
    app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
    set_per_axis_helm_ai(&mut app);

    let hold = crate::entities::config::FineSystemAiConfigToml {
        evaluate_every_ticks: crate::entities::config::default_evaluate_every_ticks(),
        idle: false,
        param: Default::default(),
        rule: vec![crate::entities::config::FineSystemAiRuleToml {
            priority: 0,
            channel: crate::entities::config::HELM_LONGITUDINAL_CHANNEL.into(),
            when: "false".into(),
            verb: crate::entities::config::HELM_ACTUATE_DESIRED_TRAVEL_VERB.into(),
            value: false,
            level: 0,
            response_index: 0,
        }],
        initial_state: None,
        state: Vec::new(),
        memory: std::collections::HashMap::new(),
    };
    attach_engines_policy(&mut app, hold);

    tick(&mut app);

    assert_eq!(
        get_thrust_input(&mut app),
        0.0,
        "an Engines policy whose guard never fires must hold thrust: no SetThrust emitted"
    );
    assert!(
        get_steering_input(&mut app) > 0.0,
        "Steering is independently authored and still actuates — the two systems are separable"
    );
}

/// AC1 mirror on the yaw axis: an explicit-idle Steering policy holds the
/// facing while the default Engines policy still throttles.
#[test]
fn idle_steering_policy_holds_yaw_but_not_thrust() {
    let mut app = test_app();
    let anchor = "station-alpha";
    set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
    app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
    set_per_axis_helm_ai(&mut app);
    attach_steering_policy(
        &mut app,
        crate::entities::config::FineSystemAiConfigToml {
            idle: true,
            ..Default::default()
        },
    );

    tick(&mut app);

    assert_eq!(
        get_steering_input(&mut app),
        0.0,
        "an idle Steering policy resolves no verb → yaw holds, no SetSteering emitted"
    );
    assert!(
        get_thrust_input(&mut app) > 0.0,
        "the default Engines policy still actuates travel"
    );
}

/// AC6: human takeover preserves input authority, and Backfill reacquisition
/// restores AI actuation without any lifecycle carry-over (the policy is
/// stateless, so reacquisition is a clean resolve, not a resumed machine).
/// Under the same authored default policy throughout, the emit tracks the
/// per-axis control source tick to tick.
#[test]
fn human_takeover_and_backfill_reacquisition_track_input_authority() {
    let mut app = test_app();
    let anchor = "station-alpha";
    set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
    app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
    attach_engines_policy(
        &mut app,
        crate::entities::authored_ai_pins::shipped_policy_toml("engines"),
    );
    attach_steering_policy(
        &mut app,
        crate::entities::authored_ai_pins::shipped_policy_toml("steering"),
    );

    // Backfill: both axes AI → the policy actuates.
    set_per_axis_helm_ai(&mut app);
    tick(&mut app);
    assert!(
        get_thrust_input(&mut app) > 0.0 && get_steering_input(&mut app) > 0.0,
        "AI-operated axes under the authored policy must actuate"
    );

    // Human takeover: both axes handed back to a human. The AI hosts must
    // not write the intent — input authority is the human's.
    set_helm_control_source(&mut app, ControlSource::Human);
    // The intent components retain their last value; zero them so a stale
    // read cannot masquerade as a fresh AI write, then confirm the AI leaves
    // them at zero.
    let ship = find_ship_entity(&mut app);
    app.world_mut().entity_mut(ship).insert((
        crate::ship::helm::ThrustInput::default(),
        crate::ship::helm::SteeringInput::default(),
    ));
    tick(&mut app);
    assert_eq!(
        get_thrust_input(&mut app),
        0.0,
        "under human takeover the AI Engines host must not write thrust"
    );
    assert_eq!(
        get_steering_input(&mut app),
        0.0,
        "under human takeover the AI Steering host must not write yaw"
    );

    // Backfill reacquisition: hand the axes back to AI. The stateless policy
    // resolves cleanly and actuation resumes the same tick — no reset needed.
    set_per_axis_helm_ai(&mut app);
    tick(&mut app);
    assert!(
        get_thrust_input(&mut app) > 0.0 && get_steering_input(&mut app) > 0.0,
        "reacquired AI axes must re-actuate immediately under the same stateless policy"
    );
}

/// Regression (issue #701 review, finding 1): `ai_helm_thrust` and
/// `ai_helm_steering` write one `LastHelmInput` field each, and
/// `publish_joystick_to_engines` reads both as a pair. Unless it is ordered
/// after *both* writers it can interleave between them and publish this
/// tick's AI throttle next to the stale human steering still sitting in
/// `LastHelmInput` — a torn pair that lands in `HelmEngineBlackboard`, i.e.
/// on the player's engine gauge. Which half tears is decided by Bevy's
/// arbitrary intra-set order, so this pins the published pair against the
/// stale value rather than against a lucky schedule.
#[test]
fn helm_ai_last_input_pair_is_not_torn() {
    let mut app = test_app();
    let anchor = "station-alpha";
    set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
    // Off the starboard bow → the AI wants positive thrust AND positive
    // steering, so both differ in sign from the stale human stick below.
    app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
    set_per_axis_helm_ai(&mut app);
    // Stale human stick, hard astern and hard to port, left over from
    // before the axes were handed to the AI.
    set_last_helm_input(
        &mut app,
        LastHelmInput {
            thrust: -0.9,
            steering: -0.9,
            lateral: 0.0,
        },
    );

    tick(&mut app);

    let ai_thrust = get_thrust_input(&mut app);
    let ai_steering = get_steering_input(&mut app);
    assert!(
        ai_thrust > 0.0 && ai_steering > 0.0,
        "precondition: the AI must actually want to move, else there is no \
         stale value to tear against; got thrust={ai_thrust} steering={ai_steering}"
    );

    let queue = app.world().resource::<InterSystemQueue>();
    let port_id = crate::ship::system_registry::helm_engine_port_system_id();
    let msgs: Vec<_> = queue.for_target(port_id.0.as_str()).collect();
    assert!(
        !msgs.is_empty(),
        "expected a JoystickState message for helm-engine-port"
    );

    for msg in &msgs {
        let InterSystemPayload::JoystickState { thrust, steering } = &msg.payload else {
            panic!("expected JoystickState payload");
        };
        assert_eq!(
            (*thrust, *steering),
            (ai_thrust, ai_steering),
            "published joystick pair must be the AI's whole decision. A \
             mismatch on one axis only means the pair tore: \
             publish_joystick_to_engines interleaved between ai_helm_thrust \
             and ai_helm_steering and picked up the stale human -0.9"
        );
    }
}

/// AC3 (Retreat consumer): with a Retreat directive top-scored, steering
/// must resolve the named anchor and steer toward it. `operate_helm`'s
/// Retreat arm is what does the work — this pins that `ai_helm_steering`
/// actually routes it through to `SteeringInput`.
#[test]
fn ai_helm_steering_retreats_toward_anchor() {
    let mut app = test_app();
    let anchor = "rally-point";
    set_ship_blackboard_objectives(&mut app, vec![retreat_scored_objective(anchor, 90.0)]);
    // Rally point off the starboard bow → positive steering.
    app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
    set_per_axis_helm_ai(&mut app);

    tick(&mut app);

    assert!(
        get_steering_input(&mut app) > 0.0,
        "Retreat must steer toward the named rally anchor"
    );
    assert!(
        get_thrust_input(&mut app) > 0.0,
        "Retreat must also throttle up to actually leave"
    );
}

/// AC3 (Retreat consumer, unresolvable case): a Retreat naming an anchor
/// the world does not declare resolves to nowhere and leaves the ship idle.
///
/// This asserted the opposite until #702: an *empty*-anchor Retreat — which
/// is what `aggregate_doctrine_blackboards` synthesised below a
/// `[behaviour] retreat_hull_threshold` — used to fall back to the ship's
/// `AiMemory.home_position`. Both the injector and `home_position` are gone.
/// The fallback only ever looked like a safety net: `home_position` was
/// never seeded in production, so "retreat home" meant "fly to world
/// origin" on every shipped ship. Retreat is authored doctrine with a real
/// anchor now (`assets/worlds/patrol.toml` authors one on `raider_alpha`), and an anchor that
/// resolves to nothing steers nowhere — see
/// `ai_helm_steering_retreats_toward_anchor` for the resolvable case.
#[test]
fn ai_helm_steering_retreat_with_unknown_anchor_does_not_steer() {
    let mut app = test_app();
    // No anchors in the world config → the Retreat cannot resolve, and
    // there is no lower-priority objective to fall through to.
    set_ship_blackboard_objectives(&mut app, vec![retreat_scored_objective("", 90.0)]);
    app.insert_resource(crate::world::config::WorldConfig::default());
    set_per_axis_helm_ai(&mut app);

    tick(&mut app);

    assert_eq!(
        get_steering_input(&mut app),
        0.0,
        "a Retreat that names nowhere must not steer; the old home_position \
         fallback made this a flight to world origin"
    );
    assert_eq!(
        get_thrust_input(&mut app),
        0.0,
        "and must not throttle up either"
    );
}

/// A top-scored Retreat wins over a lower-scored Helm objective pointing
/// the other way, so the ship actually breaks off rather than pressing on.
///
/// The pool is listed descending by score because that is the contract
/// every producer honours (`score_doctrine_pool` and
/// `ObjectiveManager::scored_pool_with_boost` both sort before publishing)
/// and what `operate_helm` consumes — it takes the first Helm-relevant
/// entry rather than scanning for the maximum.
#[test]
fn ai_helm_steering_retreat_outranks_lower_priority_objective() {
    let mut app = test_app();
    let mut cfg = crate::world::config::WorldConfig::default();
    // Rally to starboard, patrol waypoint to port.
    cfg.anchors.insert("rally".into(), [100.0, 0.0, 0.0]);
    cfg.anchors.insert("wp".into(), [-100.0, 0.0, 0.0]);
    app.insert_resource(cfg);
    set_ship_blackboard_objectives(
        &mut app,
        vec![
            retreat_scored_objective("rally", 90.0),
            patrol_scored_objective(vec!["wp"], 10.0),
        ],
    );
    set_per_axis_helm_ai(&mut app);

    tick(&mut app);

    assert!(
        get_steering_input(&mut app) > 0.0,
        "top-scored Retreat must win over the lower-scored patrol waypoint"
    );
}

/// AC4: both per-axis systems are `AiHighFidelity`-scoped. A demoted ship
/// (marker removed) must not be driven by them.
#[test]
fn per_axis_helm_ai_is_scoped_to_high_fidelity() {
    let mut app = test_app();
    let anchor = "station-alpha";
    set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
    app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
    set_per_axis_helm_ai(&mut app);

    // Demote: drop the marker, keep the intent components so a write would
    // still be observable if the scoping were missing.
    let ship = find_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(ship)
        .remove::<crate::ai::server::AiHighFidelity>();

    tick(&mut app);

    assert_eq!(
        get_thrust_input(&mut app),
        0.0,
        "ai_helm_thrust must not touch a ship without AiHighFidelity"
    );
    assert_eq!(
        get_steering_input(&mut app),
        0.0,
        "ai_helm_steering must not touch a ship without AiHighFidelity"
    );
}

// ── Per-axis helm AI: impulse (issue #703) ─────────────────────────────

/// AC1: `ai_helm_impulse` writes `ImpulseCommand`, gating on helm-impulse
/// alone. The anchor is dead ahead down -Z at 500 units — past the
/// 200-unit `engage_distance` and inside the angle tolerance — so the
/// decision is `Engage`.
#[test]
fn ai_helm_impulse_engages_toward_a_distant_target_ahead() {
    let anchor = "station-alpha";
    let mut app = impulse_ai_app(reach_scored_objective(anchor, 10.0));
    app.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, -500.0]));

    tick(&mut app);

    assert_eq!(
        get_impulse_command(&mut app),
        crate::ship::impulse::ImpulsePhase::Charging,
        "ai_helm_impulse must command a charge toward a distant anchor dead ahead"
    );
}

/// AC1: the gate is real. Identical geometry to the test above, but
/// helm-impulse is left at its Human default — and the coarse helm is
/// Human too, so nothing may command the drive.
#[test]
fn ai_helm_impulse_does_not_write_when_its_system_is_human() {
    let anchor = "station-alpha";
    let mut app = impulse_ai_app(reach_scored_objective(anchor, 10.0));
    app.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, -500.0]));
    set_fine_control_source(
        &mut app,
        crate::ship::system_registry::helm_impulse_system_id(),
        ControlSource::Human,
    );

    tick(&mut app);

    assert_eq!(
        get_impulse_command(&mut app),
        crate::ship::impulse::ImpulsePhase::Idle,
        "helm-impulse under human control must not be commanded by ai_helm_impulse"
    );
}

/// AC1, the deactivate half: inside `cancel_distance` with a charge already
/// running, `ai_helm_impulse` must stand the drive down. The command starts
/// at `Charging`, so `Idle` here is an observed write and not the default.
#[test]
fn ai_helm_impulse_cancels_when_the_target_is_close() {
    let anchor = "station-alpha";
    let mut app = impulse_ai_app(reach_scored_objective(anchor, 10.0));
    // 20 units out — inside the 40-unit `cancel_distance`.
    app.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, -20.0]));
    // `decide_impulse` only cancels from a non-Idle phase.
    let mut state = crate::ship::impulse::ImpulseState::new();
    state.start_charge();
    set_ship_impulse(&mut app, state);
    set_impulse_command(&mut app, crate::ship::impulse::ImpulsePhase::Charging);

    tick(&mut app);

    assert_eq!(
        get_impulse_command(&mut app),
        crate::ship::impulse::ImpulsePhase::Idle,
        "ai_helm_impulse must cancel the charge once the target is inside \
         cancel_distance; still Charging means it never wrote"
    );
}

/// AC3: `ai_helm_impulse` is `AiHighFidelity`-scoped. The demoted ship keeps
/// its `ImpulseCommand` here only so a stray write would be observable.
#[test]
fn ai_helm_impulse_is_scoped_to_high_fidelity() {
    let anchor = "station-alpha";
    let mut app = impulse_ai_app(reach_scored_objective(anchor, 10.0));
    app.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, -500.0]));

    let ship = find_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(ship)
        .remove::<crate::ai::server::AiHighFidelity>();

    tick(&mut app);

    assert_eq!(
        get_impulse_command(&mut app),
        crate::ship::impulse::ImpulsePhase::Idle,
        "ai_helm_impulse must not touch a ship without AiHighFidelity"
    );
}

/// A live Helm objective is a precondition: `operate_helm_ai`'s
/// no-objective branch `continue`d before its impulse block rather than
/// cancelling, and `ai_helm_impulse` inherited that when #703 extracted it —
/// a behaviour it now carries alone, the monolith having been deleted in
/// #704. A lull in objectives is not a reason to drop an in-progress
/// charge.
///
/// Pins the *behaviour*, not any one line. `ai_helm_impulse` enforces it
/// three times over — the `has_helm_objective` early-out,
/// `resolve_helm_target_position`'s top-objective filter, and the `top_obj`
/// lookup behind `use_impulse` — each carrying the same `score > 0.0 &&
/// Helm`-relevant predicate. Deleting any one or two of them leaves this
/// green; only losing all three turns it red. That is a statement about the
/// implementation being belt-and-braces, not about the test being weak: the
/// behaviour it asserts (a dead objective must not cancel a live charge) is
/// the thing that matters, and it is unreachable by any single regression.
#[test]
fn ai_helm_impulse_leaves_the_drive_alone_without_a_helm_objective() {
    let anchor = "station-alpha";
    let mut app = impulse_ai_app(reach_scored_objective(anchor, 10.0));
    // Inside cancel_distance with a charge running: the one geometry where
    // a system that ignored the objective gate would visibly cancel.
    app.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, -20.0]));
    let mut state = crate::ship::impulse::ImpulseState::new();
    state.start_charge();
    set_ship_impulse(&mut app, state);
    set_impulse_command(&mut app, crate::ship::impulse::ImpulsePhase::Charging);
    // Same objective, scored dead: `has_helm_objective` requires score > 0.
    set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 0.0)]);

    tick(&mut app);

    assert_eq!(
        get_impulse_command(&mut app),
        crate::ship::impulse::ImpulsePhase::Charging,
        "with no live Helm objective ai_helm_impulse must leave ImpulseCommand \
         untouched, as the monolith does"
    );
}

/// `use_impulse` is TOML-authored per doctrine entry (AGENTS.md rule 11):
/// an objective whose doctrine forbids impulse must not engage it, however
/// inviting the geometry.
#[test]
fn ai_helm_impulse_honours_toml_authored_use_impulse() {
    let anchor = "station-alpha";
    let mut app = impulse_ai_app(reach_scored_objective(anchor, 10.0));
    app.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, -500.0]));
    // The same doctrine entry `impulse_ai_app` installs, with the one
    // authored field flipped.
    set_behaviour_section(
        &mut app,
        crate::entities::config::BehaviourConfig {
            doctrine: vec![crate::entities::config::DoctrineObjective {
                id: "reach-station-alpha".into(),
                use_impulse: Some(false),
                target_speed: 0.8,
                maintain_range: 25.0,
                ..Default::default()
            }],
            ..Default::default()
        },
    );

    tick(&mut app);

    assert_eq!(
        get_impulse_command(&mut app),
        crate::ship::impulse::ImpulsePhase::Idle,
        "[[behaviour.doctrine]] use_impulse = false must veto the engage that \
         ai_helm_impulse_engages_toward_a_distant_target_ahead proves is otherwise \
         reachable from this geometry"
    );
}

/// `ai_helm_impulse` must resolve its target from the *same* waypoint the
/// rest of the helm AI is steering at this tick — one leg further on than the
/// tick started, because `advance_objective_cursors` (`SimSet::Modifiers`)
/// runs before this system and has already advanced the cursor off the
/// waypoint underfoot.
///
/// The name is historical, and so is the failure it guards: this system used
/// to reach that leg by *replaying* the helm decision on a scratch clone of
/// `AiMemory`, which only matched the committer's view while the memory was
/// still pre-commit — hence `.before(operate_helm_ai)`. #702 deleted
/// `AiMemory` and with it the clone, the replay and the commit; the cursor is
/// now a read-only surface that cannot move underneath this system at all
/// (see the registration comment on `ai_helm_impulse`). What is left to pin
/// is the answer, not the mechanism that reached it.
///
/// The patrol makes the leg observable. wp-a and wp-b both sit on the ship,
/// so the cursor advances off wp-a during `Modifiers`; wp-c is 500 units dead
/// ahead.
///
///   correct → cursor 1 → target wp-b underfoot → inside cancel_distance
///             with a charge running → **Cancel**
///   broken  → a leg out of step → target wp-c at 500 → far → NoChange →
///             command stays `Charging`
///
/// So the correct answer is also the one that performs a write, which keeps
/// a do-nothing regression from passing this too.
#[test]
fn ai_helm_impulse_reads_pre_commit_memory() {
    let mut app = impulse_ai_app(patrol_scored_objective(vec!["wp-a", "wp-b", "wp-c"], 20.0));
    let mut cfg = crate::world::config::WorldConfig::default();
    cfg.anchors.insert("wp-a".into(), [0.0, 0.0, 0.0]);
    cfg.anchors.insert("wp-b".into(), [0.0, 0.0, 0.0]);
    cfg.anchors.insert("wp-c".into(), [0.0, 0.0, -500.0]);
    app.insert_resource(cfg);
    set_behaviour_section(&mut app, impulse_doctrine("obj-defend"));
    // Coarse helm on AI, as this test has always run it. (This was once
    // load-bearing: it put `operate_helm_ai` in the tick as the committer
    // this system had to run ahead of. There is no committer now.)
    set_helm_control_source(&mut app, ControlSource::Ai);
    let mut state = crate::ship::impulse::ImpulseState::new();
    state.start_charge();
    set_ship_impulse(&mut app, state);
    set_impulse_command(&mut app, crate::ship::impulse::ImpulsePhase::Charging);

    tick(&mut app);

    assert_eq!(
        get_impulse_command(&mut app),
        crate::ship::impulse::ImpulsePhase::Idle,
        "ai_helm_impulse must resolve its target from this tick's advance (wp-b, \
         underfoot) — still Charging means it replayed the decision on memory \
         operate_helm_ai had already committed and skipped a leg to wp-c"
    );
}

/// The coverage gap #800 was caught by, applied to impulse: every test above
/// hand-builds its control sources, so all of them would pass with
/// `helm-impulse` declared in zero TOMLs. This one refuses to hand-build.
///
/// `alliance_cruiser` declares the coarse helm *and* helm-impulse, so with
/// the station unmanned the monolith stands down from the impulse decision
/// and a `Charging` command has nowhere else to come from.
#[test]
fn shipped_hull_config_drives_ai_helm_impulse() {
    let resolver = resolver_from_shipped_hull("alliance_cruiser");
    assert!(
        resolver
            .policy_for(&crate::ship::system_registry::helm_impulse_system_id())
            .operate_ai,
        "the shipped hull must declare helm-impulse, or ai_helm_impulse is dormant \
         in shipped content"
    );
    // #801: the shipped hull no longer declares a coarse helm at all.

    let anchor = "station-alpha";
    let mut app = impulse_ai_app(reach_scored_objective(anchor, 10.0));
    app.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, -500.0]));
    install_control_sources(&mut app, &resolver);

    tick(&mut app);

    assert_eq!(
        get_impulse_command(&mut app),
        crate::ship::impulse::ImpulsePhase::Charging,
        "ai_helm_impulse must drive a shipped hull's impulse decision \
         (operate_helm_ai stands down from it here)"
    );
}

// ── Per-axis helm AI: lateral thrust (issue #703) ──────────────────────

/// An obstacle the default avoidance tuning ignores and an authored 60-unit
/// `avoidance_buffer` treats as a threat (radius 0 + 1 + 60 = 61 > 40), on a
/// stationary ship so the look-ahead cannot also move. Any nonzero lateral
/// is this obstacle and nothing else.
fn lateral_dodge_app() -> App {
    let mut app = lateral_thrust_ai_app(Some(crate::entities::config::BehaviourConfig {
        avoidance_buffer: 60.0,
        ..Default::default()
    }));
    snapshot_with_obstacle(&mut app, [4.0, 0.0, -40.0], 1.0);
    app
}

/// AC2, the gate collapse itself. `ai_helm_lateral_thrust`'s old `L && !C`
/// gate stood the system down whenever the coarse helm was AI, because the
/// monolith owned `LateralThrustInput` outright in that case. Since #703 the
/// monolith stands down instead — so if the `!C` half had been left in
/// place, this configuration would leave the dodge with **no writer at all**
/// rather than two.
///
/// That asymmetry is what this test exploits: a nonzero lateral proves the
/// half came off. (It cannot distinguish one writer from two — both compute
/// the identical dodge from identical inputs — which is what
/// `helm_writers_are_invariant_under_coarse_policy` is for.)
#[test]
fn ai_helm_lateral_thrust_dodges_when_the_coarse_helm_is_also_ai() {
    let mut app = lateral_dodge_app();
    set_helm_control_source(&mut app, ControlSource::Ai);

    tick_twice(&mut app);

    assert!(
        lateral_intent(&mut app).abs() > 0.0,
        "with helm-lateral-thrust on AI the dodge must be written whatever the \
         coarse helm is doing; zero means the collapsed gate stood the system \
         down and the monolith had already stood down too"
    );
}

/// AC3, and a behaviour change #697 declined to make:
/// `ai_helm_lateral_thrust` is now `AiHighFidelity`-scoped like its two
/// siblings. The coarse helm stays Human, so the monolith (also scoped)
/// cannot cover for it.
#[test]
fn ai_helm_lateral_thrust_is_scoped_to_high_fidelity() {
    let mut app = lateral_dodge_app();
    let ship = find_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(ship)
        .remove::<crate::ai::server::AiHighFidelity>();

    tick_twice(&mut app);

    assert_eq!(
        lateral_intent(&mut app),
        0.0,
        "ai_helm_lateral_thrust must not touch a ship without AiHighFidelity"
    );
}

/// The monolith zeroes `LateralThrustInput` when no Helm objective is live,
/// so the shared integrator decelerates the dodge off through the normal
/// physics curve. #697 `continue`d instead, latching the last dodge forever.
/// That divergence had to close before the monolith could stand down.
#[test]
fn ai_helm_lateral_thrust_zeroes_the_dodge_without_a_helm_objective() {
    let mut app = lateral_dodge_app();
    tick_twice(&mut app);
    assert!(
        lateral_intent(&mut app).abs() > 0.0,
        "precondition: the obstacle produces a dodge while an objective is live"
    );

    // Objectives go quiet; the obstacle does not move.
    set_ship_blackboard_objectives(&mut app, vec![]);
    tick(&mut app);

    assert_eq!(
        lateral_intent(&mut app),
        0.0,
        "no live Helm objective must zero the dodge, not latch the last one"
    );
}

fn set_lateral_intent(app: &mut App, value: f32) {
    let ship = find_ship_entity(app);
    app.world_mut()
        .entity_mut(ship)
        .get_mut::<LateralThrustInput>()
        .expect("ship must carry LateralThrustInput")
        .0 = value;
}

/// A sentinel the helm-AI maths can never produce — intents are
/// normalised to [-1, 1] — so a frame that leaves the sentinel standing
/// is a frame the probed system did not run on.
const CADENCE_SENTINEL: f32 = 123.456;

/// Drive `app` at 10 ms per frame — under the 33.3 ms shared sim-tick
/// period, i.e. what a 60 Hz rAF-driven host actually does — and count
/// the frames on which the probed system ran. `arm` re-stamps the probe
/// before each frame; `ran_this_frame` reads it back after.
///
/// The shared AI-helm sim tick (issue #803) is a real fixed-rate
/// throttle, not a formality. Production `Update` is rAF-driven:
/// `server/bridge.rs` installs `WinitSettings` with
/// `UpdateMode::Continuous` for both focused and unfocused, so a 60 Hz
/// host frames at ~16.7 ms — under the period — and the helm AI must
/// recompute on only *some* frames. Without the gate the AI's decision
/// cadence would follow the host's display refresh rate (a 144 Hz host
/// deciding on ~4x fresher data than a 60 Hz one), which is exactly the
/// nondeterminism PRD #620's lockstep has to eliminate. Until #803 only
/// the lateral axis was throttled (by the private `AiLateralThrustTimer`);
/// all four per-axis systems now share one cadence, and there is one of
/// these tests per system.
fn count_sim_tick_runs(
    app: &mut App,
    mut arm: impl FnMut(&mut App),
    mut ran_this_frame: impl FnMut(&mut App) -> bool,
) -> (usize, usize) {
    const FRAME_MS: u64 = 10;
    const TICKS: usize = 12;
    // Since #895 the sim advances in `FixedUpdate`, so "the AI must not
    // decide every rendered frame" is enforced by the fixed loop itself:
    // pin the logical timestep to the shipped 33.3 ms decision period
    // (`GlobalConfig::default().ai_tick_hz`; without a `WorldConfig` every
    // fixed step is a decision tick) and drive frames at 10 ms, so only
    // the frames that accumulate a whole step can decide.
    //
    // The callers below build `app` via `test_app()` and never tick it
    // before reaching here, so `test_app()`'s `TEST_TICK` (200 ms)
    // preload is still sitting untouched in `Time<Fixed>`'s accumulator.
    // Discard it before re-pacing to the fine 10 ms frame, or this
    // function's first `tick()` bursts ~6 steps (200 ms / 33.3 ms) instead
    // of the 0-or-1 the throttle assertions below assume.
    crate::ship::test_support::discard_stale_overstep(app);
    app.world_mut()
        .resource_mut::<Time<Fixed>>()
        .set_timestep(crate::sim_tick::sim_tick_period(
            crate::entities::config::GlobalConfig::default().ai_tick_hz,
        ));
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_millis(FRAME_MS),
    ));
    let mut ran = 0usize;
    for _ in 0..TICKS {
        arm(app);
        tick(app);
        if ran_this_frame(app) {
            ran += 1;
        }
    }
    (ran, TICKS)
}

/// Shared assertions for the four cadence tests. `ran > 0` guards the
/// probe itself; `ran <= ticks / 2` is the throttle. Over 12 frames x
/// 10 ms the 33.3 ms timer fires ~3 times, plus the first frame's
/// `AiTickReady`-initialises-`true` free run (mirroring
/// `AiSnapshotReady`); `ticks / 2` leaves generous margin while still
/// failing loudly if the gate goes away.
fn assert_shared_sim_tick_cadence(system: &str, (ran, ticks): (usize, usize)) {
    assert!(
        ran > 0,
        "precondition: {ticks} frames x 10 ms spans several 33.3 ms periods, so \
         {system} must run at least once — 0 runs means the probe is broken and \
         this test proves nothing about cadence"
    );
    assert!(
        ran <= ticks / 2,
        "the shared AI-helm sim tick must throttle {system}: at 10 ms/frame — \
         under the 33.3 ms period, i.e. what a 60 Hz rAF-driven host actually \
         does — it ran on {ran} of {ticks} frames. Running every frame means the \
         run_if(ai_tick_ready) gate is gone and the decision cadence \
         follows display refresh rate again (PRD #620)"
    );
}

fn set_thrust_intent(app: &mut App, value: f32) {
    let ship = find_ship_entity(app);
    app.world_mut()
        .entity_mut(ship)
        .get_mut::<ThrustInput>()
        .expect("ship must carry ThrustInput")
        .0 = value;
}

fn set_steering_intent(app: &mut App, value: f32) {
    let ship = find_ship_entity(app);
    app.world_mut()
        .entity_mut(ship)
        .get_mut::<SteeringInput>()
        .expect("ship must carry SteeringInput")
        .0 = value;
}

/// Since #824 `process_helm_inputs` writes `LateralThrustInput` only when
/// an admitted `LateralThrustInput` command exists for the ship — and the
/// only emitter here is `ai_helm_lateral_thrust` itself — so a frame that
/// clears the sentinel is a frame the AI decided on. (`HelmInputTimer`
/// and the coarse-AI stand-down this comment used to describe are gone;
/// the coarse-AI setup is kept purely as the historical fixture shape.)
#[test]
fn ai_helm_lateral_thrust_runs_on_the_shared_sim_tick_not_per_frame() {
    let mut app = lateral_dodge_app();
    set_helm_control_source(&mut app, ControlSource::Ai);

    let counts = count_sim_tick_runs(
        &mut app,
        |app| set_lateral_intent(app, CADENCE_SENTINEL),
        |app| lateral_intent(app) != CADENCE_SENTINEL,
    );
    assert_shared_sim_tick_cadence("ai_helm_lateral_thrust", counts);
}

/// AC (issue #803): `ai_helm_thrust` used to run once per rendered frame;
/// it must now run on the shared sim tick. `set_per_axis_helm_ai` puts the
/// thrust axis on AI, so `process_helm_inputs` skips the axis and this
/// system is `ThrustInput`'s sole writer — the sentinel can only be
/// cleared by it.
#[test]
fn ai_helm_thrust_runs_on_the_shared_sim_tick_not_per_frame() {
    let mut app = test_app();
    let anchor = "station-alpha";
    set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
    app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
    set_per_axis_helm_ai(&mut app);

    let counts = count_sim_tick_runs(
        &mut app,
        |app| set_thrust_intent(app, CADENCE_SENTINEL),
        |app| get_thrust_input(app) != CADENCE_SENTINEL,
    );
    assert_shared_sim_tick_cadence("ai_helm_thrust", counts);
}

/// AC (issue #803): `ai_helm_steering` on the shared sim tick — same
/// isolation argument as the thrust test.
#[test]
fn ai_helm_steering_runs_on_the_shared_sim_tick_not_per_frame() {
    let mut app = test_app();
    let anchor = "station-alpha";
    set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
    app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
    set_per_axis_helm_ai(&mut app);

    let counts = count_sim_tick_runs(
        &mut app,
        |app| set_steering_intent(app, CADENCE_SENTINEL),
        |app| get_steering_input(app) != CADENCE_SENTINEL,
    );
    assert_shared_sim_tick_cadence("ai_helm_steering", counts);
}

/// AC (issue #803): `ai_helm_impulse` on the shared sim tick.
/// `ImpulseCommand` is an enum, so the probe is a reset-and-observe
/// rather than a sentinel: each frame re-arms the drive to `Idle` (both
/// the command and the `ShipImpulse` phase, so `decide_impulse` sees the
/// same Engage-able geometry every time — the anchor 500 units dead
/// ahead, past `engage_distance`); a frame that ends `Charging` is a
/// frame the system ran on.
#[test]
fn ai_helm_impulse_runs_on_the_shared_sim_tick_not_per_frame() {
    let anchor = "station-alpha";
    let mut app = impulse_ai_app(reach_scored_objective(anchor, 10.0));
    app.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, -500.0]));

    let counts = count_sim_tick_runs(
        &mut app,
        |app| {
            set_ship_impulse(app, crate::ship::impulse::ImpulseState::new());
            set_impulse_command(app, crate::ship::impulse::ImpulsePhase::Idle);
        },
        |app| get_impulse_command(app) == crate::ship::impulse::ImpulsePhase::Charging,
    );
    assert_shared_sim_tick_cadence("ai_helm_impulse", counts);
}

/// The shared decision cadence is TOML-authored, not hardcoded: with a
/// loaded `WorldConfig`, `tick_ai_cadence` derives it as
/// `sim_tick_hz / ai_tick_hz` logical ticks per decision (issue #895).
/// Authoring the two rates EQUAL makes every fixed step a decision tick,
/// so with the timestep pinned to the 10 ms frame period the dodge
/// recomputes every frame — where the shipped 2:1 ratio (the default
/// `WorldConfig`) would allow at most half.
#[test]
fn ai_helm_tick_rate_is_reconfigured_from_world_config() {
    let mut app = lateral_dodge_app();
    set_helm_control_source(&mut app, ControlSource::Ai);
    let mut cfg = crate::world::config::WorldConfig::default();
    cfg.global.sim_tick_hz = 100.0;
    cfg.global.ai_tick_hz = 100.0;
    cfg.global.ai_snapshot_hz = 100.0;
    // `lateral_dodge_app` leaves no WorldConfig installed; the dodge only
    // needs the snapshot obstacle, so the empty-anchor config is inert
    // apart from the authored cadence.
    app.insert_resource(cfg);
    // Pin the logical timestep to the frame period: one step per frame,
    // so decisions-per-frame is exactly the authored per-tick cadence.
    // (`reconcile_fixed_timestep` is a production registration; fixtures
    // own their timestep, so it is set directly here.)
    crate::ship::test_support::discard_stale_overstep(&mut app);
    app.world_mut()
        .resource_mut::<Time<Fixed>>()
        .set_timestep(std::time::Duration::from_millis(10));
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_millis(10),
    ));

    let mut ran = 0usize;
    const TICKS: usize = 12;
    for _ in 0..TICKS {
        set_lateral_intent(&mut app, CADENCE_SENTINEL);
        tick(&mut app);
        if lateral_intent(&mut app) != CADENCE_SENTINEL {
            ran += 1;
        }
    }
    assert!(
        ran > TICKS / 2,
        "with sim_tick_hz == ai_tick_hz every logical tick is a decision \
         tick, so the dodge must recompute on (nearly) every frame — {ran} \
         of {TICKS} means tick_ai_cadence never applied the TOML-authored \
         cadence"
    );
}

/// The #800 coverage gap, applied to lateral thrust. `alliance_cruiser`
/// declares the coarse helm *and* helm-lateral-thrust, so an unmanned Helm
/// puts both on AI — the exact combination the old `!C` half made
/// unreachable, and the one every hand-built test above misses.
#[test]
fn shipped_hull_config_drives_ai_helm_lateral_thrust() {
    let resolver = resolver_from_shipped_hull("alliance_cruiser");
    assert!(
        resolver
            .policy_for(&crate::ship::system_registry::lateral_thrust_system_id())
            .operate_ai,
        "the shipped hull must declare helm-lateral-thrust, or ai_helm_lateral_thrust \
         is dormant in shipped content"
    );
    // #801: the shipped hull no longer declares a coarse helm at all.

    let mut app = lateral_dodge_app();
    install_control_sources(&mut app, &resolver);

    tick_twice(&mut app);

    assert!(
        lateral_intent(&mut app).abs() > 0.0,
        "ai_helm_lateral_thrust must drive a shipped hull's dodge (since #704 it is \
         the lateral axis's only AI writer)"
    );
}

/// Pins the per-axis gate algebra: **the coarse helm policy `C` is not an
/// input to any intent writer.** Each writer is a function of its own axis
/// alone — this test sweeps C across all three control sources for every
/// fixed (T,S,L,I) and demands the whole outcome (every component's
/// writer) be invariant under it. It also pins the coverage half: each
/// component is written exactly when its own axis is AI.
///
/// This is a **model** test: it states the gate algebra against the policy
/// resolver, it does not run the systems. A coarse fallback re-introduced
/// into `ai_helm_thrust` leaves this test green; what catches that is
/// `coarse_helm_alone_drives_no_intent_but_the_per_axis_systems_do` and
/// its siblings, which exercise the real systems. Read this test as the
/// specification and those as the enforcement.
#[test]
fn helm_writers_are_invariant_under_coarse_policy() {
    use crate::ship::control_source::{ControlSource, ControlSourceResolver};

    // #801: "helm" is not a system; seeding it (the C dimension) must
    // have no influence on any writer — which is what this test proves.
    let coarse =
        crate::core::messages::SystemId(crate::ship::system_registry::HELM_STATION_ID.to_string());
    let thrust = crate::ship::system_registry::helm_thrust_system_id();
    let steering = crate::ship::system_registry::helm_steering_system_id();
    let lateral = crate::ship::system_registry::lateral_thrust_system_id();
    let impulse = crate::ship::system_registry::helm_impulse_system_id();

    let all = [
        ControlSource::Human,
        ControlSource::Ai,
        ControlSource::Offline,
    ];

    // Every writer decision for one ship in one tick: which system writes
    // each intent component.
    #[derive(Debug, PartialEq, Eq)]
    struct HelmWriters {
        thrust: bool,
        steering: bool,
        lateral: bool,
        impulse: bool,
    }

    let mut saw_all_four_running = false;

    for t in all {
        for s in all {
            for l in all {
                for i in all {
                    // Sweep the coarse source innermost so that, for one
                    // fixed (T,S,L,I), we can compare the outcome across all
                    // three coarse sources and demand they agree.
                    let mut outcome_per_coarse = Vec::new();

                    for c in all {
                        let mut r = ControlSourceResolver::new();
                        r.set(coarse.clone(), c);
                        r.set(thrust.clone(), t);
                        r.set(steering.clone(), s);
                        r.set(lateral.clone(), l);
                        r.set(impulse.clone(), i);

                        // The gate each system actually applies: its own
                        // system alone (#800 for thrust/steering, #703 for
                        // lateral/impulse). No system reads the coarse
                        // policy — the one that did is gone.
                        let tt = r.policy_for(&thrust).operate_ai;
                        let ss = r.policy_for(&steering).operate_ai;
                        let ll = r.policy_for(&lateral).operate_ai;
                        let ii = r.policy_for(&impulse).operate_ai;

                        let writers = HelmWriters {
                            thrust: tt,
                            steering: ss,
                            lateral: ll,
                            impulse: ii,
                        };

                        // Each component is written exactly when its own
                        // axis is AI — never otherwise (no coarse fallback),
                        // never dropped when it is (no lost writer).
                        for (name, own_axis_is_ai, written) in [
                            ("ThrustInput", tt, writers.thrust),
                            ("SteeringInput", ss, writers.steering),
                            ("LateralThrustInput", ll, writers.lateral),
                            ("ImpulseCommand", ii, writers.impulse),
                        ] {
                            assert_eq!(
                                written, own_axis_is_ai,
                                "{name} must be written exactly when its own axis is \
                                 AI-operated: coarse={c:?} thrust={t:?} steering={s:?} \
                                 lateral={l:?} impulse={i:?}"
                            );
                        }

                        if tt && ss && ll && ii {
                            saw_all_four_running = true;
                        }

                        outcome_per_coarse.push(writers);
                    }

                    // The #704 invariant: nothing above depended on `c`.
                    for (idx, other) in outcome_per_coarse.iter().enumerate().skip(1) {
                        assert_eq!(
                            &outcome_per_coarse[0], other,
                            "the coarse helm policy must not influence any helm-AI \
                             writer — #704 deleted the only system that read it. \
                             Differed between coarse={:?} and coarse={:?} at \
                             thrust={t:?} steering={s:?} lateral={l:?} impulse={i:?}",
                            all[0], all[idx]
                        );
                    }
                }
            }
        }
    }

    // The shipped-hull shape (every axis declared, station backfilled to AI)
    // must be inside the space this test covers — that combination was
    // unreachable under the old per-ship gates and is the whole point of
    // #800, #703 and #704.
    assert!(
        saw_all_four_running,
        "the shipped-hull all-AI combination must be covered"
    );
}

/// Ported in #704 from `coarse_helm_ai_result_is_unchanged_by_per_axis_systems`,
/// which pinned #800's stand-down: with the coarse helm on AI the monolith
/// owned the write and the per-axis systems stood down, so turning the fine
/// systems on changed nothing and the two runs were bit-identical.
///
/// Both terms of that equality were the monolith's output, so the delete
/// removes the property rather than moving it. Kept — same fixture, same two
/// runs, same measurement — with the assertion inverted, because inverting it
/// is precisely what #704 does: the coarse system no longer writes anything,
/// so the two runs must now *differ*, and the difference is the whole delete.
/// Equality here would now mean either a surviving coarse fallback (both
/// non-zero) or a dead per-axis path (both zero); the old test could not tell
/// you about either, and this one fails on both.
///
/// This had an end-to-end companion, `coarse_helm_alone_commits_no_memory`,
/// pinning that the coarse system wrote no `AiMemory` while this one pins
/// that it writes no intent. #702 deleted `AiMemory`, so the companion had
/// nothing left to observe and went with it; "writes no intent" is now the
/// whole of the property.
#[test]
fn coarse_helm_alone_drives_no_intent_but_the_per_axis_systems_do() {
    let anchor = "station-alpha";

    let coarse_only = {
        let mut app = test_app();
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        set_coarse_helm_only_ai(&mut app);
        tick(&mut app);
        (get_thrust_input(&mut app), get_steering_input(&mut app))
    };

    let coarse_plus_fine = {
        let mut app = test_app();
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        set_helm_control_source(&mut app, ControlSource::Ai);
        tick(&mut app);
        (get_thrust_input(&mut app), get_steering_input(&mut app))
    };

    assert_eq!(
        coarse_only,
        (0.0, 0.0),
        "the coarse helm system has no AI behaviour of its own since #704 deleted \
         operate_helm_ai; on its own it must leave the intent components untouched \
         (non-zero = a coarse fallback has come back)"
    );
    assert!(
        coarse_plus_fine.0 > 0.0 && coarse_plus_fine.1.abs() > 0.0,
        "declaring the axes is what drives the ship now: the per-axis systems must \
         produce the intent the monolith used to (got {coarse_plus_fine:?})"
    );
}

fn set_ship_blackboard_objectives(
    app: &mut App,
    objectives: Vec<crate::core::messages::ScoredObjective>,
) {
    use crate::core::messages::{SystemBlackboard, ViewscreenBlackboard};
    let vb = ViewscreenBlackboard {
        scored_objectives: objectives,
        ..Default::default()
    };
    let entry = (
        crate::ship::system_registry::viewscreen_system_id(),
        SystemBlackboard::Viewscreen(vb),
    );
    let mut q = app
        .world_mut()
        .query_filtered::<&mut crate::server_app::ShipSystemBlackboards, With<Ship>>();
    let mut bbs = q
        .single_mut(app.world_mut())
        .expect("expected Ship with ShipSystemBlackboards");
    bbs.0.insert(entry.0, entry.1);
}

/// Put `uuid` in the frozen viewscreen's Combat Lock (issue #829), leaving
/// the blackboard's scored objectives alone.
///
/// In production `ai_target_selection` publishes this lock for any ship
/// pursuing a Destroy directive, and `ai_helm_impulse` is the one helm axis
/// that resolves its target through it rather than through the directive's
/// own name — so a fixture that poses a Destroy objective without a lock has
/// an impulse system that can never resolve a target and therefore never
/// acts. Must be called AFTER `set_ship_blackboard_objectives`, which
/// replaces the whole viewscreen blackboard.
fn set_ship_combat_lock(app: &mut App, uuid: uuid::Uuid) {
    let mut q = app
        .world_mut()
        .query_filtered::<&mut crate::server_app::ShipSystemBlackboards, With<Ship>>();
    let mut bbs = q
        .single_mut(app.world_mut())
        .expect("expected Ship with ShipSystemBlackboards");
    match bbs
        .0
        .get_mut(&crate::ship::system_registry::viewscreen_system_id())
    {
        Some(crate::core::messages::SystemBlackboard::Viewscreen(bb)) => {
            bb.combat_lock = Some(uuid.to_string());
        }
        _ => panic!("set the viewscreen blackboard's objectives before its combat lock"),
    }
}

fn world_config_with_anchor(anchor: &str, pos: [f32; 3]) -> crate::world::config::WorldConfig {
    let mut cfg = crate::world::config::WorldConfig::default();
    cfg.anchors.insert(anchor.into(), pos);
    cfg
}

#[test]
fn helm_ai_navigates_toward_reach_objective() {
    let mut app = test_app();
    // Place anchor 100 units ahead (positive X) — ship starts at origin.
    let anchor = "station-alpha";
    set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
    app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
    set_helm_control_source(&mut app, ControlSource::Ai);

    tick(&mut app);

    let last = get_last_helm_input(&mut app);
    assert!(
        last.thrust > 0.0,
        "AI helm must apply positive thrust toward Reach anchor; got {last:?}"
    );
}

/// AC (issue #741): a ship pursues a test destination through the shared
/// motion path — the planner publishes a 3D desired-motion contract, the
/// per-axis AI decode it into admitted actuator input, and the shared
/// integrator moves the ship — while facing is carried and actuated
/// separately from travel.
///
/// The anchor sits off the starboard bow, so the ship must simultaneously
/// throttle up (travel) and yaw toward it (facing). The two are distinct
/// fields of the published `DesiredMotion`, and the integrator turns yaw
/// separately from forward travel.
#[test]
fn helm_motion_planner_drives_ship_to_destination_with_independent_facing() {
    use crate::ship::helm_planner::HelmMotionPlan;

    let mut app = test_app();
    let anchor = "test-destination";
    // Off the starboard bow of a ship at the origin facing -Z: +X is to the
    // right, so travel wants forward and facing wants to turn to starboard.
    set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
    app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, -50.0]));
    set_helm_control_source(&mut app, ControlSource::Ai);

    tick(&mut app);

    // The planner published a 3D desired-motion contract for the ship.
    let ship = find_ship_entity(&mut app);
    let plan = app.world().resource::<HelmMotionPlan>();
    let sp = plan
        .ships
        .get(&ship)
        .copied()
        .expect("planner must publish a desired-motion plan for the AI-helmed ship");
    assert!(
        sp.motion.desired_velocity_local.z < 0.0,
        "desired velocity must be forward (local -Z); got {:?}",
        sp.motion.desired_velocity_local
    );
    assert!(
        sp.motion.desired_facing_local.x > 0.0,
        "desired facing must point to starboard toward the destination; got {:?}",
        sp.motion.desired_facing_local
    );
    // Facing is a separate field from travel — the whole point of the split.
    assert_ne!(
        sp.motion.desired_facing_local, sp.motion.desired_velocity_local,
        "facing must be represented separately from travel"
    );

    // The shared path actuated it: the ship travels and turns.
    for _ in 0..6 {
        tick(&mut app);
    }
    let physics = get_ship_physics(&mut app);
    assert!(
        physics.forward_speed > 0.0,
        "the ship must move forward through the shared actuator path; got {physics:?}"
    );
    assert!(
        physics.yaw > 0.0,
        "the ship must yaw toward the starboard destination, integrated separately \
         from its forward travel; got {physics:?}"
    );
}

#[test]
fn helm_ai_navigates_toward_retreat_objective() {
    let mut app = test_app();
    // Place anchor 100 units ahead (positive X) — ship starts at origin.
    let anchor = "rally-point";
    set_ship_blackboard_objectives(&mut app, vec![retreat_scored_objective(anchor, 10.0)]);
    app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
    set_helm_control_source(&mut app, ControlSource::Ai);

    tick(&mut app);

    let last = get_last_helm_input(&mut app);
    assert!(
        last.thrust > 0.0,
        "AI helm must apply positive thrust toward Retreat anchor; got {last:?}"
    );
}

#[test]
fn helm_ai_patrols_from_viewscreen_objective() {
    let mut app = test_app();
    let anchor = "starbase_patrol_east";
    set_ship_blackboard_objectives(&mut app, vec![patrol_scored_objective(vec![anchor], 20.0)]);
    app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
    set_helm_control_source(&mut app, ControlSource::Ai);

    tick(&mut app);

    let last = get_last_helm_input(&mut app);
    assert!(
        last.thrust > 0.0,
        "AI helm must apply positive thrust toward Patrol anchor; got {last:?}"
    );
}

#[test]
fn helm_ai_pursues_named_destroy_objective() {
    let mut app = test_app();
    let target_uuid = uuid::Uuid::new_v4().to_string();
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(target_uuid.clone()),
        crate::entities::spawner::EntityName("Harrow Destroyer".into()),
        Transform::from_xyz(100.0, 0.0, 0.0),
    ));
    let mut runtime = crate::world::server::WorldContentRuntime::default();
    let target_uuid_str = target_uuid.clone();
    runtime.name_to_uuid.insert("wave_1".into(), target_uuid);
    app.insert_resource(runtime);
    set_ship_blackboard_objectives(&mut app, vec![destroy_scored_objective("wave_1", 80.0)]);
    set_helm_control_source(&mut app, ControlSource::Ai);

    // Tactical's lock: what the helm pursues (issue #702).
    set_ship_weapons_target(&mut app, &target_uuid_str);
    tick(&mut app);

    let last = get_last_helm_input(&mut app);
    assert!(
        last.thrust > 0.0,
        "AI helm must pursue named Destroy objective target; got {last:?}"
    );
}

// ── #674: helm radar gating ─────────────────────────────────────────────

#[test]
fn helm_ai_ignores_hostile_beyond_radar_range() {
    let mut app = test_app();
    let target_uuid = uuid::Uuid::new_v4().to_string();
    // Hostile is 100 units away.
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(target_uuid.clone()),
        crate::entities::spawner::EntityName("Harrow Destroyer".into()),
        Transform::from_xyz(100.0, 0.0, 0.0),
    ));
    let mut runtime = crate::world::server::WorldContentRuntime::default();
    runtime.name_to_uuid.insert("wave_1".into(), target_uuid);
    app.insert_resource(runtime);
    // Radar range (10.0) is far shorter than the hostile's distance (100.0).
    app.insert_resource(crate::lobby::server::ShipClientConfigResource(
        crate::core::messages::ShipClientConfig {
            helm_radar_range: 10.0,
            ..Default::default()
        },
    ));
    set_ship_blackboard_objectives(&mut app, vec![destroy_scored_objective("wave_1", 80.0)]);
    set_helm_control_source(&mut app, ControlSource::Ai);

    tick(&mut app);

    let last = get_last_helm_input(&mut app);
    assert_eq!(
        last,
        LastHelmInput::default(),
        "hostile beyond helm radar range must not be perceived; pursuit should fall through to idle, got {last:?}"
    );
}

#[test]
fn helm_ai_pursues_hostile_within_radar_range() {
    let mut app = test_app();
    let target_uuid = uuid::Uuid::new_v4().to_string();
    // Hostile is 100 units away.
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(target_uuid.clone()),
        crate::entities::spawner::EntityName("Harrow Destroyer".into()),
        Transform::from_xyz(100.0, 0.0, 0.0),
    ));
    let mut runtime = crate::world::server::WorldContentRuntime::default();
    let target_uuid_str = target_uuid.clone();
    runtime.name_to_uuid.insert("wave_1".into(), target_uuid);
    app.insert_resource(runtime);
    // Radar range (500.0) comfortably covers the hostile's distance (100.0).
    app.insert_resource(crate::lobby::server::ShipClientConfigResource(
        crate::core::messages::ShipClientConfig {
            helm_radar_range: 500.0,
            ..Default::default()
        },
    ));
    set_ship_blackboard_objectives(&mut app, vec![destroy_scored_objective("wave_1", 80.0)]);
    set_helm_control_source(&mut app, ControlSource::Ai);

    // Tactical's lock: what the helm pursues (issue #702).
    set_ship_weapons_target(&mut app, &target_uuid_str);
    tick(&mut app);

    let last = get_last_helm_input(&mut app);
    assert!(
        last.thrust > 0.0,
        "hostile within helm radar range must still be pursued as before; got {last:?}"
    );
}

// ── #677: Weapons->Helm arc-bearing request ──────────────────────────────
//
// Every test in this block flies the FLEET BASELINE steering policy — the
// stateless `actuate_desired_facing` block `attach_shipped_ai_declarations`
// takes from shipped content — which is to say a helm with no authored
// doctrine leg at all. That is what makes them issue #918's preservation
// tests as well as #677's: a hull with nothing committed has no heading to
// defend and still turns to bring a family that cannot bear onto its target.
// Nothing below was relaxed to accommodate #918, and nothing below may be.

#[test]
fn helm_ai_folds_pending_arc_bearing_request_into_steering() {
    let mut app = test_app();
    // Destroy target directly ahead and far away, so the baseline
    // pursuit steering (before any arc-bearing bias) is ~0.
    let destroy_uuid = uuid::Uuid::new_v4().to_string();
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(destroy_uuid.clone()),
        crate::entities::spawner::EntityName("Harrow Destroyer".into()),
        Transform::from_xyz(0.0, 0.0, -1000.0),
    ));
    let mut runtime = crate::world::server::WorldContentRuntime::default();
    let destroy_uuid_str = destroy_uuid.clone();
    runtime.name_to_uuid.insert("wave_1".into(), destroy_uuid);
    app.insert_resource(runtime);
    app.insert_resource(crate::lobby::server::ShipClientConfigResource(
        crate::core::messages::ShipClientConfig {
            helm_radar_range: 5000.0,
            ..Default::default()
        },
    ));
    set_ship_blackboard_objectives(&mut app, vec![destroy_scored_objective("wave_1", 80.0)]);
    set_helm_control_source(&mut app, ControlSource::Ai);

    // A separate hostile well off to starboard is the Weapons arc-bearing
    // request target — distinct from the Destroy pursuit target, so any
    // steering bias can only be attributed to the pending request.
    let bearing_uuid = uuid::Uuid::new_v4();
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(bearing_uuid.to_string()),
        crate::entities::spawner::EntityName("Bearing Contact".into()),
        Transform::from_xyz(200.0, 0.0, -1.0),
    ));
    let ship = find_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(ship)
        .insert(PendingArcBearingRequest {
            target: Some(bearing_uuid),
            arcs: vec![crate::core::messages::WeaponEmitterArc {
                facing_deg: 0.0,
                arc_deg: 30.0,
                range: 5000.0,
            }],
        });

    // Tactical's lock: what the helm pursues (issue #702).
    set_ship_weapons_target(&mut app, &destroy_uuid_str);
    tick(&mut app);

    let last = get_last_helm_input(&mut app);
    assert!(
        last.thrust > 0.0,
        "pending arc-bearing request must not disturb thrust/range-holding; got {last:?}"
    );
    assert!(
        last.steering.abs() > 0.01,
        "pending arc-bearing request must bias steering toward the requested bearing; got {last:?}"
    );
}

#[test]
fn helm_ai_clears_arc_bearing_request_once_facing_already_satisfies_the_arc() {
    let mut app = test_app();
    let destroy_uuid = uuid::Uuid::new_v4().to_string();
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(destroy_uuid.clone()),
        crate::entities::spawner::EntityName("Harrow Destroyer".into()),
        Transform::from_xyz(0.0, 0.0, -1000.0),
    ));
    let mut runtime = crate::world::server::WorldContentRuntime::default();
    runtime.name_to_uuid.insert("wave_1".into(), destroy_uuid);
    app.insert_resource(runtime);
    app.insert_resource(crate::lobby::server::ShipClientConfigResource(
        crate::core::messages::ShipClientConfig {
            helm_radar_range: 5000.0,
            ..Default::default()
        },
    ));
    set_ship_blackboard_objectives(&mut app, vec![destroy_scored_objective("wave_1", 80.0)]);
    set_helm_control_source(&mut app, ControlSource::Ai);

    // Bearing contact is directly ahead of the ship's starting facing
    // (yaw=0, forward=-Z) — i.e. the ship is already oriented such that a
    // wide-arc fore bank already bears on it.
    let bearing_uuid = uuid::Uuid::new_v4();
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(bearing_uuid.to_string()),
        crate::entities::spawner::EntityName("Bearing Contact".into()),
        Transform::from_xyz(0.0, 0.0, -200.0),
    ));
    let ship = find_ship_entity(&mut app);
    // The carried family arc (issue #767): a fore bank, narrow arc, range
    // that reaches the target directly ahead — so the ship's own facing
    // already brings it into arc AND range, i.e. the family can fire.
    app.world_mut()
        .entity_mut(ship)
        .insert(PendingArcBearingRequest {
            target: Some(bearing_uuid),
            arcs: vec![crate::core::messages::WeaponEmitterArc {
                facing_deg: 0.0,
                arc_deg: 30.0,
                range: 500.0,
            }],
        });

    tick(&mut app);

    let pending = app
        .world()
        .get::<PendingArcBearingRequest>(ship)
        .expect("ship must carry PendingArcBearingRequest");
    assert_eq!(
        pending.target, None,
        "a request must clear once the ship's own facing already brings the carried family's arc \
         onto the target, not persist indefinitely after being satisfied"
    );
}

/// AC4 (issue #767): a request clears when the target leaves the range of
/// every carried emitter arc — no yaw can help, so the bias must not
/// persist steering the ship at an unreachable contact.
#[test]
fn helm_ai_clears_arc_bearing_request_once_target_leaves_range() {
    let mut app = test_app();
    let destroy_uuid = uuid::Uuid::new_v4().to_string();
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(destroy_uuid.clone()),
        crate::entities::spawner::EntityName("Harrow Destroyer".into()),
        Transform::from_xyz(0.0, 0.0, -1000.0),
    ));
    let mut runtime = crate::world::server::WorldContentRuntime::default();
    runtime.name_to_uuid.insert("wave_1".into(), destroy_uuid);
    app.insert_resource(runtime);
    app.insert_resource(crate::lobby::server::ShipClientConfigResource(
        crate::core::messages::ShipClientConfig {
            helm_radar_range: 5000.0,
            ..Default::default()
        },
    ));
    set_ship_blackboard_objectives(&mut app, vec![destroy_scored_objective("wave_1", 80.0)]);
    set_helm_control_source(&mut app, ControlSource::Ai);

    // Bearing contact well off to starboard AND far beyond the carried
    // arc's range (range 50, target ~200 away) — out of reach entirely.
    let bearing_uuid = uuid::Uuid::new_v4();
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(bearing_uuid.to_string()),
        crate::entities::spawner::EntityName("Bearing Contact".into()),
        Transform::from_xyz(200.0, 0.0, -1.0),
    ));
    let ship = find_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(ship)
        .insert(PendingArcBearingRequest {
            target: Some(bearing_uuid),
            arcs: vec![crate::core::messages::WeaponEmitterArc {
                facing_deg: 0.0,
                arc_deg: 30.0,
                range: 50.0,
            }],
        });

    tick(&mut app);

    let pending = app
        .world()
        .get::<PendingArcBearingRequest>(ship)
        .expect("ship must carry PendingArcBearingRequest");
    assert_eq!(
        pending.target, None,
        "a request must clear once the target is beyond every carried arc's range — no bearing helps"
    );
}

#[test]
fn helm_ai_clears_arc_bearing_request_when_target_not_visible() {
    let mut app = test_app();
    let destroy_uuid = uuid::Uuid::new_v4().to_string();
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(destroy_uuid.clone()),
        crate::entities::spawner::EntityName("Harrow Destroyer".into()),
        Transform::from_xyz(0.0, 0.0, -1000.0),
    ));
    let mut runtime = crate::world::server::WorldContentRuntime::default();
    runtime.name_to_uuid.insert("wave_1".into(), destroy_uuid);
    app.insert_resource(runtime);
    app.insert_resource(crate::lobby::server::ShipClientConfigResource(
        crate::core::messages::ShipClientConfig {
            helm_radar_range: 5000.0,
            ..Default::default()
        },
    ));
    set_ship_blackboard_objectives(&mut app, vec![destroy_scored_objective("wave_1", 80.0)]);
    set_helm_control_source(&mut app, ControlSource::Ai);

    // Pending bearing references an entity that was never spawned — it
    // cannot be visible in the world view.
    let stale_uuid = uuid::Uuid::new_v4();
    let ship = find_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(ship)
        .insert(PendingArcBearingRequest {
            target: Some(stale_uuid),
            arcs: vec![crate::core::messages::WeaponEmitterArc {
                facing_deg: 0.0,
                arc_deg: 30.0,
                range: 5000.0,
            }],
        });

    tick(&mut app);

    let pending = app
        .world()
        .get::<PendingArcBearingRequest>(ship)
        .expect("ship must carry PendingArcBearingRequest");
    assert_eq!(
        pending.target, None,
        "a pending request for a no-longer-visible target must be cleared, not stuck forever"
    );
}

/// AC2 (issue #742): an arc-bearing request is *facing-only*. It biases
/// steering to bring a bank onto the target, but must never leak into the
/// travel axes — no reverse, no lateral drift. The distinction is what keeps
/// arc-bearing separate from the docking intent, which alone may translate.
#[test]
fn arc_bearing_request_never_commands_reverse_or_lateral() {
    use crate::ship::helm_planner::HelmMotionPlan;

    let mut app = test_app();
    // Destroy target directly ahead and far away → baseline steering ~0 and
    // steady forward throttle, so any change is attributable to the request.
    let destroy_uuid = uuid::Uuid::new_v4().to_string();
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(destroy_uuid.clone()),
        crate::entities::spawner::EntityName("Harrow Destroyer".into()),
        Transform::from_xyz(0.0, 0.0, -1000.0),
    ));
    let mut runtime = crate::world::server::WorldContentRuntime::default();
    let destroy_uuid_str = destroy_uuid.clone();
    runtime.name_to_uuid.insert("wave_1".into(), destroy_uuid);
    app.insert_resource(runtime);
    app.insert_resource(crate::lobby::server::ShipClientConfigResource(
        crate::core::messages::ShipClientConfig {
            helm_radar_range: 5000.0,
            ..Default::default()
        },
    ));
    set_ship_blackboard_objectives(&mut app, vec![destroy_scored_objective("wave_1", 80.0)]);
    set_helm_control_source(&mut app, ControlSource::Ai);

    // A hostile well off to starboard is the arc-bearing target.
    let bearing_uuid = uuid::Uuid::new_v4();
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(bearing_uuid.to_string()),
        crate::entities::spawner::EntityName("Bearing Contact".into()),
        Transform::from_xyz(200.0, 0.0, -1.0),
    ));
    let ship = find_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(ship)
        .insert(PendingArcBearingRequest {
            target: Some(bearing_uuid),
            arcs: vec![crate::core::messages::WeaponEmitterArc {
                facing_deg: 0.0,
                arc_deg: 30.0,
                range: 5000.0,
            }],
        });

    set_ship_weapons_target(&mut app, &destroy_uuid_str);
    tick(&mut app);

    // Facing did move (the request was folded into steering)...
    let last = get_last_helm_input(&mut app);
    assert!(
        last.steering.abs() > 0.01,
        "arc-bearing must bias steering toward the requested bearing; got {last:?}"
    );
    // ...but the travel axes are untouched: forward throttle held, no
    // reverse, and crucially no lateral drift.
    assert!(
        last.thrust > 0.0,
        "arc-bearing must not command reverse; thrust must stay forward; got {last:?}"
    );
    assert_eq!(
        last.lateral, 0.0,
        "arc-bearing is facing-only: it must never command lateral thrust; got {last:?}"
    );

    // The shared desired-motion contract confirms it at the source: the
    // planner never marked docking active and never wrote a lateral (`x`)
    // component — arc-bearing lives entirely in the facing field.
    let sp = *app
        .world()
        .resource::<HelmMotionPlan>()
        .ships
        .get(&ship)
        .expect("planner must publish a plan for the AI-helmed ship");
    assert!(
        !sp.docking_active,
        "an arc-bearing request must not engage the docking manoeuvre"
    );
    assert_eq!(
        sp.motion.desired_velocity_local.x, 0.0,
        "arc-bearing must leave the lateral travel component at zero; got {:?}",
        sp.motion.desired_velocity_local
    );
    assert!(
        sp.motion.desired_velocity_local.z < 0.0,
        "arc-bearing must leave forward travel forward (local -Z), not reverse; got {:?}",
        sp.motion.desired_velocity_local
    );
}

// ── #742: distinct docking motion intent ─────────────────────────────────

/// Build an AI-helmed ship with a Destroy objective on a far-ahead target
/// (so baseline travel is steady forward) plus a spawned dock contact within
/// radar range at `dock_pos`. Returns the app and the dock's UUID so the
/// caller can set (or mis-set) the `DockingMotionIntent`.
fn docking_app(dock_pos: [f32; 3]) -> (App, uuid::Uuid) {
    let mut app = test_app();
    let destroy_uuid = uuid::Uuid::new_v4().to_string();
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(destroy_uuid.clone()),
        crate::entities::spawner::EntityName("Harrow Destroyer".into()),
        Transform::from_xyz(0.0, 0.0, -4000.0),
    ));
    let dock_uuid = uuid::Uuid::new_v4();
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(dock_uuid.to_string()),
        crate::entities::spawner::EntityName("Axiom Station Dock".into()),
        Transform::from_xyz(dock_pos[0], dock_pos[1], dock_pos[2]),
    ));
    let mut runtime = crate::world::server::WorldContentRuntime::default();
    let destroy_uuid_str = destroy_uuid.clone();
    runtime.name_to_uuid.insert("wave_1".into(), destroy_uuid);
    app.insert_resource(runtime);
    app.insert_resource(crate::lobby::server::ShipClientConfigResource(
        crate::core::messages::ShipClientConfig {
            helm_radar_range: 5000.0,
            ..Default::default()
        },
    ));
    set_ship_blackboard_objectives(&mut app, vec![destroy_scored_objective("wave_1", 80.0)]);
    set_helm_control_source(&mut app, ControlSource::Ai);
    set_ship_weapons_target(&mut app, &destroy_uuid_str);
    (app, dock_uuid)
}

/// AC3 (issue #742): an active docking intent, once the dock is within
/// engage distance, drives a controlled *translation* — reverse and lateral
/// — through the shared motion path. These are exactly the motions
/// arc-bearing (facing-only) must never command, proving the two intents are
/// distinct.
#[test]
fn docking_intent_commands_controlled_reverse_and_lateral() {
    use crate::ship::helm_planner::HelmMotionPlan;

    // Dock 20 units astern (+Z) and to starboard (+X) of a ship at the
    // origin facing -Z — well inside the default 40-unit engage distance.
    let (mut app, dock_uuid) = docking_app([10.0, 0.0, 20.0]);
    let ship = find_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(ship)
        .insert(crate::ship_plugin::DockingMotionIntent(Some(dock_uuid)));

    tick(&mut app);

    let sp = *app
        .world()
        .resource::<HelmMotionPlan>()
        .ships
        .get(&ship)
        .expect("planner must publish a plan for the AI-helmed ship");
    assert!(
        sp.docking_active,
        "a dock within engage distance must engage the docking manoeuvre"
    );
    assert!(
        sp.motion.desired_velocity_local.z > 0.0,
        "an astern dock must command controlled reverse (local +Z); got {:?}",
        sp.motion.desired_velocity_local
    );
    assert!(
        sp.motion.desired_velocity_local.x > 0.0,
        "a starboard dock must command starboard lateral translation; got {:?}",
        sp.motion.desired_velocity_local
    );

    // The shared actuator path carried it: reverse thrust and lateral thrust
    // both landed on the ship's admitted inputs.
    let last = get_last_helm_input(&mut app);
    assert!(
        last.thrust < 0.0,
        "docking reverse must reach the thrust actuator as negative thrust; got {last:?}"
    );
    assert!(
        last.lateral.abs() > 0.0,
        "docking lateral must reach the lateral-thrust actuator; got {last:?}"
    );
}

/// AC4 (issue #742): a docking intent expires the instant its dock target is
/// no longer visible — the planner clears it rather than leaving the ship
/// manoeuvring toward a ghost. Mirrors arc-bearing's target-not-visible
/// clear.
#[test]
fn docking_intent_expires_when_target_not_visible() {
    use crate::ship::helm_planner::HelmMotionPlan;

    // Dock exists but the intent names a UUID that was never spawned.
    let (mut app, _dock_uuid) = docking_app([10.0, 0.0, 20.0]);
    let ghost = uuid::Uuid::new_v4();
    let ship = find_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(ship)
        .insert(crate::ship_plugin::DockingMotionIntent(Some(ghost)));

    tick(&mut app);

    let intent = app
        .world()
        .get::<crate::ship_plugin::DockingMotionIntent>(ship)
        .expect("ship must carry DockingMotionIntent");
    assert_eq!(
        intent.0, None,
        "a docking intent for a no-longer-visible dock must be cleared, not stuck forever"
    );
    let sp = *app
        .world()
        .resource::<HelmMotionPlan>()
        .ships
        .get(&ship)
        .expect("planner must publish a plan");
    assert!(
        !sp.docking_active,
        "an expired docking intent must not leave the manoeuvre engaged"
    );
}

/// AC1 (issue #742): the Helm AI consumes the authoritative Navigation
/// waypoint through the shared motion path — the planner's DesiredMotion
/// steers toward it — regardless of whether a human officer or the
/// Navigation AI wrote it. Both sources converge on the same
/// `NavigationWaypoint` + `HelmWaypointClearance` latch
/// (`human_set_nav_waypoint_eventually_clears_and_the_ai_helm_flies_it`
/// pins the human wire path; `operate_navigation_ai` emits the identical
/// admitted command), so asserting the planner consumes that latch covers
/// both origins.
#[test]
fn cleared_nav_waypoint_reaches_the_motion_planner_regardless_of_source() {
    use crate::ship::helm_planner::HelmMotionPlan;

    let mut app = test_app();
    set_helm_control_source(&mut app, ControlSource::Ai);
    // A Helm-relevant objective that cannot resolve, so the only thing left
    // to fly is the Navigation waypoint reaching the planner.
    set_ship_blackboard_objectives(
        &mut app,
        vec![reach_scored_objective("anchor-not-in-world-config", 8.0)],
    );
    // Waypoint dead ahead (local -Z) with the clearance latched — the shared
    // state both the human and AI Navigation sources write.
    set_cleared_nav_waypoint(&mut app, 0.0, -900.0);

    tick(&mut app);

    let ship = find_ship_entity(&mut app);
    let sp = *app
        .world()
        .resource::<HelmMotionPlan>()
        .ships
        .get(&ship)
        .expect("planner must publish a plan for the AI-helmed ship");
    assert!(
        sp.motion.desired_velocity_local.z < 0.0,
        "the planner must turn the cleared nav waypoint into forward desired travel; got {:?}",
        sp.motion.desired_velocity_local
    );
}

#[test]
fn helm_ai_does_nothing_when_helm_human() {
    let mut app = test_app();
    let anchor = "station-alpha";
    set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
    app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
    // helm stays Human (default)

    tick(&mut app);

    let last = get_last_helm_input(&mut app);
    assert_eq!(
        last,
        LastHelmInput::default(),
        "helm AI must not overwrite LastHelmInput when helm is human; got {last:?}"
    );
}

#[test]
fn helm_ai_stays_zero_when_destroy_target_missing() {
    let mut app = test_app();
    // Blackboard has a Destroy directive, but no live entity resolves to it.
    use crate::core::messages::{
        AiDirective, ObjectiveSnapshot, ObjectiveSource, ObjectiveStatus, ScoredObjective,
        SystemAffinity,
    };
    set_ship_blackboard_objectives(
        &mut app,
        vec![ScoredObjective {
            id: "destroy-pirates".into(),
            score: 5.0,
            directive: AiDirective::Destroy {
                target: "pirate".into(),
            },
            source: ObjectiveSource::Mission,
            relevance: vec![SystemAffinity::Helm],
            snapshot: ObjectiveSnapshot {
                id: "destroy-pirates".into(),
                text: "Destroy pirates".into(),
                text_params: Default::default(),
                mandatory: true,
                status: ObjectiveStatus::Active,
                targets: vec![],
                source: ObjectiveSource::Mission,
            },
        }],
    );
    set_helm_control_source(&mut app, ControlSource::Ai);

    tick(&mut app);

    // operate_helm_ai: unresolved Destroy target → zero thrust remains.
    let last = get_last_helm_input(&mut app);
    assert_eq!(
        last,
        LastHelmInput::default(),
        "missing Destroy target means Backfill zero should remain; got {last:?}"
    );
}

#[test]
fn detect_reach_completion_marks_objective_complete() {
    use crate::core::messages::{AiDirective, ObjectiveSource};
    use crate::objectives::{ObjectiveManager, UtilityConfig};
    use crate::world::server::ObjectiveManagerRes;

    let mut app = test_app();
    let anchor = "dock-alpha";
    // Anchor at origin — ship also starts at origin, so distance == 0.
    // detect_reached_objective_completion reads from ShipSystemBlackboards component.
    set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 8.0)]);
    app.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, 0.0]));
    set_helm_control_source(&mut app, ControlSource::Ai);

    let mut mgr = ObjectiveManager::new();
    mgr.add_full(
        "reach-dock-alpha",
        "Dock at Alpha",
        true,
        vec![],
        AiDirective::Reach {
            anchor: anchor.into(),
        },
        UtilityConfig::default(),
        ObjectiveSource::Mission,
    );
    app.insert_resource(ObjectiveManagerRes(mgr));

    tick(&mut app);

    let res = app.world().resource::<ObjectiveManagerRes>();
    let obj = res
        .0
        .sorted_snapshots()
        .into_iter()
        .find(|o| o.id == "reach-dock-alpha");
    assert!(
        obj.map(|o| o.status == crate::core::messages::ObjectiveStatus::Completed)
            .unwrap_or(false),
        "Reach objective should be completed when ship is within arrival radius"
    );
}

/// Drives the REAL emission seam — `detect_reached_objective_completion`
/// running in `test_app` — rather than constructing the variant by hand, so
/// a regression of the `if objectives.0.complete(...)` guard at the site
/// (issue #841) fails a test. The pure JSON round-trip test constructs the
/// variant literally and would stay green even if this wiring were deleted;
/// this is the only guard on the `ObjectiveCompleted` emission itself.
///
/// Two ticks share one cursor: arrival emits exactly one
/// `ObjectiveCompleted` for the right id, and the second tick — where
/// `complete()` no longer transitions — emits nothing, pinning the
/// idempotency guard (deleting `if complete()` would double-emit here).
#[test]
fn detect_reach_completion_emits_objective_completed_once() {
    use crate::core::messages::{AiDirective, ObjectiveSource};
    use crate::objectives::{ObjectiveManager, UtilityConfig};
    use crate::world::server::ObjectiveManagerRes;
    use bevy::ecs::message::Messages;

    let mut app = test_app();
    // Register the balance sink the emission site writes to. `init_resource`
    // (not `add_message`) so no per-frame double-buffer swap can drop the
    // first-tick message before the second-tick idempotency read.
    app.init_resource::<Messages<crate::core::balance::BalanceEvent>>();

    let anchor = "dock-alpha";
    // Anchor at origin — the ship also starts at origin (distance == 0).
    set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 8.0)]);
    app.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, 0.0]));
    set_helm_control_source(&mut app, ControlSource::Ai);

    let mut mgr = ObjectiveManager::new();
    mgr.add_full(
        "reach-dock-alpha",
        "Dock at Alpha",
        true,
        vec![],
        AiDirective::Reach {
            anchor: anchor.into(),
        },
        UtilityConfig::default(),
        ObjectiveSource::Mission,
    );
    app.insert_resource(ObjectiveManagerRes(mgr));

    let mut cursor = app
        .world()
        .resource::<Messages<crate::core::balance::BalanceEvent>>()
        .get_cursor();

    tick(&mut app);

    let messages = app
        .world()
        .resource::<Messages<crate::core::balance::BalanceEvent>>();
    let first: Vec<&crate::core::balance::BalanceEvent> = cursor.read(messages).collect();
    assert_eq!(
        first.len(),
        1,
        "arrival must emit exactly one balance event, got {first:?}"
    );
    match first[0] {
        crate::core::balance::BalanceEvent::ObjectiveCompleted { objective_id } => {
            assert_eq!(objective_id, "reach-dock-alpha");
        }
        other => panic!("expected ObjectiveCompleted, got {other:?}"),
    }

    tick(&mut app);

    let messages = app
        .world()
        .resource::<Messages<crate::core::balance::BalanceEvent>>();
    let second: Vec<&crate::core::balance::BalanceEvent> = cursor.read(messages).collect();
    assert!(
        second.is_empty(),
        "re-completing an already-Completed objective must not emit again; got {second:?}"
    );
}

// ── Channel-3 Navigation→Helm clearance (issue #702) ──────────────────
//
// `cleared_nav_waypoint` is where the Channel-3 lag lives on the read side:
// the Helm follows the ship's `NavigationWaypoint` only while its
// `HelmWaypointClearance` names that waypoint's `generation`. These pin the
// gate itself — deleting the comparison must not be a silent no-op.

/// The happy path: clearance matches the waypoint's generation, so the Helm
/// is cleared to fly it.
#[test]
fn cleared_nav_waypoint_returns_the_waypoint_when_the_clearance_matches() {
    let waypoint =
        crate::console::navigation::NavigationWaypoint::new(WaypointMode::Free { x: 5.0, z: -7.0 });
    let clearance = HelmWaypointClearance(Some(waypoint.generation()));

    assert_eq!(
        cleared_nav_waypoint(Some(&waypoint), Some(&clearance)),
        Some([5.0, -7.0])
    );
}

/// The lag itself: Navigation has set a *new* waypoint, but the `NavigateTo`
/// carrying its generation is still in the coordination queue. The Helm must
/// not fly it yet — it has been given the waypoint but not the order.
///
/// This is why the clearance is a generation rather than a bool: a bool
/// ("Navigation has spoken") would go true once and wave every subsequent
/// waypoint straight through, so only the first order would ever be delayed.
#[test]
fn cleared_nav_waypoint_withholds_a_waypoint_newer_than_the_clearance() {
    let mut waypoint =
        crate::console::navigation::NavigationWaypoint::new(WaypointMode::Free { x: 5.0, z: -7.0 });
    // The Helm was cleared for this one, and is flying it.
    let clearance = HelmWaypointClearance(Some(waypoint.generation()));
    assert!(cleared_nav_waypoint(Some(&waypoint), Some(&clearance)).is_some());

    // Navigation now re-tasks the ship. The order has not arrived yet.
    waypoint.set(WaypointMode::Free { x: 900.0, z: 900.0 });

    assert_eq!(
        cleared_nav_waypoint(Some(&waypoint), Some(&clearance)),
        None,
        "a re-tasked waypoint must re-incur the Channel-3 lag; without this \
         every waypoint after the first would be followed instantly"
    );

    // …and once Helm's post-lag receiver latches the new generation, it is.
    let caught_up = HelmWaypointClearance(Some(waypoint.generation()));
    assert_eq!(
        cleared_nav_waypoint(Some(&waypoint), Some(&caught_up)),
        Some([900.0, 900.0])
    );
}

/// A ship never cleared for anything follows nothing.
#[test]
fn cleared_nav_waypoint_is_none_without_a_clearance() {
    let waypoint =
        crate::console::navigation::NavigationWaypoint::new(WaypointMode::Free { x: 5.0, z: -7.0 });

    assert_eq!(
        cleared_nav_waypoint(Some(&waypoint), Some(&HelmWaypointClearance(None))),
        None,
        "never cleared = never followed"
    );
    assert_eq!(
        cleared_nav_waypoint(Some(&waypoint), None),
        None,
        "a ship with no clearance component at all is never cleared"
    );
    assert_eq!(
        cleared_nav_waypoint(None, Some(&HelmWaypointClearance(Some(1)))),
        None,
        "a clearance with no waypoint names nowhere"
    );
}

/// Through the real system: an uncleared waypoint does not move the ship,
/// and the same waypoint does once the clearance lands.
///
/// The unit tests above pin `cleared_nav_waypoint`; this pins that
/// `ai_helm_thrust` actually consults it rather than reading the waypoint
/// directly and skipping the lag.
#[test]
fn ai_helm_flies_the_nav_waypoint_only_once_cleared() {
    fn app_with_waypoint(clear_it: bool) -> App {
        let mut app = test_app();
        set_helm_control_source(&mut app, ControlSource::Ai);
        // A Helm-relevant objective that cannot resolve, so the only thing
        // left to fly is the Navigation waypoint.
        set_ship_blackboard_objectives(
            &mut app,
            vec![reach_scored_objective("anchor-not-in-world-config", 8.0)],
        );
        if clear_it {
            set_cleared_nav_waypoint(&mut app, 0.0, -900.0);
        } else {
            // Waypoint set, order not yet delivered.
            let ship = find_ship_entity(&mut app);
            let mut entity = app.world_mut().entity_mut(ship);
            let mut waypoint = entity
                .get_mut::<crate::console::navigation::NavigationWaypoint>()
                .expect("ship must carry NavigationWaypoint");
            waypoint.set(WaypointMode::Free { x: 0.0, z: -900.0 });
        }
        tick(&mut app);
        app
    }

    assert_eq!(
        get_thrust_input(&mut app_with_waypoint(false)),
        0.0,
        "the waypoint is set but the Channel-3 order has not been delivered, \
         so the AI helm must not fly it yet"
    );
    assert!(
        get_thrust_input(&mut app_with_waypoint(true)) > 0.0,
        "once Helm's post-lag receiver latches the clearance, the same \
         waypoint must be flown"
    );
}

/// Rule-6 symmetry, end to end over the wire: a *human* navigation
/// officer's admitted `SetNavigationWaypoint` reaches an AI Helm exactly
/// as an AI-set waypoint does — the same `NavigateTo` clearance, the same
/// Channel-3 delivery lag, the same `HelmWaypointClearance` latch — and
/// the AI Helm then flies it.
///
/// Before the fix only `operate_navigation_ai` enqueued the clearance, so
/// a human-set waypoint sat on the shared `NavigationWaypoint` forever
/// unfollowed: `cleared_nav_waypoint` withholds any generation the
/// clearance has not latched, and nothing ever latched one.
#[test]
fn human_set_nav_waypoint_eventually_clears_and_the_ai_helm_flies_it() {
    let mut app = test_app();
    // The waypoint write path lives in NavigationPlugin
    // (`handle_navigation_waypoint`); its blackboard publisher needs the
    // client-config resource.
    app.add_plugins(crate::console::navigation::NavigationPlugin)
        .init_resource::<crate::lobby::server::ShipClientConfigResource>();

    // A human captain + navigation officer, game started; the Helm
    // station is unmanned and on AI.
    push(
        &mut app,
        "captain",
        ClientMessage::Identify {
            token: "captain".into(),
            name: "Alice".into(),
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "captain",
        ClientMessage::SelectStation {
            station: "Captain".into(),
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "navigation",
        ClientMessage::Identify {
            token: "navigation".into(),
            name: "Decker".into(),
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "navigation",
        ClientMessage::SelectStation {
            station: "Navigation".into(),
        },
    );
    tick(&mut app);
    push(&mut app, "captain", ClientMessage::SetReady { ready: true });
    push(
        &mut app,
        "navigation",
        ClientMessage::SetReady { ready: true },
    );
    tick(&mut app);

    set_helm_control_source(&mut app, ControlSource::Ai);
    // A Helm-relevant objective that cannot resolve, so the only thing
    // left to fly is the Navigation waypoint (same shape as
    // `ai_helm_flies_the_nav_waypoint_only_once_cleared`).
    set_ship_blackboard_objectives(
        &mut app,
        vec![reach_scored_objective("anchor-not-in-world-config", 8.0)],
    );

    // The human sets the waypoint over the wire.
    push(
        &mut app,
        "navigation",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("navigation".into()),
            payload: crate::core::messages::SystemControlPayload::SetNavigationWaypoint {
                x: 0.0,
                z: -900.0,
                source_uuid: None,
            },
        },
    );
    tick(&mut app);

    let ship = find_ship_entity(&mut app);
    let generation = app
        .world()
        .entity(ship)
        .get::<crate::console::navigation::NavigationWaypoint>()
        .expect("ship must carry NavigationWaypoint")
        .generation();
    assert!(
        app.world()
            .entity(ship)
            .get::<crate::console::navigation::NavigationWaypoint>()
            .and_then(|w| w.snapshot())
            .is_some(),
        "the admitted SetNavigationWaypoint must set the shared waypoint"
    );
    assert_eq!(
        app.world()
            .entity(ship)
            .get::<HelmWaypointClearance>()
            .expect("ship must carry HelmWaypointClearance")
            .0,
        None,
        "the NavigateTo order must still be serving its Channel-3 lag"
    );
    assert_eq!(
        get_thrust_input(&mut app),
        0.0,
        "the AI Helm must not fly a waypoint before the clearance lands"
    );

    // Serve the Channel-3 delivery lag (authored per hull; each tick
    // advances the manual clock by 200 ms), plus slack for the tick that
    // enqueues and the tick that delivers.
    let lag_secs = {
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipConfigComponent, With<Ship>>();
        q.single(app.world())
            .expect("ship config")
            .0
            .coordination_lag_secs
    };
    let ticks = (lag_secs / 0.2).ceil() as u32 + 4;
    for _ in 0..ticks {
        tick(&mut app);
    }

    assert_eq!(
        app.world()
            .entity(ship)
            .get::<HelmWaypointClearance>()
            .expect("ship must carry HelmWaypointClearance")
            .0,
        Some(generation),
        "the human-set waypoint's NavigateTo must latch its generation \
         into the AI Helm's clearance once the lag is served"
    );
    assert!(
        get_thrust_input(&mut app) > 0.0,
        "once cleared, the AI Helm must fly the human-set waypoint — \
         rule-6 symmetry with the AI-set path"
    );
}

/// Waypoint clearance survives a helm control flip: a waypoint set while
/// the helm is HUMAN-manned delivers as suppressed/popup (no latch); when
/// the helm later flips to AI (disconnect → Backfill), the shared issuer
/// re-issues the `NavigateTo` on the Human→AI edge, the order serves the
/// normal Channel-3 lag, latches, and the AI helm flies the existing
/// waypoint — no human re-set required, and no instant latch.
#[test]
fn waypoint_set_while_helm_human_is_flown_once_helm_flips_to_ai() {
    let mut app = test_app();
    app.add_plugins(crate::console::navigation::NavigationPlugin)
        .init_resource::<crate::lobby::server::ShipClientConfigResource>();

    // A human captain + navigation officer, game started. The helm axes
    // stay on their default Human control for now.
    push(
        &mut app,
        "captain",
        ClientMessage::Identify {
            token: "captain".into(),
            name: "Alice".into(),
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "captain",
        ClientMessage::SelectStation {
            station: "Captain".into(),
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "navigation",
        ClientMessage::Identify {
            token: "navigation".into(),
            name: "Decker".into(),
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "navigation",
        ClientMessage::SelectStation {
            station: "Navigation".into(),
        },
    );
    tick(&mut app);
    push(&mut app, "captain", ClientMessage::SetReady { ready: true });
    push(
        &mut app,
        "navigation",
        ClientMessage::SetReady { ready: true },
    );
    tick(&mut app);

    // A Helm-relevant objective that cannot resolve, so once the helm is
    // AI the only thing left to fly is the Navigation waypoint.
    set_ship_blackboard_objectives(
        &mut app,
        vec![reach_scored_objective("anchor-not-in-world-config", 8.0)],
    );

    // The human sets the waypoint over the wire while the helm is human.
    push(
        &mut app,
        "navigation",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("navigation".into()),
            payload: crate::core::messages::SystemControlPayload::SetNavigationWaypoint {
                x: 0.0,
                z: -900.0,
                source_uuid: None,
            },
        },
    );
    tick(&mut app);

    let ship = find_ship_entity(&mut app);
    let generation = app
        .world()
        .entity(ship)
        .get::<crate::console::navigation::NavigationWaypoint>()
        .expect("ship must carry NavigationWaypoint")
        .generation();

    // Serve well past the delivery lag with the helm still human: the
    // order routes to the human helm (suppress — human sender, human
    // target) and must NOT latch a clearance.
    let lag_secs = {
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipConfigComponent, With<Ship>>();
        q.single(app.world())
            .expect("ship config")
            .0
            .coordination_lag_secs
    };
    let ticks = (lag_secs / 0.2).ceil() as u32 + 4;
    for _ in 0..ticks {
        tick(&mut app);
    }
    assert_eq!(
        app.world()
            .entity(ship)
            .get::<HelmWaypointClearance>()
            .expect("ship must carry HelmWaypointClearance")
            .0,
        None,
        "an order delivered to a human helm must not latch a clearance"
    );
    assert_eq!(
        get_thrust_input(&mut app),
        0.0,
        "no AI helm, no flight — nothing should be driving the thrust axis"
    );

    // The helm flips to AI (the disconnect → Backfill shape).
    set_helm_control_source(&mut app, ControlSource::Ai);

    // The clearance must not latch instantly — the re-issued order still
    // serves the authored Channel-3 delivery lag.
    tick(&mut app);
    assert_eq!(
        app.world()
            .entity(ship)
            .get::<HelmWaypointClearance>()
            .expect("ship must carry HelmWaypointClearance")
            .0,
        None,
        "the re-issued NavigateTo must serve the delivery lag, not latch instantly"
    );

    // Serve the lag (authored per hull), plus slack for the tick that
    // enqueues and the tick that delivers.
    for _ in 0..ticks {
        tick(&mut app);
    }
    assert_eq!(
        app.world()
            .entity(ship)
            .get::<HelmWaypointClearance>()
            .expect("ship must carry HelmWaypointClearance")
            .0,
        Some(generation),
        "after the helm flips to AI, the re-issued NavigateTo must latch \
         the existing waypoint's generation once the lag is served"
    );
    assert!(
        get_thrust_input(&mut app) > 0.0,
        "the AI helm must fly the waypoint that was set while the helm \
         was human — clearance survives the control flip"
    );
}

/// Regression (issue #696 review, finding 2): `[behaviour]
/// waypoint_arrival_radius` is authored per entity template in TOML and
/// read by the cursor evaluator at every LOD. The high-LOD helm's own
/// turn-at-waypoint decision must agree with it rather than hardcoding
/// `WAYPOINT_ARRIVAL_RADIUS` — otherwise a designer's widened radius is
/// honoured for triggers but ignored for steering.
///
/// Probed through the helm's *steering* rather than through a waypoint
/// index (issue #702). The helm no longer keeps an index of its own to
/// look at: `advance_objective_cursors` owns every cursor, and `helm_patrol`
/// only reads. What the radius still decides here — and all this test ever
/// really cared about — is the helm's own arrival branch: short of the
/// radius it turns toward the waypoint; inside it, it flies straight
/// through. That is directly observable.
#[test]
fn high_lod_helm_honours_toml_authored_waypoint_arrival_radius() {
    fn patrol_app(arrival_radius: Option<f32>) -> App {
        let mut app = test_app();
        // wp0 sits 100 units to starboard — inside a 150 radius, outside
        // the default 20.
        let mut cfg = crate::world::config::WorldConfig::default();
        cfg.anchors.insert("wp0".into(), [100.0, 0.0, 0.0]);
        cfg.anchors.insert("wp1".into(), [900.0, 0.0, 0.0]);
        set_ship_blackboard_objectives(
            &mut app,
            vec![patrol_scored_objective(vec!["wp0", "wp1"], 20.0)],
        );
        app.insert_resource(cfg);
        set_helm_control_source(&mut app, ControlSource::Ai);
        if let Some(radius) = arrival_radius {
            let ship = find_ship_entity(&mut app);
            app.world_mut()
                .entity_mut(ship)
                .insert(crate::entities::spawner::BehaviourSection(
                    crate::entities::config::BehaviourConfig {
                        waypoint_arrival_radius: radius,
                        ..Default::default()
                    },
                ));
        }
        tick(&mut app);
        app
    }

    assert!(
        get_steering_input(&mut patrol_app(None)) > 0.0,
        "with the default arrival radius the helm is still 100 units short of \
         wp0, so it must turn toward it (wp0 is to starboard)"
    );
    assert_eq!(
        get_steering_input(&mut patrol_app(Some(150.0))),
        0.0,
        "a TOML-widened arrival radius must put the high-LOD helm *inside* \
         wp0, so it flies straight through — the same radius, and the same \
         call, the cursor evaluator makes. A hardcoded WAYPOINT_ARRIVAL_RADIUS \
         would still be turning."
    );
}

// ── TOML-authored avoidance tuning (AGENTS.md rule 11) ────────────────
//
// `[behaviour] avoidance_buffer` / `avoidance_look_ahead_secs` are
// declared with serde defaults, so a designer can author them per entity
// template. Two sites feed them to the pure AI: `helm_ai_decision`
// (steering/thrust) and the per-axis `ai_helm_lateral_thrust` (lateral
// dodge). Each test below pins one of the tuning
// fields by choosing a geometry that the constant and the authored value
// disagree about, so reverting a site to `crate::ai::AVOIDANCE_*` turns
// the assertion red.

/// Seeds a `WorldSnapshot` holding a single stationary obstacle, so the
/// avoidance maths has exactly one threat to reason about and the
/// assertions below can attribute any lateral dodge to it alone.
fn snapshot_with_obstacle(app: &mut App, position: [f32; 3], radius: f32) {
    app.insert_resource(crate::ai::server::WorldSnapshot {
        entities: vec![crate::ai::AiWorldEntity {
            uuid: uuid::Uuid::new_v4(),
            name: Some("rock".into()),
            position,
            faction: None,
            shields: None,
            hull_fraction: None,
            // `None` yaw keeps the obstacle un-projected, so
            // `avoidance_look_ahead_secs` only moves *our* projected
            // position — one variable, not two.
            yaw: None,
            radius,
            forward_speed: 0.0,
            // A static rock: not movable, but a dangerous collision hazard;
            // size rating tracks its radius (issue #743).
            movable: false,
            dangerous: true,
            size_rating: radius,
            direct_fire_range: 0.0,
            weapon_arcs: Vec::new(),
        }],
    });
}

fn set_behaviour_section(app: &mut App, behaviour: crate::entities::config::BehaviourConfig) {
    let ship = find_ship_entity(app);
    app.world_mut()
        .entity_mut(ship)
        .insert(crate::entities::spawner::BehaviourSection(behaviour));
}

fn lateral_intent(app: &mut App) -> f32 {
    app.world_mut()
        .query::<&LateralThrustInput>()
        .single(app.world())
        .expect("ship must carry LateralThrustInput")
        .0
}

/// `ai_helm_lateral_thrust` under the "Simplified" partial-automation
/// rating: lateral thrust AI-operated, the helm proper still human. Since
/// #703 the coarse helm's state no longer gates the system, but these tests
/// keep it human so the monolith cannot be the writer of the dodge they
/// measure.
fn lateral_thrust_ai_app(behaviour: Option<crate::entities::config::BehaviourConfig>) -> App {
    let mut app = test_app();
    set_helm_control_source(&mut app, ControlSource::Human);
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipSystemControlSources, With<Ship>>();
        for mut cs in q.iter_mut(app.world_mut()) {
            cs.0.set(
                crate::ship::system_registry::lateral_thrust_system_id(),
                ControlSource::Ai,
            );
        }
    }
    set_ship_blackboard_objectives(&mut app, vec![patrol_scored_objective(vec!["wp0"], 20.0)]);
    if let Some(behaviour) = behaviour {
        set_behaviour_section(&mut app, behaviour);
    }
    app
}

/// A wider TOML `avoidance_buffer` must widen the dodge radius of the
/// standalone lateral-thrust AI. The obstacle sits 40 units off the bow:
/// outside the default 5-unit buffer (radius 0+1+5 = 6), inside an
/// authored 60 (radius 0+1+60 = 61).
#[test]
fn lateral_thrust_ai_honours_toml_authored_avoidance_buffer() {
    // Stationary ship, so `avoidance_look_ahead_secs` scales a zero
    // velocity and cannot influence the result — isolating the buffer.
    let obstacle = [4.0, 0.0, -40.0];

    let mut default_app = lateral_thrust_ai_app(None);
    snapshot_with_obstacle(&mut default_app, obstacle, 1.0);
    // Two ticks: belt-and-braces against the shared AI-helm sim tick
    // (#803) — the first update runs on the ready latch's initial `true`,
    // and the second's 200 ms delta fires the timer outright.
    tick_twice(&mut default_app);
    assert_eq!(
        lateral_intent(&mut default_app),
        0.0,
        "with the default 5-unit buffer a 40-unit-distant obstacle is not a threat"
    );

    let mut authored_app = lateral_thrust_ai_app(Some(crate::entities::config::BehaviourConfig {
        avoidance_buffer: 60.0,
        ..Default::default()
    }));
    snapshot_with_obstacle(&mut authored_app, obstacle, 1.0);
    tick_twice(&mut authored_app);
    assert!(
        lateral_intent(&mut authored_app).abs() > 0.0,
        "a TOML-authored 60-unit avoidance_buffer must bring the same obstacle \
         inside the dodge radius; got no lateral thrust, so the system is still \
         reading crate::ai::AVOIDANCE_BUFFER"
    );
}

/// A longer TOML `avoidance_look_ahead_secs` must project the ship further
/// forward before testing for a threat. At 10 u/s the default 3 s horizon
/// stops 70 units short of the obstacle (well outside the 6-unit dodge
/// radius); an authored 10 s lands the projection right on top of it.
#[test]
fn lateral_thrust_ai_honours_toml_authored_avoidance_look_ahead() {
    // Forward at yaw 0 is -Z, so the obstacle sits 100 units down -Z with
    // a 2-unit lateral offset to give the dodge a defined sign.
    let obstacle = [2.0, 0.0, -100.0];

    fn moving_app(behaviour: Option<crate::entities::config::BehaviourConfig>) -> App {
        let mut app = lateral_thrust_ai_app(behaviour);
        let mut physics = get_ship_physics(&mut app);
        physics.forward_speed = 10.0;
        physics.yaw = 0.0;
        set_ship_physics(&mut app, physics);
        app
    }

    // ONE tick per app: since #895 the first fixed step is always a
    // decision tick (the latch derives from the tick count and arms at
    // tick 0), and a second tick would integrate 200 ms of drag first —
    // the projection would then be measured at a decayed speed rather
    // than the seeded 10 u/s the geometry above is computed from.
    let mut default_app = moving_app(None);
    snapshot_with_obstacle(&mut default_app, obstacle, 1.0);
    tick(&mut default_app);
    assert_eq!(
        lateral_intent(&mut default_app),
        0.0,
        "the default 3 s horizon projects only 30 units ahead — the obstacle at \
         100 is not yet a threat"
    );

    let mut authored_app = moving_app(Some(crate::entities::config::BehaviourConfig {
        avoidance_look_ahead_secs: 10.0,
        ..Default::default()
    }));
    snapshot_with_obstacle(&mut authored_app, obstacle, 1.0);
    tick(&mut authored_app);
    assert!(
        lateral_intent(&mut authored_app).abs() > 0.0,
        "a TOML-authored 10 s look-ahead projects 100 units ahead, onto the \
         obstacle; got no lateral thrust, so the system is still reading \
         crate::ai::AVOIDANCE_LOOK_AHEAD_SECS"
    );
}

/// Seeds a `WorldSnapshot` holding a single *moving* obstacle: a ship with
/// its own `yaw`/`forward_speed`, so the predictive projection folds the
/// obstacle's motion into the collision test (issue #743). `movable` is set
/// so the published fact matches a real moving hull.
fn snapshot_with_moving_obstacle(
    app: &mut App,
    position: [f32; 3],
    radius: f32,
    yaw: f32,
    forward_speed: f32,
) {
    app.insert_resource(crate::ai::server::WorldSnapshot {
        entities: vec![crate::ai::AiWorldEntity {
            uuid: uuid::Uuid::new_v4(),
            name: Some("raider".into()),
            position,
            faction: None,
            shields: None,
            hull_fraction: None,
            yaw: Some(yaw),
            radius,
            forward_speed,
            movable: true,
            dangerous: true,
            size_rating: radius,
            direct_fire_range: 0.0,
            weapon_arcs: Vec::new(),
        }],
    });
}

/// Give the subject ship a collision radius, so its `self_size_rating` is
/// nonzero and the authored ignore-smaller rule has a size to compare a
/// hazard against (issue #743). Without a collider the test ship rates 0 and
/// the rule can never fire.
fn set_ship_collider(app: &mut App, radius: f32) {
    let ship = find_ship_entity(app);
    app.world_mut()
        .entity_mut(ship)
        .insert(crate::entities::spawner::ColliderSection(
            crate::entities::config::ColliderConfig {
                shape: crate::entities::config::ColliderShape::Ball,
                radius,
                length: 0.0,
                half_height: None,
                // The subject is a ship, so it is a mobile contact in
                // anyone else's hazard picture (issue #958).
                movable: true,
            },
        ));
}

/// A static hazard on the starboard bow must push the lateral dodge to port
/// (negative): the shared hazard assessment's repulsion points away from the
/// obstacle, and the actuator follows it (issue #743). The obstacle sits
/// inside an authored 60-unit buffer.
#[test]
fn lateral_thrust_ai_dodges_static_hazard() {
    // Starboard bow (+X), dead-ahead-ish down -Z. Stationary ship, so the
    // obstacle's own (absent) motion cannot confound the sign.
    let obstacle = [4.0, 0.0, -40.0];
    let mut app = lateral_thrust_ai_app(Some(crate::entities::config::BehaviourConfig {
        avoidance_buffer: 60.0,
        ..Default::default()
    }));
    snapshot_with_obstacle(&mut app, obstacle, 1.0);
    tick_twice(&mut app);
    assert!(
        lateral_intent(&mut app) < 0.0,
        "a starboard-bow hazard must dodge to port (negative lateral); got {}",
        lateral_intent(&mut app)
    );
}

/// A moving hazard is handled through the same shared surface: an obstacle
/// that is out of range when static becomes a threat once its own forward
/// motion is projected into the collision test, and the lateral dodge fires
/// (issue #743).
#[test]
fn lateral_thrust_ai_dodges_moving_hazard() {
    // Stationary self at origin (its projection is fixed), obstacle 50 units
    // ahead on the starboard bow. Static, it is far outside the default
    // ~7-unit dodge radius; closing at 16 u/s (yaw = PI faces +Z, back
    // toward us) its 3 s projection lands ~2 units ahead — a real threat.
    let obstacle = [2.0, 0.0, -50.0];

    let mut static_app = lateral_thrust_ai_app(None);
    snapshot_with_obstacle(&mut static_app, obstacle, 1.0);
    tick_twice(&mut static_app);
    assert_eq!(
        lateral_intent(&mut static_app),
        0.0,
        "a static obstacle 50 units off is outside the default dodge radius"
    );

    let mut moving_app = lateral_thrust_ai_app(None);
    snapshot_with_moving_obstacle(&mut moving_app, obstacle, 1.0, std::f32::consts::PI, 16.0);
    tick_twice(&mut moving_app);
    assert!(
        lateral_intent(&mut moving_app) < 0.0,
        "the obstacle's own motion must bring it into collision and dodge to \
         port; got {}",
        lateral_intent(&mut moving_app)
    );
}

/// The authored `lateral_hazard_sensitivity` gates the response to the
/// shared hazard surface: an obstacle that dodges at the default sensitivity
/// produces no lateral thrust when the hull authors sensitivity 0, and a
/// wider authored sensitivity does not zero it (issue #743). This pins that
/// the actuator reads the shared surface scaled by its own authored weight.
#[test]
fn lateral_thrust_ai_responds_to_shared_hazard_surface() {
    let obstacle = [4.0, 0.0, -40.0];

    // Default sensitivity (1.0): the in-range obstacle dodges.
    let mut responsive = lateral_thrust_ai_app(Some(crate::entities::config::BehaviourConfig {
        avoidance_buffer: 60.0,
        ..Default::default()
    }));
    snapshot_with_obstacle(&mut responsive, obstacle, 1.0);
    tick_twice(&mut responsive);
    assert!(
        lateral_intent(&mut responsive).abs() > 0.0,
        "the shared hazard force must drive a dodge at the default sensitivity"
    );

    // Sensitivity 0: the same shared hazard force is weighted to nothing.
    let mut muted = lateral_thrust_ai_app(Some(crate::entities::config::BehaviourConfig {
        avoidance_buffer: 60.0,
        lateral_hazard_sensitivity: 0.0,
        ..Default::default()
    }));
    snapshot_with_obstacle(&mut muted, obstacle, 1.0);
    tick_twice(&mut muted);
    assert_eq!(
        lateral_intent(&mut muted),
        0.0,
        "an authored zero sensitivity must mute the response to the shared \
         hazard surface"
    );
}

/// The authored ignore-smaller rule reaches the lateral dodge — and stops at
/// mobile contacts. A large ship ignores a small *ship* below its own size
/// rating, so an obstacle that would otherwise dodge produces zero lateral
/// thrust (issue #743); an identically-placed, identically-rated *rock* is
/// still dodged, because static terrain cannot manoeuvre out of the way
/// (issue #958).
///
/// All three cases share one geometry and one authored buffer, so the only
/// variables are the ratio and the hazard's published `movable` fact.
#[test]
fn lateral_thrust_ai_ignores_small_ships_but_never_small_terrain() {
    // Obstacle inside an authored 60-unit buffer so it *is* a threat when
    // the ignore rule is off. Self rates size 10 (collider radius); the
    // obstacle rates 1.
    let obstacle = [4.0, 0.0, -40.0];
    let ignoring = || {
        Some(crate::entities::config::BehaviourConfig {
            avoidance_buffer: 60.0,
            // Ignore any MOBILE hazard whose size rating is below self's
            // (10 × 1.0 = 10); the obstacle rates 1.
            hazard_ignore_size_ratio: 1.0,
            ..Default::default()
        })
    };

    let mut dodges = lateral_thrust_ai_app(Some(crate::entities::config::BehaviourConfig {
        avoidance_buffer: 60.0,
        ..Default::default()
    }));
    set_ship_collider(&mut dodges, 10.0);
    snapshot_with_obstacle(&mut dodges, obstacle, 1.0);
    tick_twice(&mut dodges);
    assert!(
        lateral_intent(&mut dodges).abs() > 0.0,
        "with the ignore rule off, the in-range obstacle must dodge"
    );

    // A small SHIP: `snapshot_with_moving_obstacle` publishes `movable`, and
    // a zero forward speed keeps its projected position identical to the
    // rock's, so the geometry is unchanged.
    let mut ignores = lateral_thrust_ai_app(ignoring());
    set_ship_collider(&mut ignores, 10.0);
    snapshot_with_moving_obstacle(&mut ignores, obstacle, 1.0, 0.0, 0.0);
    tick_twice(&mut ignores);
    assert_eq!(
        lateral_intent(&mut ignores),
        0.0,
        "a SHIP smaller than self must be ignored under the authored rule"
    );

    // The same small hazard published as static terrain is still avoided.
    let mut terrain = lateral_thrust_ai_app(ignoring());
    set_ship_collider(&mut terrain, 10.0);
    snapshot_with_obstacle(&mut terrain, obstacle, 1.0);
    tick_twice(&mut terrain);
    assert!(
        lateral_intent(&mut terrain).abs() > 0.0,
        "static terrain below own size must still dodge — the ignore-smaller \
         rule is a mobile-contact rule (issue #958)"
    );
}

// ── Vertical thrust AI (issue #744) ──────────────────────────────────

fn capability_with_mode(
    mode: crate::entities::config::VerticalMovementMode,
    max_vertical_offset: f32,
) -> crate::entities::config::HelmCapabilityConfig {
    crate::entities::config::HelmCapabilityConfig {
        vertical_movement_mode: mode,
        max_vertical_offset,
        ..Default::default()
    }
}

/// Build an app whose ship runs AI vertical thrust under the given
/// capability. The helm proper stays human so only the vertical-thrust
/// operator can ever write the vertical axis (issue #744).
fn vertical_thrust_ai_app(
    capability: crate::entities::config::HelmCapabilityConfig,
    behaviour: Option<crate::entities::config::BehaviourConfig>,
) -> App {
    let mut app = test_app();
    set_helm_control_source(&mut app, ControlSource::Human);
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipSystemControlSources, With<Ship>>();
        for mut cs in q.iter_mut(app.world_mut()) {
            cs.0.set(
                crate::ship::system_registry::vertical_thrust_system_id(),
                ControlSource::Ai,
            );
        }
    }
    set_ship_blackboard_objectives(&mut app, vec![patrol_scored_objective(vec!["wp0"], 20.0)]);
    let ship = find_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(ship)
        .insert(crate::entities::spawner::HelmCapabilitySection(capability));
    if let Some(behaviour) = behaviour {
        set_behaviour_section(&mut app, behaviour);
    }
    app
}

fn vertical_intent(app: &mut App) -> f32 {
    app.world_mut()
        .query::<&VerticalThrustInput>()
        .single(app.world())
        .expect("ship must carry VerticalThrustInput")
        .0
}

/// The initial vertical policy filters to *moving* hazards: an in-range
/// static obstacle (which the planar actuators would still dodge) drives no
/// vertical thrust, while an in-range moving hazard makes the ship climb.
/// Both sit inside the same authored 60-unit buffer, so the only difference
/// is the `movable` fact (issue #744).
#[test]
fn vertical_thrust_ai_responds_to_moving_hazard_not_static() {
    let obstacle = [4.0, 0.0, -40.0];
    let behaviour = crate::entities::config::BehaviourConfig {
        avoidance_buffer: 60.0,
        ..Default::default()
    };

    // Static hazard, in range: no vertical response.
    let mut static_app = vertical_thrust_ai_app(
        capability_with_mode(crate::entities::config::VerticalMovementMode::Bounded, 30.0),
        Some(behaviour.clone()),
    );
    snapshot_with_obstacle(&mut static_app, obstacle, 1.0);
    tick_twice(&mut static_app);
    assert_eq!(
        vertical_intent(&mut static_app),
        0.0,
        "an in-range STATIC hazard must not drive vertical thrust"
    );

    // Moving hazard, same spot and range: the ship climbs to dodge.
    let mut moving_app = vertical_thrust_ai_app(
        capability_with_mode(crate::entities::config::VerticalMovementMode::Bounded, 30.0),
        Some(behaviour),
    );
    snapshot_with_moving_obstacle(&mut moving_app, obstacle, 1.0, 0.0, 0.0);
    tick_twice(&mut moving_app);
    assert!(
        vertical_intent(&mut moving_app) > 0.0,
        "an in-range MOVING hazard must drive a climb; got {}",
        vertical_intent(&mut moving_app)
    );
}

/// The authored `vertical_hazard_sensitivity` gates the response: sensitivity
/// 0 mutes the climb the default weight produces (issue #744).
#[test]
fn vertical_thrust_ai_honours_authored_sensitivity() {
    let obstacle = [4.0, 0.0, -40.0];

    let mut muted = vertical_thrust_ai_app(
        capability_with_mode(crate::entities::config::VerticalMovementMode::Bounded, 30.0),
        Some(crate::entities::config::BehaviourConfig {
            avoidance_buffer: 60.0,
            vertical_hazard_sensitivity: 0.0,
            ..Default::default()
        }),
    );
    snapshot_with_moving_obstacle(&mut muted, obstacle, 1.0, 0.0, 0.0);
    tick_twice(&mut muted);
    assert_eq!(
        vertical_intent(&mut muted),
        0.0,
        "authored zero vertical sensitivity must mute the climb"
    );
}

/// The three movement modes produce demonstrably divergent authoritative Y
/// motion under the same persistent moving hazard (issue #744): Planar holds
/// the cruise plane, Bounded climbs but is capped at its authored offset,
/// and Full3D keeps climbing past that cap.
#[test]
fn vertical_movement_modes_diverge_under_a_moving_hazard() {
    use crate::entities::config::VerticalMovementMode;
    let obstacle = [4.0, 0.0, -40.0];
    const BOUNDED_OFFSET: f32 = 2.0;

    fn final_y(mode: VerticalMovementMode, obstacle: [f32; 3], offset: f32) -> f32 {
        let mut app = vertical_thrust_ai_app(
            capability_with_mode(mode, offset),
            Some(crate::entities::config::BehaviourConfig {
                avoidance_buffer: 60.0,
                ..Default::default()
            }),
        );
        // A persistent, planar moving hazard: assess_hazards is planar, so it
        // stays a threat no matter how high the ship climbs.
        snapshot_with_moving_obstacle(&mut app, obstacle, 1.0, 0.0, 0.0);
        // The observation window, not the assertion, is what issue #968
        // changed here. This obstacle sits well inside its (authored, 60-unit)
        // avoidance buffer but nowhere near contact, and the severity ramp is
        // squared rather than linear from that issue on — so the same climb
        // takes longer to open the same gap. Sixty ticks was enough at the old
        // response strength; the claim under test is that Full3D keeps going
        // where Bounded is capped, and that is unchanged.
        for _ in 0..150 {
            tick(&mut app);
        }
        get_ship_physics(&mut app).y
    }

    let planar_y = final_y(VerticalMovementMode::Planar, obstacle, BOUNDED_OFFSET);
    let bounded_y = final_y(VerticalMovementMode::Bounded, obstacle, BOUNDED_OFFSET);
    let full3d_y = final_y(VerticalMovementMode::Full3D, obstacle, BOUNDED_OFFSET);

    assert!(
        planar_y.abs() < 0.01,
        "Planar hull must never leave the cruise plane, got y={planar_y}"
    );
    assert!(
        bounded_y > 0.5,
        "Bounded hull must climb to dodge, got y={bounded_y}"
    );
    assert!(
        bounded_y <= BOUNDED_OFFSET + 5.0,
        "Bounded hull must respect its authored max offset ({BOUNDED_OFFSET}), got y={bounded_y}"
    );
    assert!(
        full3d_y > bounded_y + 3.0,
        "Full3D hull must climb well past the bounded cap; bounded={bounded_y} full3d={full3d_y}"
    );
}

/// Bounded avoidance returns gradually to the cruise plane once the moving
/// hazard is gone (issue #744): the ship climbs while threatened, then eases
/// back toward y = 0 when the threat clears.
#[test]
fn bounded_vertical_returns_to_cruise_after_hazard_clears() {
    let obstacle = [4.0, 0.0, -40.0];
    let mut app = vertical_thrust_ai_app(
        capability_with_mode(crate::entities::config::VerticalMovementMode::Bounded, 30.0),
        Some(crate::entities::config::BehaviourConfig {
            avoidance_buffer: 60.0,
            ..Default::default()
        }),
    );
    snapshot_with_moving_obstacle(&mut app, obstacle, 1.0, 0.0, 0.0);
    for _ in 0..40 {
        tick(&mut app);
    }
    let climbed = get_ship_physics(&mut app).y;
    assert!(
        climbed > 1.0,
        "the ship must have climbed while threatened, got y={climbed}"
    );

    // Threat clears: the world is now empty.
    app.insert_resource(crate::ai::server::WorldSnapshot { entities: vec![] });
    for _ in 0..120 {
        tick(&mut app);
    }
    let returned = get_ship_physics(&mut app).y;
    assert!(
        returned < climbed - 0.5,
        "the ship must ease back toward cruise after the hazard clears; \
         climbed={climbed} returned={returned}"
    );
    assert!(
        returned < 1.0,
        "the ship must return close to the cruise plane, got y={returned}"
    );
}

// ── Secondary-actuator policy gate + fact seeding (issue #780) ───────────

fn set_vertical_ai_policy(app: &mut App, policy: crate::ai::policy::AiPolicy) {
    let ship = find_ship_entity(app);
    set_fine_policy(
        app,
        ship,
        crate::ship::system_registry::vertical_thrust_system_id(),
        policy,
    );
}

fn set_lateral_ai_policy(app: &mut App, policy: crate::ai::policy::AiPolicy) {
    let ship = find_ship_entity(app);
    set_fine_policy(
        app,
        ship,
        crate::ship::system_registry::lateral_thrust_system_id(),
        policy,
    );
}

/// A vertical policy that actuates only while the seeded `moving_hazard_threat`
/// fact exceeds an authored threshold — a `fact(...)`-referencing guard.
fn threat_gated_vertical_policy(threshold: f64) -> crate::ai::policy::AiPolicy {
    let mut params = crate::world::flags::AiParams::new();
    params.set("threshold", threshold);
    crate::ai::policy::AiPolicy {
        params,
        rules: vec![crate::ai::policy::AiPolicyRule {
            priority: 10,
            channel: crate::entities::config::HELM_VERTICAL_CHANNEL.into(),
            when: crate::world::flags::parse_predicate(
                "fact(moving_hazard_threat) > param(threshold)",
            )
            .unwrap(),
            verb: crate::ai::policy::AiPolicyVerb::ActuateVerticalThrust,
        }],
        idle: false,
        machine: None,
    }
}

/// THE #779 empty-facts sharp edge, resolved (issue #780). A vertical policy
/// whose guard references the seeded `moving_hazard_threat` fact must actually
/// FIRE — impossible before #780 because the helm hosts passed an empty
/// `AiFacts`. With no moving hazard the guard is false and the axis holds at
/// cruise; introduce a moving hazard and the same guard fires and the ship
/// climbs. Proves the host now seeds real hazard facts.
#[test]
fn vertical_fact_guard_fires_only_once_hazard_fact_is_seeded() {
    // Guard needs threat > 0.1. No hazard → fact seeds 0.0 → hold at cruise.
    let mut calm = vertical_thrust_ai_app(
        capability_with_mode(crate::entities::config::VerticalMovementMode::Bounded, 30.0),
        Some(crate::entities::config::BehaviourConfig {
            avoidance_buffer: 60.0,
            ..Default::default()
        }),
    );
    set_vertical_ai_policy(&mut calm, threat_gated_vertical_policy(0.1));
    app_empty_snapshot(&mut calm);
    tick_twice(&mut calm);
    assert_eq!(
        vertical_intent(&mut calm),
        0.0,
        "with no hazard the seeded moving_hazard_threat is 0, the guard is \
         false, and the vertical axis holds — the pre-#780 empty facts would \
         have made this guard un-fireable at all"
    );

    // Same policy, now a moving hazard seeds a nonzero threat → guard fires.
    let mut threatened = vertical_thrust_ai_app(
        capability_with_mode(crate::entities::config::VerticalMovementMode::Bounded, 30.0),
        Some(crate::entities::config::BehaviourConfig {
            avoidance_buffer: 60.0,
            ..Default::default()
        }),
    );
    set_vertical_ai_policy(&mut threatened, threat_gated_vertical_policy(0.1));
    snapshot_with_moving_obstacle(&mut threatened, [4.0, 0.0, -40.0], 1.0, 0.0, 0.0);
    tick_twice(&mut threatened);
    assert!(
        vertical_intent(&mut threatened) > 0.0,
        "a seeded moving_hazard_threat above the authored threshold must fire \
         the guard and climb; got {}",
        vertical_intent(&mut threatened)
    );
}

/// AC1/AC7 typed output + an authored idle/hold: a vertical policy that never
/// fires holds the axis, proving the actuator emits a TYPED VerticalThrustInput
/// only when its own channel resolves — not unconditionally.
#[test]
fn vertical_actuator_holds_under_never_firing_policy() {
    let mut app = vertical_thrust_ai_app(
        capability_with_mode(crate::entities::config::VerticalMovementMode::Bounded, 30.0),
        Some(crate::entities::config::BehaviourConfig {
            avoidance_buffer: 60.0,
            ..Default::default()
        }),
    );
    // Threshold above 1.0 can never be crossed by a [0,1] threat → never fires.
    set_vertical_ai_policy(&mut app, threat_gated_vertical_policy(2.0));
    snapshot_with_moving_obstacle(&mut app, [4.0, 0.0, -40.0], 1.0, 0.0, 0.0);
    tick_twice(&mut app);
    assert_eq!(
        vertical_intent(&mut app),
        0.0,
        "a policy that never fires must hold the vertical axis despite a live \
         moving hazard the default policy would climb for"
    );
}

/// AC3: ordinary avoidance BENDS travel without swapping the doctrine. A
/// lateral policy that never fires suppresses the dodge, proving the dodge
/// flows through the actuator gate — while the same tick's engine/steering
/// doctrine (a forward Reach) is untouched.
#[test]
fn lateral_actuator_holds_under_never_firing_policy() {
    let mut app = lateral_dodge_app();
    // A policy on the lateral channel that never fires.
    let never = crate::ai::policy::AiPolicy {
        params: crate::world::flags::AiParams::new(),
        rules: vec![crate::ai::policy::AiPolicyRule {
            priority: 10,
            channel: crate::entities::config::HELM_LATERAL_CHANNEL.into(),
            when: crate::world::flags::parse_predicate("fact(hazard_urgency) > 9.0").unwrap(),
            verb: crate::ai::policy::AiPolicyVerb::ActuateLateralThrust,
        }],
        idle: false,
        machine: None,
    };
    set_lateral_ai_policy(&mut app, never);
    tick_twice(&mut app);
    assert_eq!(
        lateral_intent(&mut app),
        0.0,
        "a never-firing lateral policy must hold the dodge even with a hazard \
         in range the default policy would dodge"
    );
}

fn app_empty_snapshot(app: &mut App) {
    app.insert_resource(crate::ai::server::WorldSnapshot { entities: vec![] });
}

/// Re-seed a `lateral_dodge_app`'s snapshot with its obstacle PLUS one armed
/// hostile whose published arcs either bear on us or do not, and wire the
/// factions so the reduction treats it as an enemy.
///
/// The hostile is deliberately not a hazard — zero radius, not `movable`,
/// not `dangerous` — so it contributes nothing to the hazard surface the
/// dodge magnitude is computed from. The ONLY thing `bearing` changes is the
/// arc fact, which is what makes the assertions below attributable.
fn snapshot_with_obstacle_and_hostile(app: &mut App, bearing: bool) {
    let hostile_faction = uuid::Uuid::new_v4();
    let own_faction = uuid::Uuid::new_v4();
    let mut registry = crate::ai::faction::FactionRegistry::new();
    registry.insert(crate::ai::faction::FactionConfig {
        display_name: None,
        uuid: own_faction,
        name: "Own".into(),
        enemies: vec![hostile_faction],
        compliance: None,
    });
    app.insert_resource(crate::entities::config_cache::FactionRegistryResource(
        registry,
    ));
    let ship = find_ship_entity(app);
    app.world_mut()
        .entity_mut(ship)
        .insert(crate::entities::spawner::FactionComponent(own_faction));

    // The hostile sits 80 units astern of us, so the world bearing from it
    // to us is 0. A bank centred on 0 covers us; the same bank turned to
    // 180 points away and covers nothing.
    let arcs = vec![crate::weapons::arc_geometry::WeaponArcSector {
        bearing_deg: if bearing { 0.0 } else { 180.0 },
        half_angle_deg: 30.0,
        range: 200.0,
    }];
    app.insert_resource(crate::ai::server::WorldSnapshot {
        entities: vec![
            crate::ai::AiWorldEntity {
                uuid: uuid::Uuid::new_v4(),
                name: Some("rock".into()),
                position: [4.0, 0.0, -40.0],
                faction: None,
                shields: None,
                hull_fraction: None,
                yaw: None,
                radius: 1.0,
                forward_speed: 0.0,
                movable: false,
                dangerous: true,
                size_rating: 1.0,
                direct_fire_range: 0.0,
                weapon_arcs: Vec::new(),
            },
            crate::ai::AiWorldEntity {
                uuid: uuid::Uuid::new_v4(),
                name: Some("raider".into()),
                position: [0.0, 0.0, 80.0],
                faction: Some(hostile_faction),
                shields: None,
                hull_fraction: None,
                yaw: Some(0.0),
                radius: 0.0,
                forward_speed: 0.0,
                movable: false,
                dangerous: false,
                size_rating: 0.0,
                direct_fire_range: 200.0,
                weapon_arcs: arcs,
            },
        ],
    });
}

/// A lateral policy whose ONLY guard is the #874 exposure fact.
fn arc_gated_lateral_policy() -> crate::ai::policy::AiPolicy {
    crate::ai::policy::AiPolicy {
        params: crate::world::flags::AiParams::new(),
        rules: vec![crate::ai::policy::AiPolicyRule {
            priority: 10,
            channel: crate::entities::config::HELM_LATERAL_CHANNEL.into(),
            when: crate::world::flags::parse_predicate("fact(hostile_arc_exposure) > 0").unwrap(),
            verb: crate::ai::policy::AiPolicyVerb::ActuateLateralThrust,
        }],
        idle: false,
        machine: None,
    }
}

/// The seeding gap the SCOPE note used to paper over: `ai_helm_lateral_thrust`
/// seeds `seed_helm_actuator_facts`, NOT `seed_helm_travel_facts`, so before
/// `seed_hostile_arc_facts` was split out and called here a
/// `[helm_console.lateral_ai]` guard on `hostile_arc_exposure` validated at
/// load and then read absent for ever (the #779 shape).
///
/// Lateral is the literal dodge axis, so it is the first axis a #877 dodging
/// doctrine will author this fact on. Asserted through the axis's own
/// observable output — a policy that actuates ONLY when arcs bear.
#[test]
fn hostile_arc_exposure_is_readable_from_a_lateral_axis_policy_guard() {
    let mut bearing = lateral_dodge_app();
    snapshot_with_obstacle_and_hostile(&mut bearing, true);
    set_lateral_ai_policy(&mut bearing, arc_gated_lateral_policy());
    tick_twice(&mut bearing);
    assert!(
        lateral_intent(&mut bearing).abs() > 0.0,
        "a lateral policy guarded on fact(hostile_arc_exposure) must actuate \
         while a hostile's arcs bear; zero means the fact never reached this \
         host's snapshot and the guard read absent"
    );

    let mut clear = lateral_dodge_app();
    snapshot_with_obstacle_and_hostile(&mut clear, false);
    set_lateral_ai_policy(&mut clear, arc_gated_lateral_policy());
    tick_twice(&mut clear);
    assert_eq!(
        lateral_intent(&mut clear),
        0.0,
        "the same policy must hold when the same hostile's arcs point away — \
         otherwise the first assertion proves only that the axis always \
         actuates, not that it read the fact"
    );
}

fn set_impulse_ai_policy(app: &mut App, policy: crate::ai::policy::AiPolicy) {
    let ship = find_ship_entity(app);
    set_fine_policy(
        app,
        ship,
        crate::ship::system_registry::helm_impulse_system_id(),
        policy,
    );
}

/// A vertical policy whose ONLY guard is the #874 exposure fact.
fn arc_gated_vertical_policy() -> crate::ai::policy::AiPolicy {
    crate::ai::policy::AiPolicy {
        params: crate::world::flags::AiParams::new(),
        rules: vec![crate::ai::policy::AiPolicyRule {
            priority: 10,
            channel: crate::entities::config::HELM_VERTICAL_CHANNEL.into(),
            when: crate::world::flags::parse_predicate("fact(hostile_arc_exposure) > 0").unwrap(),
            verb: crate::ai::policy::AiPolicyVerb::ActuateVerticalThrust,
        }],
        idle: false,
        machine: None,
    }
}

/// An impulse policy whose ONLY guard is the #874 exposure fact.
fn arc_gated_impulse_policy() -> crate::ai::policy::AiPolicy {
    crate::ai::policy::AiPolicy {
        params: crate::world::flags::AiParams::new(),
        rules: vec![crate::ai::policy::AiPolicyRule {
            priority: 10,
            channel: crate::entities::config::HELM_IMPULSE_CHANNEL.into(),
            when: crate::world::flags::parse_predicate("fact(hostile_arc_exposure) > 0").unwrap(),
            verb: crate::ai::policy::AiPolicyVerb::EngageImpulse,
        }],
        idle: false,
        machine: None,
    }
}

/// The lateral sibling above, applied to the vertical axis. `ai_helm_vertical
/// _thrust` calls `seed_hostile_arc_facts` directly (it seeds
/// `seed_helm_actuator_facts`, not `seed_helm_travel_facts`), so the same
/// #779 shape was available here — the SCOPE note claims seven hosts and
/// inspection is not what keeps that claim true.
///
/// Observed through the axis's own output rather than the fact table.
/// Vertical magnitude is driven by the moving-hazard threat, and the rock in
/// this snapshot is static, so the readable difference is the Bounded
/// return-to-cruise: put the ship off the cruise plane, and a tick on which
/// the policy actuates eases it back (negative) while a tick on which the
/// guard holds emits nothing at all (zero).
#[test]
fn hostile_arc_exposure_is_readable_from_a_vertical_axis_policy_guard() {
    let off_cruise = |app: &mut App| {
        let ship = find_ship_entity(app);
        app.world_mut()
            .entity_mut(ship)
            .get_mut::<ShipPhysics>()
            .expect("ship must carry ShipPhysics")
            .y = 5.0;
    };
    let vertical_app = || {
        vertical_thrust_ai_app(
            capability_with_mode(crate::entities::config::VerticalMovementMode::Bounded, 30.0),
            Some(crate::entities::config::BehaviourConfig {
                avoidance_buffer: 60.0,
                ..Default::default()
            }),
        )
    };

    let mut bearing = vertical_app();
    snapshot_with_obstacle_and_hostile(&mut bearing, true);
    off_cruise(&mut bearing);
    set_vertical_ai_policy(&mut bearing, arc_gated_vertical_policy());
    tick_twice(&mut bearing);
    assert!(
        vertical_intent(&mut bearing) < 0.0,
        "a vertical policy guarded on fact(hostile_arc_exposure) must actuate \
         while a hostile's arcs bear; zero means the fact never reached this \
         host's snapshot and the guard read absent"
    );

    let mut clear = vertical_app();
    snapshot_with_obstacle_and_hostile(&mut clear, false);
    off_cruise(&mut clear);
    set_vertical_ai_policy(&mut clear, arc_gated_vertical_policy());
    tick_twice(&mut clear);
    assert_eq!(
        vertical_intent(&mut clear),
        0.0,
        "the same policy must hold when the same hostile's arcs point away — \
         otherwise the first assertion proves only that the axis always \
         actuates, not that it read the fact"
    );
}

/// The same guard on the impulse axis, the third host that seeds these facts
/// directly. Geometry is
/// `ai_helm_impulse_engages_toward_a_distant_target_ahead`'s — an anchor
/// dead ahead at 500 units, so the manoeuvre decision is `Engage` and the
/// ONLY thing standing between that and a `Charging` command is whether the
/// arc guard resolved.
#[test]
fn hostile_arc_exposure_is_readable_from_an_impulse_axis_policy_guard() {
    let anchor = "station-alpha";

    let mut bearing = impulse_ai_app(reach_scored_objective(anchor, 10.0));
    bearing.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, -500.0]));
    snapshot_with_obstacle_and_hostile(&mut bearing, true);
    set_impulse_ai_policy(&mut bearing, arc_gated_impulse_policy());
    tick(&mut bearing);
    assert_eq!(
        get_impulse_command(&mut bearing),
        crate::ship::impulse::ImpulsePhase::Charging,
        "an impulse policy guarded on fact(hostile_arc_exposure) must permit \
         the engage while a hostile's arcs bear; Idle means the fact never \
         reached this host's snapshot and the guard read absent"
    );

    let mut clear = impulse_ai_app(reach_scored_objective(anchor, 10.0));
    clear.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, -500.0]));
    snapshot_with_obstacle_and_hostile(&mut clear, false);
    set_impulse_ai_policy(&mut clear, arc_gated_impulse_policy());
    tick(&mut clear);
    assert_eq!(
        get_impulse_command(&mut clear),
        crate::ship::impulse::ImpulsePhase::Idle,
        "the same policy must hold the drive when the same hostile's arcs \
         point away — otherwise the first assertion proves only that the \
         geometry engages, not that the guard read the fact"
    );
}

// ── Hostile weapon-arc facts (issue #874) ───────────────────────────────
//
// The reduction itself, and its hostility gate, are covered end-to-end in
// `ai::server`'s snapshot tests. What these prove is the SEEDING seam: that
// the three names a doctrine may author actually reach the fact snapshot the
// per-axis hosts resolve guards against — the #779 hole where a guard
// validated at load and then never fired.

fn arc_facts_from(frame: &HelmAiShipFrame) -> crate::world::flags::AiFacts {
    let mut facts = crate::world::flags::AiFacts::new();
    seed_helm_travel_facts(&mut facts, Some(frame), &ShipPhysics::default(), 30.0);
    facts
}

fn guard_holds(facts: &crate::world::flags::AiFacts, predicate: &str) -> bool {
    crate::world::flags::parse_predicate(predicate)
        .expect("guard must parse")
        .evaluate_with(facts, &crate::world::flags::AiParams::new(), &[])
}

// ── The movement POSTURE fact (issue #875) ───────────────────────────────

/// `seed_helm_actuator_facts` with nothing but the alert reading, which is
/// the only input the posture derivation has.
fn actuator_facts_at_alert(red_alert: bool) -> crate::world::flags::AiFacts {
    seed_helm_actuator_facts(None, false, false, 0.0, red_alert)
}

/// **The host-side half of AC2.** `posture` is really seeded from the ship's
/// own red alert, in BOTH directions, and it is PRESENT when the alert is
/// clear rather than absent.
///
/// The absent case is the whole point. An absent fact makes every comparison
/// against it false, so a misspelled name — here or in the authored guard —
/// presents exactly like a permanently defensive hull, with nothing else
/// going red. `authored_ai_pins::posture_guard_truth_table` is the content
/// half; this is the seam.
#[test]
fn posture_is_seeded_unconditionally_from_red_alert() {
    let pressed = actuator_facts_at_alert(true);
    assert_eq!(
        pressed.get(POSTURE_FACT),
        Some(POSTURE_PRESSED),
        "at red alert the ship's movement posture is PRESSED"
    );
    assert!(guard_holds(&pressed, "fact(posture) >= 1"));

    let clear = actuator_facts_at_alert(false);
    assert_eq!(
        clear.get(POSTURE_FACT),
        Some(POSTURE_DEFENSIVE),
        "alert clear ⇒ seeded at the DEFENSIVE rung, not left absent — an \
         absent fact makes every comparison read false, which hides the \
         difference between 'stood down' and 'never wired up'"
    );
    assert!(!guard_holds(&clear, "fact(posture) >= 1"));
    assert!(
        guard_holds(&clear, "fact(posture) < 1"),
        "and the break-off guard, which is a `<` and therefore the one an \
         ABSENT fact would silently disable, must read true"
    );
}

/// The posture reading a host uses comes off the shared frame, so all seven
/// hosts see the same one — and a ship with no frame entry reads defensive
/// rather than panicking or inventing.
#[test]
fn posture_is_read_off_the_shared_frame() {
    assert!(frame_red_alert(Some(&HelmAiShipFrame {
        red_alert: true,
        ..Default::default()
    })));
    assert!(!frame_red_alert(Some(&HelmAiShipFrame::default())));
    assert!(
        !frame_red_alert(None),
        "no frame entry ⇒ no AI-operated helm axis ⇒ defensive is the honest \
         reading; the fact is still SET, which is what matters"
    );
}

/// **Every one of the seven policy hosts seeds a REAL posture reading.**
///
/// The compiler already forces all seven to pass something —
/// `seed_helm_actuator_facts` takes `red_alert` by value, and it is the one
/// seeder every host calls. What it cannot catch is a site that passes a
/// hardcoded `false`, which is the #779 failure in its most plausible form:
/// a new host added later, wired up mechanically, that compiles and runs and
/// leaves exactly one axis permanently defensive while the other six press.
///
/// So this scans this module's own source for the call sites and requires
/// each to pass one of the two honest readings. Deliberately a source scan
/// rather than a per-host integration test: a scan covers hosts added after
/// it was written, which is the case that matters.
#[test]
fn every_helm_policy_host_seeds_a_real_posture_reading() {
    const CALL: &str = "seed_helm_actuator_facts(";
    // The two honest readings: off the shared frame, or off a frame entry
    // the host already holds.
    const HONEST: [&str; 2] = ["frame_red_alert(", "sf.red_alert"];

    // The seven call sites are the shared `ai_policy_state_tick` (mod.rs) plus
    // the six per-axis actuators — one per per-host file since issue #1206 split
    // the monolith into `src/ship/helm_ai/`. Scan the PRODUCTION half of every
    // module file; only mod.rs carries the inline `mod tests`, cut at its
    // barrier (the test module holds the literal in this test's own `CALL`
    // constant and in a fixture helper, neither of which is a policy host).
    const FILES: [&str; 9] = [
        "src/ship/helm_ai/mod.rs",
        "src/ship/helm_ai/surfaces.rs",
        "src/ship/helm_ai/facts.rs",
        "src/ship/helm_ai/engines.rs",
        "src/ship/helm_ai/steering.rs",
        "src/ship/helm_ai/impulse.rs",
        "src/ship/helm_ai/lateral.rs",
        "src/ship/helm_ai/vertical.rs",
        "src/ship/helm_ai/boost.rs",
    ];
    let mut whole = String::new();
    for path in FILES {
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("helm_ai source {path} must be readable: {e}"));
        let prod = match text.find("\nmod tests {") {
            Some(cut) => &text[..cut],
            None => &text,
        };
        whole.push_str(prod);
        whole.push('\n');
    }
    let src = whole.as_str();
    let mut sites = 0usize;
    let mut cursor = 0usize;
    while let Some(i) = src[cursor..].find(CALL) {
        let at = cursor + i;
        cursor = at + CALL.len();
        // The DEFINITION matches the literal too, and its parameter list
        // names `red_alert` without reading one.
        if src[..at].ends_with("fn ") {
            continue;
        }
        // Balanced-paren scan for the argument list: several sites pass a
        // closure (`.map(|sp| &sp.hazard)`), so the first `)` is not the
        // end of the call.
        let mut depth = 1usize;
        let mut end = cursor;
        for (off, ch) in src[cursor..].char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = cursor + off;
                        break;
                    }
                }
                _ => {}
            }
        }
        assert!(end > cursor, "unbalanced call at byte {at}");
        let args = &src[cursor..end];
        sites += 1;
        assert!(
            HONEST.iter().any(|h| args.contains(h)),
            "a `seed_helm_actuator_facts` call site passes no honest posture \
             reading. Every helm policy host must seed `posture` from the \
             ship's real alert state — a hardcoded `false` compiles, runs, and \
             leaves that one axis permanently defensive while the others \
             press. Args were:\n{args}"
        );
    }
    assert_eq!(
        sites, 7,
        "the scan found {sites} helm policy hosts, not the seven this module \
         declares (`ai_policy_state_tick` plus the six per-axis actuators). \
         Either a host was added or removed, or the scan has stopped parsing."
    );
}

/// The exposure fact FIRES: a frame carrying covering arcs seeds a count a
/// `fact(hostile_arc_exposure) > 0` guard reads true.
#[test]
fn hostile_arc_exposure_fact_fires_when_arcs_bear() {
    let frame = HelmAiShipFrame {
        hostile_arc_exposure: crate::weapons::arc_geometry::ArcExposure {
            covering_count: 2,
            escape_offset_deg: -35.0,
            inescapable: false,
        },
        ..Default::default()
    };
    let facts = arc_facts_from(&frame);
    assert_eq!(facts.get(HOSTILE_ARC_EXPOSURE_FACT), Some(2.0));
    assert!(guard_holds(&facts, "fact(hostile_arc_exposure) > 0"));
    assert!(guard_holds(&facts, "fact(hostile_arc_escape_deg) < 0"));
}

/// The never-firing twin: nothing bearing seeds `0.0` — PRESENT and false,
/// not absent — so the same guard reads false rather than reading nothing.
#[test]
fn hostile_arc_exposure_fact_reads_false_when_nothing_bears() {
    let frame = HelmAiShipFrame::default();
    let facts = arc_facts_from(&frame);
    assert_eq!(
        facts.get(HOSTILE_ARC_EXPOSURE_FACT),
        Some(0.0),
        "seeded at zero, not left absent — an absent fact makes every \
         comparison read false, which hides the difference between 'clear' \
         and 'never wired up'"
    );
    assert_eq!(facts.get(HOSTILE_ARC_ESCAPE_DEG_FACT), Some(0.0));
    assert!(!guard_holds(&facts, "fact(hostile_arc_exposure) > 0"));
}

/// The arc facts are seeded independently of target resolution: a ship with
/// no target at all is still told it is being borne on. A dodging policy
/// that only reacted to its own target would fly through a third ship's
/// broadside.
#[test]
fn hostile_arc_facts_are_seeded_without_a_target() {
    let frame = HelmAiShipFrame {
        hostile_arc_exposure: crate::weapons::arc_geometry::ArcExposure {
            covering_count: 1,
            escape_offset_deg: 20.0,
            inescapable: false,
        },
        ..Default::default()
    };
    let facts = arc_facts_from(&frame);
    assert_eq!(
        facts.get(TARGET_VALID_FACT),
        Some(0.0),
        "fixture must have no target, or this proves nothing"
    );
    assert_eq!(facts.get(HOSTILE_ARC_EXPOSURE_FACT), Some(1.0));
    assert_eq!(facts.get(HOSTILE_ARC_ESCAPE_DEG_FACT), Some(20.0));
}

/// The escape offset keeps its SIGN through the seam — it is a direction,
/// and a policy that lost the sign could only thrash.
#[test]
fn hostile_arc_escape_fact_keeps_its_direction() {
    for offset in [-90.0_f32, -5.0, 5.0, 90.0] {
        let frame = HelmAiShipFrame {
            hostile_arc_exposure: crate::weapons::arc_geometry::ArcExposure {
                covering_count: 1,
                escape_offset_deg: offset,
                inescapable: false,
            },
            ..Default::default()
        };
        let facts = arc_facts_from(&frame);
        assert_eq!(
            facts.get(HOSTILE_ARC_ESCAPE_DEG_FACT),
            Some(offset as f64),
            "offset {offset}"
        );
    }
}

/// The THIRD reading reaches the snapshot as its own fact: "I cannot turn
/// out of this" and "nothing bears on me" both read
/// `hostile_arc_escape_deg == 0`, so without this flag a #877 dodging
/// doctrine could not tell them apart — and would come about for ever
/// against a hull whose banks cover every bearing.
#[test]
fn hostile_arc_inescapable_fact_separates_trapped_from_clear() {
    let trapped = arc_facts_from(&HelmAiShipFrame {
        hostile_arc_exposure: crate::weapons::arc_geometry::ArcExposure {
            covering_count: 2,
            escape_offset_deg: 0.0,
            inescapable: true,
        },
        ..Default::default()
    });
    let clear = arc_facts_from(&HelmAiShipFrame::default());
    assert_eq!(
        trapped.get(HOSTILE_ARC_ESCAPE_DEG_FACT),
        clear.get(HOSTILE_ARC_ESCAPE_DEG_FACT),
        "precondition: the escape magnitude alone cannot separate the two"
    );
    assert_eq!(trapped.get(HOSTILE_ARC_INESCAPABLE_FACT), Some(1.0));
    assert_eq!(clear.get(HOSTILE_ARC_INESCAPABLE_FACT), Some(0.0));
    assert!(guard_holds(&trapped, "fact(hostile_arc_inescapable) > 0"));
    assert!(!guard_holds(&clear, "fact(hostile_arc_inescapable) > 0"));
    // And the guard that separates trapped from clear is the pair, which is
    // the reading a "break contact" doctrine actually branches on.
    assert!(guard_holds(&trapped, "fact(hostile_arc_exposure) > 0"));
    assert!(!guard_holds(&clear, "fact(hostile_arc_exposure) > 0"));
}

/// An ESCAPABLE covering arc is the third state again from the other side:
/// exposed, with a real magnitude, and the flag down.
#[test]
fn hostile_arc_inescapable_fact_stays_down_for_an_escapable_arc() {
    let facts = arc_facts_from(&HelmAiShipFrame {
        hostile_arc_exposure: crate::weapons::arc_geometry::ArcExposure {
            covering_count: 1,
            escape_offset_deg: -35.0,
            inescapable: false,
        },
        ..Default::default()
    });
    assert_eq!(facts.get(HOSTILE_ARC_INESCAPABLE_FACT), Some(0.0));
    assert_eq!(facts.get(HOSTILE_ARC_ESCAPE_DEG_FACT), Some(-35.0));
}

// ── Boost AI operator (issue #780) ───────────────────────────────────────

fn boost_command(app: &mut App) -> bool {
    app.world_mut()
        .query::<&crate::ship::helm::BoostCommand>()
        .single(app.world())
        .expect("ship must carry BoostCommand")
        .0
}

/// Build an app whose ship runs AI boost. Boost feature enabled, helm-boost
/// on AI, an objective + a moving hazard so the plan carries urgency.
fn boost_ai_app(policy: Option<crate::ai::policy::AiPolicy>) -> App {
    let mut app = test_app();
    // Full helm on AI: this puts a travel axis on AI so the shared frame +
    // hazard plan are built (the frame is gated on any of
    // thrust/steering/lateral/vertical/impulse being AI, not boost), and it
    // puts helm-boost on AI so the boost operator runs.
    set_helm_control_source(&mut app, ControlSource::Ai);
    set_ship_blackboard_objectives(&mut app, vec![patrol_scored_objective(vec!["wp0"], 20.0)]);
    set_behaviour_section(
        &mut app,
        crate::entities::config::BehaviourConfig {
            avoidance_buffer: 60.0,
            ..Default::default()
        },
    );
    let ship = find_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(ship)
        .insert(crate::ship::components::BoostConfigResource {
            enabled: true,
            ..Default::default()
        });
    if let Some(policy) = policy {
        set_fine_policy(
            &mut app,
            ship,
            crate::ship::system_registry::helm_boost_system_id(),
            policy,
        );
    }
    snapshot_with_moving_obstacle(&mut app, [4.0, 0.0, -40.0], 1.0, 0.0, 0.0);
    app
}

/// A boost policy that engages while the seeded hazard-urgency fact is above a
/// threshold and boost is available.
fn hazard_boost_policy() -> crate::ai::policy::AiPolicy {
    crate::ai::policy::AiPolicy {
        params: crate::world::flags::AiParams::new(),
        rules: vec![crate::ai::policy::AiPolicyRule {
            priority: 10,
            channel: crate::entities::config::HELM_BOOST_CHANNEL.into(),
            when: crate::world::flags::parse_predicate(
                "fact(hazard_urgency) > 0.0 and fact(boost_available) > 0",
            )
            .unwrap(),
            verb: crate::ai::policy::AiPolicyVerb::EngageBoost,
        }],
        idle: false,
        machine: None,
    }
}

/// AC1/AC6: `ai_helm_boost` emits a typed `SetBoost` through the same admitted
/// seam a human uses, engaging boost when its authored policy fires and the
/// feature is available.
#[test]
fn ai_helm_boost_engages_under_authored_hazard_policy() {
    let mut app = boost_ai_app(Some(hazard_boost_policy()));
    tick_twice(&mut app);
    assert!(
        boost_command(&mut app),
        "an authored boost policy firing on the seeded hazard fact must engage \
         boost through the admitted SetBoost seam"
    );
}

/// AC6 availability/capability filtering: with the boost feature absent, the
/// operator stands down and boost never engages, however urgent the hazard —
/// even under the same policy that engages it when available.
#[test]
fn ai_helm_boost_stands_down_without_boost_config() {
    let mut app = boost_ai_app(Some(hazard_boost_policy()));
    // Strip the boost capability.
    let ship = find_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(ship)
        .remove::<crate::ship::components::BoostConfigResource>();
    tick_twice(&mut app);
    assert!(
        !boost_command(&mut app),
        "no BoostConfigResource means no boost capability — the operator must \
         emit nothing"
    );
}

/// Baseline preservation (issue #780): the canonical default boost policy is
/// idle, so a ship that authors no `[helm_console.boost_ai]` never AI-boosts,
/// exactly as before #780 — even with the feature enabled and a live hazard.
#[test]
fn ai_helm_boost_default_idle_never_engages() {
    // No policy component → the host falls back to the idle default.
    let mut app = boost_ai_app(None);
    tick_twice(&mut app);
    assert!(
        !boost_command(&mut app),
        "the default idle boost policy must never engage boost (the pre-#780 \
         baseline: no AI boost)"
    );
}

// ── World-flag chains on the six helm hosts (issue #891 stage 2) ─────────

/// A stateless policy whose ONLY guard is the world flag `scenario_go`, on
/// `channel` resolving `verb`.
fn flag_gated_helm_policy(
    channel: &str,
    verb: crate::ai::policy::AiPolicyVerb,
) -> crate::ai::policy::AiPolicy {
    crate::ai::policy::AiPolicy {
        params: crate::world::flags::AiParams::new(),
        rules: vec![crate::ai::policy::AiPolicyRule {
            priority: 10,
            channel: channel.into(),
            when: crate::world::flags::parse_predicate("flag(scenario_go)").unwrap(),
            verb,
        }],
        idle: false,
        machine: None,
    }
}

/// Ensure the world runtime exists and set the `scenario_go` flag.
fn raise_scenario_go(app: &mut App) {
    app.init_resource::<crate::world::server::WorldContentRuntime>();
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .flags
        .set_flag("scenario_go");
}

/// Issue #891 stage 2, both-directions proof for the Helm ENGINES host: a
/// `flag()` guard on the longitudinal channel holds thrust while the
/// scenario flag is clear and actuates once it is set.
#[test]
fn helm_engines_flag_guard_reads_the_world_in_both_directions() {
    let build = || {
        let mut app = test_app();
        let anchor = "station-alpha";
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        set_per_axis_helm_ai(&mut app);
        app.init_resource::<crate::world::server::WorldContentRuntime>();
        let ship = find_ship_entity(&mut app);
        set_fine_policy(
            &mut app,
            ship,
            crate::ship::system_registry::helm_thrust_system_id(),
            flag_gated_helm_policy(
                crate::entities::config::HELM_LONGITUDINAL_CHANNEL,
                crate::ai::policy::AiPolicyVerb::ActuateDesiredTravel,
            ),
        );
        app
    };

    let mut held = build();
    tick(&mut held);
    assert_eq!(
        get_thrust_input(&mut held),
        0.0,
        "with the world flag clear the engines guard must read false and hold"
    );

    let mut fired = build();
    raise_scenario_go(&mut fired);
    tick(&mut fired);
    assert!(
        get_thrust_input(&mut fired) > 0.0,
        "with the world flag set the same engines guard must actuate thrust"
    );
}

/// Issue #891 stage 2, both-directions proof for the Helm STEERING host.
#[test]
fn helm_steering_flag_guard_reads_the_world_in_both_directions() {
    let build = || {
        let mut app = test_app();
        let anchor = "station-alpha";
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        set_per_axis_helm_ai(&mut app);
        app.init_resource::<crate::world::server::WorldContentRuntime>();
        let ship = find_ship_entity(&mut app);
        set_fine_policy(
            &mut app,
            ship,
            crate::ship::system_registry::helm_steering_system_id(),
            flag_gated_helm_policy(
                crate::entities::config::HELM_YAW_CHANNEL,
                crate::ai::policy::AiPolicyVerb::ActuateDesiredFacing,
            ),
        );
        app
    };

    let mut held = build();
    tick(&mut held);
    assert_eq!(
        get_steering_input(&mut held),
        0.0,
        "with the world flag clear the steering guard must read false and hold"
    );

    let mut fired = build();
    raise_scenario_go(&mut fired);
    tick(&mut fired);
    assert!(
        get_steering_input(&mut fired) > 0.0,
        "with the world flag set the same steering guard must actuate the turn"
    );
}

/// Issue #891 stage 2, both-directions proof for the Helm LATERAL host: the
/// dodge a moving hazard would drive is gated on the world flag.
#[test]
fn helm_lateral_flag_guard_reads_the_world_in_both_directions() {
    let mut held = lateral_dodge_app();
    held.init_resource::<crate::world::server::WorldContentRuntime>();
    set_lateral_ai_policy(
        &mut held,
        flag_gated_helm_policy(
            crate::entities::config::HELM_LATERAL_CHANNEL,
            crate::ai::policy::AiPolicyVerb::ActuateLateralThrust,
        ),
    );
    tick_twice(&mut held);
    assert_eq!(
        lateral_intent(&mut held),
        0.0,
        "with the world flag clear the lateral guard must read false and hold"
    );

    let mut fired = lateral_dodge_app();
    raise_scenario_go(&mut fired);
    set_lateral_ai_policy(
        &mut fired,
        flag_gated_helm_policy(
            crate::entities::config::HELM_LATERAL_CHANNEL,
            crate::ai::policy::AiPolicyVerb::ActuateLateralThrust,
        ),
    );
    tick_twice(&mut fired);
    assert!(
        lateral_intent(&mut fired) != 0.0,
        "with the world flag set the same lateral guard must actuate the dodge"
    );
}

/// Issue #891 stage 2, both-directions proof for the Helm VERTICAL host:
/// the climb a moving hazard would drive is gated on the world flag.
#[test]
fn helm_vertical_flag_guard_reads_the_world_in_both_directions() {
    let vertical_app = || {
        vertical_thrust_ai_app(
            capability_with_mode(crate::entities::config::VerticalMovementMode::Bounded, 30.0),
            Some(crate::entities::config::BehaviourConfig {
                avoidance_buffer: 60.0,
                ..Default::default()
            }),
        )
    };

    let mut held = vertical_app();
    held.init_resource::<crate::world::server::WorldContentRuntime>();
    set_vertical_ai_policy(
        &mut held,
        flag_gated_helm_policy(
            crate::entities::config::HELM_VERTICAL_CHANNEL,
            crate::ai::policy::AiPolicyVerb::ActuateVerticalThrust,
        ),
    );
    snapshot_with_moving_obstacle(&mut held, [4.0, 0.0, -40.0], 1.0, 0.0, 0.0);
    tick_twice(&mut held);
    assert_eq!(
        vertical_intent(&mut held),
        0.0,
        "with the world flag clear the vertical guard must read false and hold"
    );

    let mut fired = vertical_app();
    raise_scenario_go(&mut fired);
    set_vertical_ai_policy(
        &mut fired,
        flag_gated_helm_policy(
            crate::entities::config::HELM_VERTICAL_CHANNEL,
            crate::ai::policy::AiPolicyVerb::ActuateVerticalThrust,
        ),
    );
    snapshot_with_moving_obstacle(&mut fired, [4.0, 0.0, -40.0], 1.0, 0.0, 0.0);
    tick_twice(&mut fired);
    assert!(
        vertical_intent(&mut fired) > 0.0,
        "with the world flag set the same vertical guard must climb"
    );
}

/// Issue #891 stage 2, both-directions proof for the Helm IMPULSE host:
/// the charge toward a distant anchor is gated on the world flag.
#[test]
fn helm_impulse_flag_guard_reads_the_world_in_both_directions() {
    let anchor = "station-alpha";

    let mut held = impulse_ai_app(reach_scored_objective(anchor, 10.0));
    held.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, -500.0]));
    held.init_resource::<crate::world::server::WorldContentRuntime>();
    set_impulse_ai_policy(
        &mut held,
        flag_gated_helm_policy(
            crate::entities::config::HELM_IMPULSE_CHANNEL,
            crate::ai::policy::AiPolicyVerb::EngageImpulse,
        ),
    );
    tick(&mut held);
    assert_eq!(
        get_impulse_command(&mut held),
        crate::ship::impulse::ImpulsePhase::Idle,
        "with the world flag clear the impulse guard must read false and hold"
    );

    let mut fired = impulse_ai_app(reach_scored_objective(anchor, 10.0));
    fired.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, -500.0]));
    raise_scenario_go(&mut fired);
    set_impulse_ai_policy(
        &mut fired,
        flag_gated_helm_policy(
            crate::entities::config::HELM_IMPULSE_CHANNEL,
            crate::ai::policy::AiPolicyVerb::EngageImpulse,
        ),
    );
    tick(&mut fired);
    assert_eq!(
        get_impulse_command(&mut fired),
        crate::ship::impulse::ImpulsePhase::Charging,
        "with the world flag set the same impulse guard must command the charge"
    );
}

/// Issue #891 stage 2, both-directions proof for the Helm BOOST host.
#[test]
fn helm_boost_flag_guard_reads_the_world_in_both_directions() {
    let flag_policy = || {
        flag_gated_helm_policy(
            crate::entities::config::HELM_BOOST_CHANNEL,
            crate::ai::policy::AiPolicyVerb::EngageBoost,
        )
    };

    let mut held = boost_ai_app(Some(flag_policy()));
    held.init_resource::<crate::world::server::WorldContentRuntime>();
    tick_twice(&mut held);
    assert!(
        !boost_command(&mut held),
        "with the world flag clear the boost guard must read false and hold"
    );

    let mut fired = boost_ai_app(Some(flag_policy()));
    raise_scenario_go(&mut fired);
    tick_twice(&mut fired);
    assert!(
        boost_command(&mut fired),
        "with the world flag set the same boost guard must engage boost"
    );
}

// ── AC8 demo host: the minimal stateful Boost policy (issue #882) ────────

/// The AC8 demonstrator, authored as TOML and decoded through the real
/// schema path so the test exercises content authoring, not a hand-built
/// typed value. Two states on the existing `boost` channel: `cruise` holds
/// boost and leaves for `surge` when the seeded hazard-urgency fact crosses
/// the AUTHORED `surge_urgency` param; `surge` engages boost unconditionally
/// and returns to `cruise` once `state_time` reaches the AUTHORED
/// `surge_dwell_secs`. Every threshold is an authored param (AGENTS.md #11)
/// — there is not a gameplay number in the Rust.
///
/// It also READS the host-written private memory: `cruise` only surges
/// while `memory(engagements)` is under the authored `max_engagements` cap.
/// That closes the #882 loop — the host writes the slot on entering a
/// boost-engaging state, the authored guard reads it back on a later tick —
/// and it is why `memory(...)` is not just a second spelling of `param`.
fn stateful_boost_policy() -> crate::ai::policy::AiPolicy {
    // A re-engagement cap far above anything these tests drive, so the
    // demonstrator behaves exactly as it did before the memory read was
    // authored; `stateful_boost_policy_capped` exercises the cap itself.
    stateful_boost_policy_with("3.0", "99.0")
}

/// The demonstrator with its authored dwell and re-engagement cap supplied,
/// so a test can drive the memory read without a second copy of the TOML.
fn stateful_boost_policy_with(
    surge_dwell_secs: &str,
    max_engagements: &str,
) -> crate::ai::policy::AiPolicy {
    let src = format!(
        r#"
initial_state = "cruise"

[param]
surge_urgency = 0.0
surge_dwell_secs = {surge_dwell_secs}
max_engagements = {max_engagements}

[memory]
engagements = 0.0
peak_hazard_urgency = 0.0

[[state]]
id = "cruise"

[[state.transition]]
priority = 10
to = "surge"
when = "fact(hazard_urgency) > param(surge_urgency) and fact(boost_available) > 0 and memory(engagements) < param(max_engagements)"

[[state]]
id = "surge"

[[state.rule]]
priority = 0
channel = "boost"
when = "true"
verb = "engage_boost"

[[state.transition]]
priority = 0
to = "cruise"
when = "state_time >= param(surge_dwell_secs)"
"#
    );
    let cfg: crate::entities::config::FineSystemAiConfigToml =
        toml::from_str(&src).expect("the authored stateful boost policy parses");
    assert!(
        crate::entities::config::validate_fine_system_ai_policy(
            &cfg,
            &[crate::entities::config::HELM_BOOST_CHANNEL],
            &[crate::entities::config::HELM_ENGAGE_BOOST_VERB],
        )
        .is_ok(),
        "the demo policy must pass real content validation"
    );
    cfg.to_policy().expect("decodes to a typed machine")
}

fn boost_policy_state(app: &mut App) -> crate::ai::policy::AiPolicyRuntimeState {
    let ship = find_ship_entity(app);
    app.world()
        .entity(ship)
        .get::<HelmBoostAiPolicyState>()
        .expect("ship must carry HelmBoostAiPolicyState")
        .0
        .clone()
}

/// AC8 (+ AC1, AC2): the minimal stateful host end to end. The machine
/// starts in the authored initial state `cruise`, which holds boost. On the
/// tick its transition guard fires, `ai_policy_state_tick` commits `surge`
/// BEFORE `ai_helm_boost` resolves — so the entered state's continuous rule
/// engages boost through the same admitted `SetBoost` seam a human uses,
/// in that very tick.
#[test]
fn stateful_boost_policy_transitions_and_engages_in_the_same_tick() {
    let mut app = boost_ai_app(Some(stateful_boost_policy()));
    // Before any tick: nothing committed, nothing engaged.
    assert!(!boost_command(&mut app));

    tick_twice(&mut app);
    assert_eq!(
        boost_policy_state(&mut app).current,
        "surge",
        "the hazard guard must carry the machine out of `cruise`"
    );
    assert!(
        boost_command(&mut app),
        "the entered state's continuous rule must engage boost through the \
         admitted SetBoost seam in the same tick the transition committed"
    );
}

/// Issue #1152: the machine tick records its transition diagnostics
/// read-only. After the hazard guard carries `cruise → surge`, the runtime
/// state names that committed transition; while it then holds in `surge`
/// inside the authored dwell, it names the guard blocking the return to
/// `cruise`. This is the surface the per-host debug view projects, driven end
/// to end through the real host rather than a hand-built state.
#[test]
fn stateful_boost_machine_records_last_and_blocked_transitions() {
    let mut app = boost_ai_app(Some(stateful_boost_policy()));
    tick_twice(&mut app);

    let state = boost_policy_state(&mut app);
    assert_eq!(state.current, "surge");
    let last = state
        .last_transition
        .expect("the committed cruise -> surge transition must be recorded");
    assert_eq!(last.from, "cruise");
    assert_eq!(last.to, "surge");
    assert!(
        last.guard.contains("hazard_urgency"),
        "the recorded guard reads as authored, got {}",
        last.guard
    );

    // Held in `surge` inside the 3 s dwell: the only outgoing edge (back to
    // `cruise`, gated on `state_time`) is not yet satisfied, so it is the
    // blocking guard.
    tick_twice(&mut app);
    let state = boost_policy_state(&mut app);
    assert_eq!(state.current, "surge");
    let blocked = state
        .blocked_transition
        .expect("the unsatisfied return-to-cruise guard must be recorded");
    assert_eq!(blocked.to, "cruise");
    assert!(
        blocked.guard.contains("state_time"),
        "the blocking guard reads as authored, got {}",
        blocked.guard
    );
}

/// AC2 (one transition per tick) at the host: `surge` can only return to
/// `cruise` after the authored dwell, so a machine cannot walk two edges in
/// one tick — the state is `surge`, never back at `cruise`, immediately
/// after the first transition.
#[test]
fn stateful_boost_policy_fires_at_most_one_transition_per_tick() {
    let mut app = boost_ai_app(Some(stateful_boost_policy()));
    tick_twice(&mut app);
    assert_eq!(boost_policy_state(&mut app).current, "surge");
    // Several more ticks inside the authored dwell keep it there: the AI
    // tick cadence is 30 Hz and the dwell is 3 s, so ~90 ticks would be
    // needed. One tick can only ever advance one edge.
    tick_twice(&mut app);
    assert_eq!(
        boost_policy_state(&mut app).current,
        "surge",
        "no second edge may be walked while the authored dwell holds"
    );
}

/// AC4: state time is derived from the shared AI tick cadence, not from
/// `Time::delta`. The clock advances by exactly one authored tick period
/// per gated run, so `entered_at_secs` and the state clock are reproducible
/// regardless of frame rate.
#[test]
fn stateful_policy_state_time_advances_on_the_shared_ai_tick() {
    let mut app = boost_ai_app(Some(stateful_boost_policy()));
    tick_twice(&mut app);
    let before = app.world().resource::<AiPolicyTickClock>().0;
    tick_twice(&mut app);
    let after = app.world().resource::<AiPolicyTickClock>().0;
    assert!(
        after > before,
        "the tick-derived policy clock must advance on gated ticks"
    );
    let period = 1.0 / crate::entities::config::GlobalConfig::default().ai_tick_hz as f64;
    let advanced = after - before;
    assert!(
        (advanced % period).abs() < 1e-9 || ((advanced % period) - period).abs() < 1e-9,
        "the clock must advance in whole authored tick periods, got {advanced}"
    );
}

/// AC5: policy state resets when the system is unavailable, and again when
/// AI regains control — a recovered system never resumes a stale
/// mid-manoeuvre state.
#[test]
fn stateful_policy_state_resets_when_the_system_is_unavailable_and_on_recovery() {
    let mut app = boost_ai_app(Some(stateful_boost_policy()));
    tick_twice(&mut app);
    assert_eq!(boost_policy_state(&mut app).current, "surge");

    // Boost becomes unavailable (the capability is stripped): the machine
    // is put back to the authored initial state rather than left in
    // `surge`, and boost stands down.
    let ship = find_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(ship)
        .remove::<crate::ship::components::BoostConfigResource>();
    tick_twice(&mut app);
    assert_eq!(
        boost_policy_state(&mut app).current,
        "cruise",
        "an unavailable system must reset its policy state"
    );

    // The system recovers: it restarts from `cruise` (proved above) and
    // re-earns `surge` through its guard rather than resuming it.
    app.world_mut()
        .entity_mut(ship)
        .insert(crate::ship::components::BoostConfigResource {
            enabled: true,
            ..Default::default()
        });
    tick_twice(&mut app);
    assert_eq!(
        boost_policy_state(&mut app).current,
        "surge",
        "on recovery the machine re-enters `surge` via its guard, from initial"
    );
}

// ── Authored history operators, through the real host (issue #890) ───────

/// How many shared AI ticks the fixtures below author their window over.
///
/// Deliberately not a small number and not a multiple of the number of
/// per-axis hosts: if the window were folded once per axis rather than once
/// per shared tick it would fill in two ticks rather than eight, and the
/// equality below reads that difference straight off.
const WINDOW_TICKS: usize = 8;

/// How many shared AI ticks the policy clock has counted. That clock
/// advances by exactly one authored period per run of
/// [`ai_policy_state_tick`], which runs under the shared `ai_tick_ready`
/// latch — so this is the number of SHARED ticks, whatever the frame rate
/// or the number of `app.update()` calls it took to reach them.
fn shared_ai_ticks(app: &App) -> usize {
    let period = 1.0 / crate::entities::config::GlobalConfig::default().ai_tick_hz as f64;
    (app.world().resource::<AiPolicyTickClock>().0 / period).round() as usize
}

/// Decode a boost policy through the REAL schema + host validation path, so
/// these fixtures exercise content authoring rather than a hand-built value.
fn validated_boost_policy(src: &str) -> crate::ai::policy::AiPolicy {
    let cfg: crate::entities::config::FineSystemAiConfigToml =
        toml::from_str(src).expect("the authored windowed boost policy parses");
    crate::entities::config::validate_fine_system_ai_policy_for(
        &crate::entities::ai_flag_hosts::HELM_BOOST,
        &cfg,
        &[crate::entities::config::HELM_BOOST_CHANNEL],
        &[crate::entities::config::HELM_ENGAGE_BOOST_VERB],
    )
    .expect("a windowed guard on a folding host must pass real content validation");
    cfg.to_policy().expect("decodes to a typed machine")
}

/// A machine whose TRANSITION asks a windowed question of a fact the host
/// seeds every tick. The window length is authored in TOML, not in Rust.
fn windowed_transition_policy() -> crate::ai::policy::AiPolicy {
    validated_boost_policy(&format!(
        r#"
initial_state = "watching"

[param]
window_ticks = {WINDOW_TICKS}.0

[[state]]
id = "watching"

[[state.transition]]
priority = 0
to = "armed"
when = "history(min, boost_available, param(window_ticks)) >= 1"

[[state]]
id = "armed"

[[state.rule]]
priority = 0
channel = "boost"
when = "true"
verb = "engage_boost"
"#
    ))
}

/// The same window in the OTHER authorable position: a per-state continuous
/// RULE guard, which `ai_helm_boost` resolves later in the same tick.
fn windowed_rule_policy() -> crate::ai::policy::AiPolicy {
    validated_boost_policy(&format!(
        r#"
initial_state = "holding"

[param]
window_ticks = {WINDOW_TICKS}.0

[[state]]
id = "holding"

[[state.rule]]
priority = 0
channel = "boost"
when = "history(min, boost_available, param(window_ticks)) >= 1"
verb = "engage_boost"
"#
    ))
}

/// Run until `done`, returning how many SHARED AI ticks it took.
fn ticks_until(app: &mut App, mut done: impl FnMut(&mut App) -> bool) -> Option<usize> {
    for _ in 0..80 {
        tick(app);
        if done(app) {
            return Some(shared_ai_ticks(app));
        }
    }
    None
}

/// AC (the once-per-shared-tick fold): an authored window of N ticks fills
/// after exactly N SHARED AI ticks.
///
/// This is the test that fails if the fold moves. `ai_policy_state_tick`
/// calls the fold once per fine system per gated tick; the four per-axis
/// actuator systems resolve guards off the same ship in the same tick and
/// fold nothing. Move the fold into `resolve_helm_channel` (or add a second
/// call anywhere on the per-tick path) and this window fills several times
/// faster — the transition commits at tick 2 or 3 instead of 8 — which is
/// exactly the failure #789 had to design around by keeping its bespoke
/// window facts out of rule guards altogether.
#[test]
fn an_authored_window_fills_after_exactly_that_many_shared_ai_ticks() {
    let mut app = boost_ai_app(Some(windowed_transition_policy()));
    let armed_at = ticks_until(&mut app, |app| boost_policy_state(app).current == "armed")
        .expect("the authored window must fill within the drive");
    assert_eq!(
        armed_at, WINDOW_TICKS,
        "an authored {WINDOW_TICKS}-tick window must take {WINDOW_TICKS} SHARED AI \
         ticks to fill. Filling sooner means the window is being folded more than \
         once per shared tick — the per-axis hosts resolve guards off this same \
         ship in this same tick, and a fold in any of them makes every authored \
         span mean a fraction of what the file says."
    );
    assert!(
        boost_command(&mut app),
        "the entered state's rule must engage boost through the admitted seam"
    );
}

/// AC (evaluation scope): the SAME window is readable from a per-state RULE
/// guard, which a DIFFERENT system (`ai_helm_boost`) resolves later in the
/// same tick off the same per-fine-system bag.
///
/// This is the position the #788/#789 bespoke facts could not be authored in
/// at all: those were seeded only in `ai_policy_state_tick`, so a rule guard
/// naming one parsed, validated, and read absent for ever. Here the two
/// positions agree by construction, and the timing proves it — the rule
/// fires on the same tick the window completes, not later and not sooner.
#[test]
fn a_windowed_rule_guard_fires_in_the_per_axis_host_on_the_same_tick() {
    let mut app = boost_ai_app(Some(windowed_rule_policy()));
    let engaged_at = ticks_until(&mut app, boost_command).expect("the guard must fire");
    assert_eq!(
        engaged_at, WINDOW_TICKS,
        "a windowed RULE guard is resolved by ai_helm_boost from the window \
         ai_policy_state_tick folded earlier in the same tick, so it must fire on \
         the tick the authored window completes"
    );
}

/// AC5 covers the window too, through the real host: a system that loses AI
/// control re-earns its window rather than resuming a stale one.
#[test]
fn losing_the_system_restarts_the_authored_window() {
    let mut app = boost_ai_app(Some(windowed_transition_policy()));
    // Nearly there, then the capability is stripped.
    for _ in 0..6 {
        tick(&mut app);
    }
    assert_ne!(boost_policy_state(&mut app).current, "armed");
    let ship = find_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(ship)
        .remove::<crate::ship::components::BoostConfigResource>();
    tick_twice(&mut app);
    assert!(
        boost_policy_state(&mut app).current == "watching"
            && boost_policy_state(&mut app).memory.history().is_empty(),
        "an unavailable system must drop the evidence it gathered while it was \
         running, not bank it"
    );

    app.world_mut()
        .entity_mut(ship)
        .insert(crate::ship::components::BoostConfigResource {
            enabled: true,
            ..Default::default()
        });
    let before = shared_ai_ticks(&app);
    let armed_at = ticks_until(&mut app, |app| boost_policy_state(app).current == "armed")
        .expect("the window must re-fill after recovery");
    assert_eq!(
        armed_at - before,
        WINDOW_TICKS,
        "the window must be re-earned in full, not resumed"
    );
}

/// AC5 (the other half): a system NOT operated by AI holds at the authored
/// initial state, so the tick AI gains control begins from `initial`.
#[test]
fn stateful_policy_state_holds_at_initial_while_ai_does_not_operate_the_system() {
    let mut app = boost_ai_app(Some(stateful_boost_policy()));
    set_helm_control_source(&mut app, ControlSource::Human);
    tick_twice(&mut app);
    assert_eq!(
        boost_policy_state(&mut app).current,
        "cruise",
        "a human-operated system's policy state stays at initial"
    );
    assert!(!boost_command(&mut app));
}

/// AC7 at the HOST: the stateless boost path is untouched. The same host,
/// given a #775-shaped policy, still resolves through `resolve_channel` and
/// never touches the state component — which stays at its default.
#[test]
fn stateless_boost_policy_never_enters_the_state_machine_path() {
    let mut app = boost_ai_app(Some(hazard_boost_policy()));
    tick_twice(&mut app);
    assert!(
        boost_command(&mut app),
        "the stateless hazard policy engages exactly as it did before #882"
    );
    assert_eq!(
        boost_policy_state(&mut app).current,
        "",
        "a stateless policy must leave the state component untouched"
    );
}

// ── Host-written private memory (issue #882) ─────────────────────────────

fn boost_memory(app: &mut App, slot: &str) -> Option<f64> {
    boost_policy_state(app).memory.get(slot)
}

/// Empty the world snapshot, so the shared plan carries no hazard and the
/// live `hazard_urgency` reading falls back to zero.
fn clear_snapshot(app: &mut App) {
    app.insert_resource(crate::ai::server::WorldSnapshot {
        entities: Vec::new(),
    });
}

/// Tick until the machine reaches `want`, so a test does not depend on
/// exactly which gated tick the shared plan first carries hazard.
fn tick_until_state(app: &mut App, want: &str) {
    for _ in 0..8 {
        if boost_policy_state(app).current == want {
            return;
        }
        tick(app);
    }
    panic!(
        "machine never reached `{want}` (stuck in `{}`)",
        boost_policy_state(app).current
    );
}

/// Finding 1's guard, at the host: the PRODUCTION player ship is a
/// `LocalShip`, and a `LocalShip` carrying a stateful boost policy must
/// actually transition. It only can if it carries `HelmBoostAiPolicyState`
/// — which it now takes from `ai_high_fidelity_components`, the one
/// definition both `lod_ai_ships` and `server_app::spawn_game_start_entities`
/// insert. Before that, the player ship silently had no state component,
/// `ai_policy_state_tick`'s non-optional query skipped it, and the host fell
/// through to the stateless arm with an empty top-level rule list: boost
/// never engaged, with no warning.
#[test]
fn local_ship_with_a_stateful_boost_policy_transitions_and_engages() {
    let mut app = boost_ai_app(Some(stateful_boost_policy()));
    let ship = find_ship_entity(&mut app);
    assert!(
        app.world()
            .entity(ship)
            .get::<crate::server_app::LocalShip>()
            .is_some(),
        "this is the player-ship spawn shape, not an NPC's"
    );
    assert!(
        app.world()
            .entity(ship)
            .get::<HelmBoostAiPolicyState>()
            .is_some(),
        "the LocalShip must carry the per-fine-system policy state component"
    );
    tick_twice(&mut app);
    assert_eq!(boost_policy_state(&mut app).current, "surge");
    assert!(boost_command(&mut app));
}

/// The writer exists and its value PERSISTS across ticks.
///
/// `peak_hazard_urgency` is a running maximum the host folds every tick. It
/// is not authored (so it cannot be a `param`) and it is not a reading of
/// this tick (so it cannot be a `fact`): retention is the whole content of
/// the slot. Later ticks whose hazard is lower must not lower it.
#[test]
fn host_written_memory_persists_across_ticks() {
    let mut app = boost_ai_app(Some(stateful_boost_policy()));
    tick_until_state(&mut app, "surge");
    let peak = boost_memory(&mut app, PEAK_HAZARD_MEMORY)
        .expect("the host must have written the peak-hazard slot");
    assert!(
        peak > 0.0,
        "the seeded moving hazard must have been recorded, got {peak}"
    );

    // Clear the hazard: this tick's reading is 0, but the retained maximum
    // is not a reading of this tick.
    clear_snapshot(&mut app);
    tick_twice(&mut app);
    tick_twice(&mut app);
    assert_eq!(
        boost_memory(&mut app, PEAK_HAZARD_MEMORY),
        Some(peak),
        "a retained maximum must survive ticks whose live reading is lower"
    );
}

/// The written value SURVIVES a transition: `engagements` is incremented on
/// entering the boost-engaging `surge` state and is still there after the
/// machine walks the edge back to `cruise`. State time restarts on entry;
/// private memory deliberately does not.
#[test]
fn host_written_memory_survives_a_transition() {
    // Zero dwell so `surge` returns to `cruise` as soon as it is re-eligible,
    // and a cap high enough that the memory read never blocks the re-entry.
    let mut app = boost_ai_app(Some(stateful_boost_policy_with("0.0", "99.0")));
    tick_until_state(&mut app, "surge");
    assert_eq!(
        boost_memory(&mut app, ENGAGEMENTS_MEMORY),
        Some(1.0),
        "entering a boost-engaging state must increment the host-written slot"
    );

    // Walk the edge back to `cruise`. State time restarts; memory does not.
    tick_until_state(&mut app, "cruise");
    let state = boost_policy_state(&mut app);
    assert_eq!(
        state.memory.get(ENGAGEMENTS_MEMORY),
        Some(1.0),
        "private memory must survive the transition that follows it"
    );
}

/// The POLICY reads what the host wrote. With the authored re-engagement cap
/// at one, the machine surges once, returns on the zero dwell, and can never
/// surge again — because `cruise`'s guard reads `memory(engagements)`. If
/// the slot were frozen at its declared 0.0 (i.e. behaviourally a `param`),
/// the machine would surge again immediately.
#[test]
fn authored_guard_reads_the_host_written_memory() {
    let mut app = boost_ai_app(Some(stateful_boost_policy_with("0.0", "1.0")));
    tick_until_state(&mut app, "surge");
    assert_eq!(boost_memory(&mut app, ENGAGEMENTS_MEMORY), Some(1.0));

    // Back to cruise on the zero dwell...
    tick_until_state(&mut app, "cruise");
    // ...and the cap now holds it there, with the hazard still live.
    for _ in 0..10 {
        tick(&mut app);
        assert_eq!(
            boost_policy_state(&mut app).current,
            "cruise",
            "the authored cap must be read from host-written memory"
        );
    }
}

/// The reset CLEARS it. An unavailable system is reset to the authored
/// initial state AND the authored memory, so a recovered system never
/// resumes a stale count (AC5).
#[test]
fn host_written_memory_is_cleared_by_the_reset() {
    let mut app = boost_ai_app(Some(stateful_boost_policy()));
    tick_until_state(&mut app, "surge");
    assert_eq!(boost_memory(&mut app, ENGAGEMENTS_MEMORY), Some(1.0));
    assert!(boost_memory(&mut app, PEAK_HAZARD_MEMORY).unwrap_or(0.0) > 0.0);

    // Strip the capability → AC5 reset.
    let ship = find_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(ship)
        .remove::<crate::ship::components::BoostConfigResource>();
    tick_twice(&mut app);
    assert_eq!(boost_policy_state(&mut app).current, "cruise");
    assert_eq!(
        boost_memory(&mut app, ENGAGEMENTS_MEMORY),
        Some(0.0),
        "reset must restore the AUTHORED memory, not keep the drifted count"
    );
    assert_eq!(
        boost_memory(&mut app, PEAK_HAZARD_MEMORY),
        Some(0.0),
        "every host-written slot goes back to its authored declaration"
    );
}

/// `ai_helm_boost` runs on the shared AI-helm sim tick like its four siblings
/// (issue #780 + #803), not once per rendered frame.
#[test]
fn ai_helm_boost_runs_on_the_shared_sim_tick_not_per_frame() {
    // A boost policy keyed off a sentinel-independent fact so it toggles
    // deterministically: engage while the seeded hazard is present. The probe
    // measures BoostCommand transitions, which only the operator can drive.
    let mut app = boost_ai_app(Some(hazard_boost_policy()));
    let counts = count_sim_tick_runs(
        &mut app,
        // Arm: force the applied BoostCommand back off so a run is observable
        // as a re-engage. (The operator re-emits only on change.)
        |app| {
            let ship = find_ship_entity(app);
            let mut entity = app.world_mut().entity_mut(ship);
            entity
                .get_mut::<crate::ship::helm::BoostCommand>()
                .unwrap()
                .0 = false;
            *entity.get_mut::<ShipBoost>().unwrap() = ShipBoost::default();
        },
        boost_command,
    );
    assert_shared_sim_tick_cadence("ai_helm_boost", counts);
}

// ── Avoidance bends travel; only imminent collision overrides facing ─────

fn plan_desired_facing(app: &mut App, ship: Entity) -> Vec3 {
    app.world()
        .resource::<crate::ship::helm_planner::HelmMotionPlan>()
        .ships
        .get(&ship)
        .map(|sp| sp.motion.desired_facing_local)
        .unwrap_or_default()
}

/// An app steering toward a forward Reach with a moving hazard on the
/// starboard bow. `imminent_collision_facing_threshold` is authored so the
/// same hazard is either ordinary avoidance (default 1.0 — off) or an
/// imminent-collision facing override (low threshold).
fn avoidance_facing_app(imminent_threshold: f32) -> App {
    let mut app = test_app();
    set_helm_control_source(&mut app, ControlSource::Ai);
    app.insert_resource(world_config_with_anchor("far-ahead", [0.0, 0.0, -900.0]));
    set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective("far-ahead", 8.0)]);
    set_behaviour_section(
        &mut app,
        crate::entities::config::BehaviourConfig {
            avoidance_buffer: 60.0,
            imminent_collision_facing_threshold: imminent_threshold,
            ..Default::default()
        },
    );
    // Stationary: below `AVOIDANCE_MIN_SPEED`, so the ordinary steering
    // doctrine (which includes avoidance_steering) does NOT turn facing —
    // isolating the imminent-collision facing override as the only thing that
    // can move desired facing off the forward objective heading.
    let mut physics = get_ship_physics(&mut app);
    physics.forward_speed = 0.0;
    physics.yaw = 0.0;
    set_ship_physics(&mut app, physics);
    // Close hazard on the starboard bow → high urgency, port-ward repulsion.
    snapshot_with_moving_obstacle(&mut app, [3.0, 0.0, -10.0], 1.0, 0.0, 0.0);
    app
}

/// AC4: only an imminent collision may temporarily override desired facing.
/// With the default (off) threshold the same in-range hazard leaves facing on
/// the forward objective heading (≈ -Z, x ≈ 0); with a low authored threshold
/// the imminent hazard overrides it toward the escape heading (a nonzero
/// local-X facing away from the starboard threat). The ship is stationary so
/// the ordinary avoidance-steering doctrine cannot itself turn facing —
/// proving the override, not doctrine, is what moves it.
#[test]
fn facing_overridden_only_when_collision_imminent() {
    let mut ordinary = avoidance_facing_app(1.0);
    let ship_o = find_ship_entity(&mut ordinary);
    tick_twice(&mut ordinary);
    let ordinary_facing = plan_desired_facing(&mut ordinary, ship_o);
    assert!(
        ordinary_facing.x.abs() < 0.1 && ordinary_facing.z < 0.0,
        "ordinary avoidance must leave facing on the forward objective heading \
         (the doctrine never touches facing on hazards below the imminent \
         threshold), got {ordinary_facing:?}"
    );

    let mut imminent = avoidance_facing_app(0.01);
    let ship_i = find_ship_entity(&mut imminent);
    tick_twice(&mut imminent);
    let imminent_facing = plan_desired_facing(&mut imminent, ship_i);
    assert!(
        imminent_facing.x.abs() > 0.2,
        "an imminent collision must temporarily override facing toward the \
         escape heading (nonzero local-X), got {imminent_facing:?}"
    );
}

/// AC3: ordinary avoidance BENDS travel without changing the active doctrine.
/// A forward Reach's throttle doctrine is identical with and without a hazard
/// in range — the avoidance response shows up in the lateral dodge, not in a
/// swapped travel decision.
#[test]
fn avoidance_bends_travel_without_changing_doctrine() {
    fn forward_throttle_and_dodge(with_hazard: bool) -> (f32, f32) {
        let mut app = lateral_thrust_ai_app(Some(crate::entities::config::BehaviourConfig {
            avoidance_buffer: 60.0,
            ..Default::default()
        }));
        // A forward objective so the engine doctrine commands forward travel.
        set_helm_control_source(&mut app, ControlSource::Ai);
        app.insert_resource(world_config_with_anchor("ahead", [0.0, 0.0, -900.0]));
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective("ahead", 8.0)]);
        let mut physics = get_ship_physics(&mut app);
        physics.forward_speed = 10.0;
        physics.yaw = 0.0;
        set_ship_physics(&mut app, physics);
        if with_hazard {
            snapshot_with_obstacle(&mut app, [4.0, 0.0, -40.0], 1.0);
        }
        tick_twice(&mut app);
        let ship = find_ship_entity(&mut app);
        let thrust = app
            .world()
            .resource::<crate::ship::helm_planner::HelmMotionPlan>()
            .ships
            .get(&ship)
            .map(|sp| {
                crate::ai::decode_thrust_from_velocity(sp.motion.desired_velocity_local.to_array())
            })
            .unwrap_or(0.0);
        (thrust, lateral_intent(&mut app))
    }

    let (clear_thrust, clear_dodge) = forward_throttle_and_dodge(false);
    let (hazard_thrust, hazard_dodge) = forward_throttle_and_dodge(true);

    assert!(
        (clear_thrust - hazard_thrust).abs() < 1e-4,
        "the travel doctrine (forward throttle) must be UNCHANGED by avoidance; \
         clear={clear_thrust} hazard={hazard_thrust}"
    );
    assert_eq!(
        clear_dodge, 0.0,
        "no hazard means no dodge — precondition for the bend below"
    );
    assert!(
        hazard_dodge.abs() > 0.0,
        "avoidance must BEND travel via the lateral dodge, got {hazard_dodge}"
    );
}

/// AC6 capability filtering for impulse: with no `ImpulseConfigResource` the
/// impulse operator stands down and never charges, even with an engaging
/// objective geometry that otherwise would.
#[test]
fn ai_helm_impulse_stands_down_without_impulse_config() {
    let anchor = "station-alpha";
    let mut app = impulse_ai_app(reach_scored_objective(anchor, 10.0));
    app.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, -500.0]));
    // Strip the impulse capability the fixture installed.
    let ship = find_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(ship)
        .remove::<ImpulseConfigResource>();
    tick(&mut app);
    assert_eq!(
        get_impulse_command(&mut app),
        crate::ship::impulse::ImpulsePhase::Idle,
        "no ImpulseConfigResource means no impulse capability — no charge"
    );
}

/// Drives the `helm_ai_decision` → `operate_helm` → `avoidance_steering`
/// path: a Reach anchor dead ahead down -Z, so the base steer sits in the
/// deadband at zero and any nonzero `SteeringInput` is avoidance and
/// nothing else. `avoidance_steering` ignores ships slower than
/// `AVOIDANCE_MIN_SPEED`, hence the explicit forward speed.
fn helm_ai_steering_app(behaviour: Option<crate::entities::config::BehaviourConfig>) -> App {
    let mut app = test_app();
    set_helm_control_source(&mut app, ControlSource::Ai);
    app.insert_resource(world_config_with_anchor("far-ahead", [0.0, 0.0, -900.0]));
    set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective("far-ahead", 8.0)]);
    if let Some(behaviour) = behaviour {
        set_behaviour_section(&mut app, behaviour);
    }
    let mut physics = get_ship_physics(&mut app);
    physics.forward_speed = 10.0;
    physics.yaw = 0.0;
    set_ship_physics(&mut app, physics);
    app
}

fn steering_intent(app: &mut App) -> f32 {
    app.world_mut()
        .query::<&SteeringInput>()
        .single(app.world())
        .expect("ship must carry SteeringInput")
        .0
}

/// `helm_ai_decision` feeds `avoidance_buffer` to `operate_helm`, where it
/// widens the radius `avoidance_steering` treats as a threat.
#[test]
fn helm_ai_decision_honours_toml_authored_avoidance_buffer() {
    // Projected 30 units ahead (10 u/s × the default 3 s), the obstacle is
    // ~10.8 units away: outside the default 6-unit dodge radius
    // (0 + 1 + 5), inside an authored 61 (0 + 1 + 60).
    let obstacle = [4.0, 0.0, -40.0];

    let mut default_app = helm_ai_steering_app(None);
    snapshot_with_obstacle(&mut default_app, obstacle, 1.0);
    tick(&mut default_app);
    assert_eq!(
        steering_intent(&mut default_app),
        0.0,
        "with the default 5-unit buffer the obstacle is no threat and the anchor \
         is dead ahead, so steering stays in the deadband"
    );

    let mut authored_app = helm_ai_steering_app(Some(crate::entities::config::BehaviourConfig {
        avoidance_buffer: 60.0,
        ..Default::default()
    }));
    snapshot_with_obstacle(&mut authored_app, obstacle, 1.0);
    tick(&mut authored_app);
    assert!(
        steering_intent(&mut authored_app).abs() > 0.0,
        "a TOML-authored 60-unit avoidance_buffer must make the helm steer around \
         the obstacle; got no steering, so helm_ai_decision is still passing \
         crate::ai::AVOIDANCE_BUFFER"
    );
}

/// `helm_ai_decision` feeds `avoidance_look_ahead_secs` to `operate_helm`,
/// where it sets how far forward `avoidance_steering` projects the ship
/// before testing for a threat.
#[test]
fn helm_ai_decision_honours_toml_authored_avoidance_look_ahead() {
    // At 10 u/s the default 3 s horizon projects 30 units ahead, leaving
    // the obstacle ~70 units off; a 10 s horizon projects 100 units, right
    // onto it.
    let obstacle = [2.0, 0.0, -100.0];

    let mut default_app = helm_ai_steering_app(None);
    snapshot_with_obstacle(&mut default_app, obstacle, 1.0);
    tick(&mut default_app);
    assert_eq!(
        steering_intent(&mut default_app),
        0.0,
        "the default 3 s horizon does not reach the obstacle at 100 units"
    );

    let mut authored_app = helm_ai_steering_app(Some(crate::entities::config::BehaviourConfig {
        avoidance_look_ahead_secs: 10.0,
        ..Default::default()
    }));
    snapshot_with_obstacle(&mut authored_app, obstacle, 1.0);
    tick(&mut authored_app);
    assert!(
        steering_intent(&mut authored_app).abs() > 0.0,
        "a TOML-authored 10 s look-ahead must bring the obstacle into the helm's \
         projected path; got no steering, so helm_ai_decision is still passing \
         crate::ai::AVOIDANCE_LOOK_AHEAD_SECS"
    );
}

/// Drives the full-AI helm's dodge — every helm axis on AI, the shape an
/// unmanned Helm station or an NPC hull comes up in.
///
/// Until #704 the subject here was `operate_helm_ai`, which derived the
/// dodge itself; it now comes from `ai_helm_lateral_thrust` like every other
/// lateral write, reading the shared hazard surface (issue #743). The
/// fixture still earns its place next to `lateral_thrust_ai_app`: that one
/// pins the same tunables under the *Simplified* rating (coarse helm human,
/// lateral automated — what the cruiser and destroyer ship), this one under
/// a fully-AI helm. Same system, the two gate shapes real content deploys.
///
/// Forward speed is not optional scaffolding — the shared `assess_hazards`
/// projects the ship by `forward_speed * avoidance_look_ahead_secs`, so a
/// stationary ship collapses that projection onto its own position and makes
/// the look-ahead term unobservable no matter what value is passed.
fn helm_ai_app(behaviour: Option<crate::entities::config::BehaviourConfig>) -> App {
    let mut app = test_app();
    set_helm_control_source(&mut app, ControlSource::Ai);
    let mut cfg = crate::world::config::WorldConfig::default();
    // Waypoint far down -Z keeps the helm driving straight ahead, so
    // the lateral axis reflects avoidance alone.
    cfg.anchors.insert("wp0".into(), [0.0, 0.0, -900.0]);
    app.insert_resource(cfg);
    set_ship_blackboard_objectives(&mut app, vec![patrol_scored_objective(vec!["wp0"], 20.0)]);
    if let Some(behaviour) = behaviour {
        set_behaviour_section(&mut app, behaviour);
    }
    let mut physics = get_ship_physics(&mut app);
    physics.forward_speed = 10.0;
    physics.yaw = 0.0;
    set_ship_physics(&mut app, physics);
    app
}

/// The same two tunables reach the pure AI a second way: the full-AI helm's
/// dodge. The dodge and the steering must agree about clearance, so this
/// site must read the same TOML the steering does.
///
/// Ported in #704, then rewired in #743: the dodge is now the shared
/// hazard surface read by `ai_helm_lateral_thrust`. Faithful because the
/// property under test is unchanged — a TOML-authored `avoidance_buffer`
/// must reach the shared `assess_hazards` on a fully-AI helm rather than the
/// `crate::ai::AVOIDANCE_BUFFER` constant — and it is asserted on the same
/// hull, obstacle and geometry as before. What the delete changed is only
/// *which* system performs the write, and hence the tick count: the
/// monolith's call was unthrottled, whereas `ai_helm_lateral_thrust` is
/// gated by the deliberate shared AI-helm sim tick (~30 Hz by default,
/// issue #803). Hence `tick_twice`, matching `lateral_thrust_ai_honours_*`.
#[test]
fn full_ai_helm_honours_toml_authored_avoidance_buffer() {
    // Projected 30 units ahead (10 u/s × the default 3 s), the obstacle is
    // ~10.8 units away: outside the default 6-unit dodge radius
    // (0 + 1 + 5), inside an authored 61 (0 + 1 + 60).
    let obstacle = [4.0, 0.0, -40.0];

    let mut default_app = helm_ai_app(None);
    snapshot_with_obstacle(&mut default_app, obstacle, 1.0);
    tick_twice(&mut default_app);
    assert_eq!(
        lateral_intent(&mut default_app),
        0.0,
        "with the default 5-unit buffer the obstacle sits ~10.8 units off the \
         projected path and is not a threat"
    );

    let mut authored_app = helm_ai_app(Some(crate::entities::config::BehaviourConfig {
        avoidance_buffer: 60.0,
        ..Default::default()
    }));
    snapshot_with_obstacle(&mut authored_app, obstacle, 1.0);
    tick_twice(&mut authored_app);
    assert!(
        lateral_intent(&mut authored_app).abs() > 0.0,
        "the full-AI helm must pass the TOML-authored avoidance_buffer to \
         the shared assess_hazards, not crate::ai::AVOIDANCE_BUFFER"
    );
}

/// The full-AI helm must pass the TOML-authored `avoidance_look_ahead_secs`
/// to the shared `assess_hazards`, which uses it to project the ship forward
/// before testing for a threat. Mirrors
/// `lateral_thrust_ai_honours_toml_authored_avoidance_look_ahead`, but with
/// every helm axis on AI rather than the Simplified rating's lateral-only.
///
/// Ported in #704, rewired in #743 to the shared hazard surface — same
/// property, same geometry, hence `tick_twice` for the shared AI-helm sim
/// tick. See that test's note.
#[test]
fn full_ai_helm_honours_toml_authored_avoidance_look_ahead() {
    // Forward at yaw 0 is -Z. At 10 u/s the default 3 s horizon projects
    // only 30 units ahead, leaving the obstacle ~70 units off; an authored
    // 10 s projects 100 units, landing 2 units from it — inside the default
    // 6-unit dodge radius (0 + 1 + 5), so the buffer is held constant and
    // the look-ahead is the only variable.
    let obstacle = [2.0, 0.0, -100.0];

    let mut default_app = helm_ai_app(None);
    snapshot_with_obstacle(&mut default_app, obstacle, 1.0);
    tick_twice(&mut default_app);
    assert_eq!(
        lateral_intent(&mut default_app),
        0.0,
        "the default 3 s horizon projects only 30 units ahead — the obstacle at \
         100 is not yet a threat"
    );

    let mut authored_app = helm_ai_app(Some(crate::entities::config::BehaviourConfig {
        avoidance_look_ahead_secs: 10.0,
        ..Default::default()
    }));
    snapshot_with_obstacle(&mut authored_app, obstacle, 1.0);
    tick_twice(&mut authored_app);
    assert!(
        lateral_intent(&mut authored_app).abs() > 0.0,
        "the full-AI helm must pass the TOML-authored avoidance_look_ahead_secs to the \
         shared assess_hazards, not crate::ai::AVOIDANCE_LOOK_AHEAD_SECS"
    );
}

/// `nav_handoff_speed` is the throttle the helm adopts for a Channel-3
/// Navigation→Helm handoff. It is authored in `[behaviour]`, and the
/// `crate::ai::NAV_HANDOFF_SPEED` fallback exists only for an entity with
/// no `[behaviour]` section at all.
#[test]
fn helm_ai_honours_toml_authored_nav_handoff_speed() {
    fn nav_goal_app(behaviour: Option<crate::entities::config::BehaviourConfig>) -> App {
        let mut app = test_app();
        set_helm_control_source(&mut app, ControlSource::Ai);
        // A Helm-relevant objective must exist (an empty pool makes
        // `operate_helm_ai` zero the intent and skip the decision
        // entirely), but it must not *resolve* — a Reach whose anchor is
        // absent from the WorldConfig yields `None`, so `operate_helm`
        // falls through to the Navigation waypoint handoff, the only path
        // that reads `nav_handoff_speed`.
        set_ship_blackboard_objectives(
            &mut app,
            vec![reach_scored_objective("anchor-not-in-world-config", 8.0)],
        );
        if let Some(behaviour) = behaviour {
            set_behaviour_section(&mut app, behaviour);
        }
        // Post-#702 the handoff is the ship's own `NavigationWaypoint`,
        // gated by a matching `HelmWaypointClearance`, rather than a
        // private `AiMemory.nav_goal` copy. Dead ahead and far away, so the
        // helm throttles up at exactly `nav_handoff_speed`.
        set_cleared_nav_waypoint(&mut app, 0.0, -900.0);
        tick(&mut app);
        app
    }

    fn thrust(app: &mut App) -> f32 {
        app.world_mut()
            .query::<&ThrustInput>()
            .single(app.world())
            .expect("ship must carry ThrustInput")
            .0
    }

    assert!(
        (thrust(&mut nav_goal_app(None)) - crate::ai::NAV_HANDOFF_SPEED).abs() < 1e-6,
        "a ship with no [behaviour] section must fall back to NAV_HANDOFF_SPEED"
    );
    assert!(
        (thrust(&mut nav_goal_app(Some(
            crate::entities::config::BehaviourConfig {
                nav_handoff_speed: 0.25,
                ..Default::default()
            }
        ))) - 0.25)
            .abs()
            < 1e-6,
        "a TOML-authored nav_handoff_speed must be the throttle the helm adopts, \
         not crate::ai::NAV_HANDOFF_SPEED"
    );
}

/// Regression (issue #696 review, finding 2): Reach completion is the
/// other site that judged arrival against the hardcoded constant.
#[test]
fn detect_reach_completion_honours_toml_authored_arrival_radius() {
    use crate::core::messages::{AiDirective, ObjectiveSource};
    use crate::objectives::{ObjectiveManager, UtilityConfig};
    use crate::world::server::ObjectiveManagerRes;

    fn reach_app(arrival_radius: Option<f32>) -> App {
        let mut app = test_app();
        let anchor = "dock-mid";
        // 100 units out: inside a 150 radius, outside the default 20.
        set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 8.0)]);
        app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
        set_helm_control_source(&mut app, ControlSource::Ai);
        if let Some(radius) = arrival_radius {
            let ship = find_ship_entity(&mut app);
            app.world_mut()
                .entity_mut(ship)
                .insert(crate::entities::spawner::BehaviourSection(
                    crate::entities::config::BehaviourConfig {
                        waypoint_arrival_radius: radius,
                        ..Default::default()
                    },
                ));
        }
        let mut mgr = ObjectiveManager::new();
        mgr.add_full(
            "reach-dock-mid",
            "Dock at Mid",
            true,
            vec![],
            AiDirective::Reach {
                anchor: anchor.into(),
            },
            UtilityConfig::default(),
            ObjectiveSource::Mission,
        );
        app.insert_resource(ObjectiveManagerRes(mgr));
        tick(&mut app);
        app
    }

    fn status(app: &App) -> Option<crate::core::messages::ObjectiveStatus> {
        app.world()
            .resource::<ObjectiveManagerRes>()
            .0
            .sorted_snapshots()
            .into_iter()
            .find(|o| o.id == "reach-dock-mid")
            .map(|o| o.status)
    }

    assert_eq!(
        status(&reach_app(None)),
        Some(crate::core::messages::ObjectiveStatus::Active),
        "the default arrival radius must not count 100 units away as reached"
    );
    assert_eq!(
        status(&reach_app(Some(150.0))),
        Some(crate::core::messages::ObjectiveStatus::Completed),
        "a TOML-widened arrival radius must complete the Reach objective"
    );
}

#[test]
fn detect_reach_completion_does_not_complete_when_far() {
    use crate::core::messages::{AiDirective, ObjectiveSource};
    use crate::objectives::{ObjectiveManager, UtilityConfig};
    use crate::world::server::ObjectiveManagerRes;

    let mut app = test_app();
    let anchor = "dock-far";
    // Anchor 500 units away — ship starts at origin.
    set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 8.0)]);
    app.insert_resource(world_config_with_anchor(anchor, [500.0, 0.0, 0.0]));
    set_helm_control_source(&mut app, ControlSource::Ai);

    let mut mgr = ObjectiveManager::new();
    mgr.add_full(
        "reach-dock-far",
        "Dock at Far",
        true,
        vec![],
        AiDirective::Reach {
            anchor: anchor.into(),
        },
        UtilityConfig::default(),
        ObjectiveSource::Mission,
    );
    app.insert_resource(ObjectiveManagerRes(mgr));

    tick(&mut app);

    let res = app.world().resource::<ObjectiveManagerRes>();
    let obj = res
        .0
        .sorted_snapshots()
        .into_iter()
        .find(|o| o.id == "reach-dock-far");
    assert!(
        obj.map(|o| o.status == crate::core::messages::ObjectiveStatus::Active)
            .unwrap_or(false),
        "Reach objective must remain Active when ship is far from the anchor"
    );
}

#[test]
fn detect_reach_completion_does_not_complete_when_helm_human() {
    use crate::core::messages::{AiDirective, ObjectiveSource};
    use crate::objectives::{ObjectiveManager, UtilityConfig};
    use crate::world::server::ObjectiveManagerRes;

    let mut app = test_app();
    let anchor = "dock-beta";
    set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 8.0)]);
    app.insert_resource(world_config_with_anchor(anchor, [0.0, 0.0, 0.0]));
    // helm stays Human — completion system must not fire

    let mut mgr = ObjectiveManager::new();
    mgr.add_full(
        "reach-dock-beta",
        "Dock at Beta",
        true,
        vec![],
        AiDirective::Reach {
            anchor: anchor.into(),
        },
        UtilityConfig::default(),
        ObjectiveSource::Mission,
    );
    app.insert_resource(ObjectiveManagerRes(mgr));

    tick(&mut app);

    let res = app.world().resource::<ObjectiveManagerRes>();
    let obj = res
        .0
        .sorted_snapshots()
        .into_iter()
        .find(|o| o.id == "reach-dock-beta");
    assert!(
        obj.map(|o| o.status == crate::core::messages::ObjectiveStatus::Active)
            .unwrap_or(false),
        "Reach completion must not fire when helm is human-controlled"
    );
}

// ── E5 smoke tests (#553) ─────────────────────────────────────────────────

// (a) Enemy NPC hull — verifies that an NPC ship with both stick axes on
// Ai control satisfies `helm_axes_operate_ai`, the gate every per-ship
// "is the AI flying this" consumer reads since #801.
#[test]
fn npc_hull_ai_helm_policy_routes_through_npc_path() {
    use crate::ship::control_source::{ControlSource, ControlSourceResolver};

    let mut resolver = ControlSourceResolver::new();
    for system_id in [
        crate::ship::system_registry::helm_thrust_system_id(),
        crate::ship::system_registry::helm_steering_system_id(),
    ] {
        resolver.set(system_id, ControlSource::Ai);
    }
    let sources = ShipSystemControlSources(resolver);
    assert!(
        helm_axes_operate_ai(&sources),
        "NPC hull helm axes must route through the AI helm path"
    );
    assert!(
        !sources
            .0
            .policy_for(&crate::ship::system_registry::helm_thrust_system_id())
            .accept_human_input,
        "an NPC hull must not accept human helm input"
    );
}

// (b) All-Backfill player ship — verifies that when the player ship has
// both stick axes on Ai control but no BehaviourSection (no
// behaviour tree), `helm_axes_operate_ai` still returns true. A single
// AI axis is NOT enough — the predicate answers "is the AI flying this
// ship", which needs both.
#[test]
fn all_backfill_helm_policy_gates_operate_ai() {
    use crate::ship::control_source::{ControlSource, ControlSourceResolver};

    let mut resolver = ControlSourceResolver::new();
    resolver.set(
        crate::ship::system_registry::helm_thrust_system_id(),
        ControlSource::Ai,
    );
    let sources = ShipSystemControlSources(resolver);
    assert!(
        !helm_axes_operate_ai(&sources),
        "one AI axis alone must not satisfy the whole-helm AI predicate"
    );

    let mut resolver = ControlSourceResolver::new();
    resolver.set(
        crate::ship::system_registry::helm_thrust_system_id(),
        ControlSource::Ai,
    );
    resolver.set(
        crate::ship::system_registry::helm_steering_system_id(),
        ControlSource::Ai,
    );
    let sources = ShipSystemControlSources(resolver);
    assert!(
        helm_axes_operate_ai(&sources),
        "Backfill player helm (both axes AI) must satisfy the AI-helm gate"
    );
}

// (c) Player ship Backfill runs full operate_helm (avoidance + doctrine).
// Verifies that the player ship on Backfill goes through the same
// `operate_helm` decision (via the per-axis AI systems) as NPC ships — not
// a Reach-only stub — satisfying issue #587 AC.
#[test]
fn backfill_runs_full_operate_helm_with_objectives() {
    let mut app = test_app();
    // Give the ship a Destroy objective (non-Reach) pointing at an entity.
    let target_uuid = uuid::Uuid::new_v4().to_string();
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(target_uuid.clone()),
        crate::entities::spawner::EntityName("enemy_fighter".into()),
        Transform::from_xyz(80.0, 0.0, 0.0),
    ));
    set_ship_blackboard_objectives(
        &mut app,
        vec![destroy_scored_objective("enemy_fighter", 60.0)],
    );
    set_helm_control_source(&mut app, ControlSource::Ai);
    // Tactical's lock: what the helm pursues (issue #702).
    set_ship_weapons_target(&mut app, &target_uuid);

    tick(&mut app);

    let last = get_last_helm_input(&mut app);
    // The Destroy directive targets an entity at (80, 0). Full operate_helm
    // should produce non-zero thrust to pursue it.
    assert!(
        last.thrust > 0.0 || last.steering.abs() > 0.0,
        "player ship Backfill must run full operate_helm (non-Reach); \
         got thrust={}, steering={}",
        last.thrust,
        last.steering
    );
}

/// The `HELM_AI_MAX_DT_SECS` cap on the integration step, exercised
/// through the only path that can still reach it since issue #895: a
/// bare-`App` fixture, which authors no world and paces itself at the
/// 200 ms `TEST_TICK`.
///
/// In production the cap is dead. The sim runs in `FixedUpdate`, so the
/// step is `1 / [global] sim_tick_hz`, and `world::config::parse_world`
/// rejects any authored rate below `entity_config::MIN_SIM_TICK_HZ` —
/// derived from this very constant. So what this test pins is the fixture
/// contract, not a production frame-rate guard.
#[test]
fn backfill_helm_ai_caps_long_frame_yaw_step() {
    let mut app = test_app();
    let target_uuid = uuid::Uuid::new_v4().to_string();
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(target_uuid),
        crate::entities::spawner::EntityName("enemy_fighter".into()),
        Transform::from_xyz(80.0, 0.0, 0.0),
    ));
    set_ship_blackboard_objectives(
        &mut app,
        vec![destroy_scored_objective("enemy_fighter", 60.0)],
    );
    set_helm_control_source(&mut app, ControlSource::Ai);

    let before = get_ship_physics(&mut app);
    tick(&mut app);
    let after = get_ship_physics(&mut app);

    let max_step = ShipPhysicsConfig::new().max_yaw_rate * HELM_AI_MAX_DT_SECS;
    let yaw_delta = (after.yaw - before.yaw).abs();
    assert!(
        yaw_delta <= max_step + 0.0001,
        "AI helm must not consume a long frame as one oversized yaw step; \
         yaw_delta={yaw_delta}, max_step={max_step}"
    );
}

// ── The shared decision-surface frame (issue #824) ─────────────────────

/// Probe scratch for `all_four_axes_observe_the_same_frame`: snapshots of
/// the frame taken immediately after it is built and again after every
/// per-axis system has run.
#[derive(Resource, Default)]
struct FrameProbe {
    before: Option<String>,
    after: Option<String>,
}

fn probe_frame_before(frame: Res<HelmAiSurfacesFrame>, mut probe: ResMut<FrameProbe>) {
    probe.before = Some(format!("{:?}|{:?}", frame.anchors, frame.ships));
}

fn probe_frame_after(frame: Res<HelmAiSurfacesFrame>, mut probe: ResMut<FrameProbe>) {
    probe.after = Some(format!("{:?}|{:?}", frame.anchors, frame.ships));
}

/// AC (issue #824): all four per-axis systems observe the *same* frame —
/// the identical-inputs invariant is true by construction. The axis
/// systems take `Res<HelmAiSurfacesFrame>` (immutable), so the compiler
/// already forbids them mutating it; this pins the runtime half — nothing
/// else rebuilds or edits the frame between the builder and the last
/// axis system within a tick.
#[test]
fn all_four_axes_observe_the_same_frame() {
    let mut app = test_app();
    let anchor = "station-alpha";
    set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
    app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
    set_helm_control_source(&mut app, ControlSource::Ai);

    app.init_resource::<FrameProbe>();
    // `FixedUpdate` (issue #895): the probes bracket systems that live in
    // the fixed schedule, and ordering edges are only real within it.
    app.add_systems(
        FixedUpdate,
        (
            probe_frame_before
                .in_set(crate::sim_sets::SimSet::Physics)
                .after(build_helm_ai_surfaces_frame)
                .before(ai_helm_thrust)
                .before(ai_helm_steering)
                .before(ai_helm_lateral_thrust)
                .before(ai_helm_impulse)
                .run_if(ai_tick_ready),
            probe_frame_after
                .in_set(crate::sim_sets::SimSet::Physics)
                .after(ai_helm_thrust)
                .after(ai_helm_steering)
                .after(ai_helm_lateral_thrust)
                .after(ai_helm_impulse)
                // The cadence derivation that re-arms the latch lives in
                // `FixedLast` since #895, so it is unconditionally after
                // every `FixedUpdate` system — no explicit `.before` edge
                // needed.
                .run_if(ai_tick_ready),
        ),
    );

    tick(&mut app);

    let probe = app.world().resource::<FrameProbe>();
    let before = probe
        .before
        .as_ref()
        .expect("probe must run on the first (always-ready) AI tick");
    let after = probe
        .after
        .as_ref()
        .expect("probe must run after the four axis systems");
    assert!(
        before.contains("station-alpha") && before.contains("HelmAiShipFrame"),
        "precondition: the frame must actually carry a ship entry and the anchor, \
         else this equality is vacuous; got {before}"
    );
    assert_eq!(
        before, after,
        "the frame every axis observes must be identical before the first \
         and after the last per-axis system — a difference means something \
         mutated the shared decision surface mid-tick"
    );
}

/// AC (issue #824, work item 1): with per-entity Helm publishing, an NPC
/// ship's `helm_ai_radar_range` reads the live (damage-scaled) value from
/// its own Helm blackboard entry instead of the static
/// `HelmConsoleSection` fallback — which remains in place for ships whose
/// entry has not been published (low-LOD / missing-entry).
#[test]
fn helm_ai_radar_range_prefers_the_npc_blackboard_entry() {
    let helm_config = crate::entities::config::EntityConfig::from_toml(
        "[helm_console]\nmax_speed = 30.0\n\n[helm_console.radar]\nrange = 800.0\nshows = [\"ship\"]\n",
    )
    .unwrap()
    .helm_console
    .unwrap();
    let helm_section = crate::entities::spawner::HelmConsoleSection(helm_config);

    // With a published Helm entry (as per-entity publish now provides for
    // NPCs): the live, damage-scaled value wins.
    let mut bbs = crate::server_app::ShipSystemBlackboards::default();
    bbs.0.insert(
        crate::ship::system_registry::helm_station_key(),
        crate::core::messages::SystemBlackboard::Helm(crate::core::messages::HelmBlackboard {
            radar_range: 400.0,
            ..Default::default()
        }),
    );
    assert_eq!(
        helm_ai_radar_range(&bbs, Some(&helm_section), None, false),
        400.0,
        "an NPC with a published Helm entry must read the live radar_range"
    );

    // Without an entry (low-LOD ship / pre-first-publish): the static
    // config fallback is preserved.
    let empty_bbs = crate::server_app::ShipSystemBlackboards::default();
    assert_eq!(
        helm_ai_radar_range(&empty_bbs, Some(&helm_section), None, false),
        800.0,
        "a ship with no Helm entry must fall back to its authored radar range"
    );
}

// ── The Harrow Destroyer fly-through attack pass (issue #883) ────────────
//
// These drive the SHIPPED hull's authored policies through a real ticking
// app, so they fail on the content as well as on the code. Every assertion
// below is about something observable — an admitted actuator input, the
// ship's boost state, the committed policy state — never about an internal
// computation.
//
// Positions are set directly rather than flown, because flying a 200-unit
// approach at 200 ms per tick would take dozens of ticks and pin nothing
// extra: the interesting events are the merge and the tick after it, and
// setting the pose reaches them exactly and deterministically.

const BOGEY: &str = "bogey";

fn destroyer_hull() -> crate::entities::config::EntityConfig {
    crate::entities::config::EntityConfig::from_toml(
        crate::entities::include_resolve::resolve_from_disk(
            "assets/entities/ship_harrow_destroyer.toml",
        )
        .expect("ship_harrow_destroyer must resolve")
        .toml
        .as_str(),
    )
    .expect("the shipped destroyer hull must parse")
}

/// Put a single named target into the world snapshot at `pos`, heading
/// `yaw` at `speed`. The heading and speed matter: the closing rate is
/// reconstructed from them, so a target with its own velocity is a
/// genuinely different problem from a stationary one.
fn set_bogey(app: &mut App, uuid: uuid::Uuid, pos: [f32; 3], yaw: f32, speed: f32) {
    app.insert_resource(crate::ai::server::WorldSnapshot {
        entities: vec![crate::ai::AiWorldEntity {
            uuid,
            name: Some(BOGEY.into()),
            position: pos,
            yaw: Some(yaw),
            forward_speed: speed,
            radius: 3.0,
            size_rating: 3.0,
            movable: true,
            dangerous: true,
            ..Default::default()
        }],
    });
}

/// A ship carrying the shipped destroyer's three authored policy machines,
/// its physics envelope, and an enabled boost drive — the same components
/// `entities::spawner` would attach — hunting a single named bogey.
fn fly_through_app(bogey_pos: [f32; 3]) -> (App, uuid::Uuid) {
    fly_through_app_omitting(bogey_pos, &[])
}

/// As [`fly_through_app`], but with the named STEERING `param`s stripped
/// from the hull before its policy is built — the partially-authored hull
/// AGENTS.md #11 says must decline rather than invent.
///
/// Each name must actually be present to begin with, so this cannot quietly
/// pass by "removing" a param the hull renamed out from under it.
fn fly_through_app_omitting(bogey_pos: [f32; 3], omit: &[&str]) -> (App, uuid::Uuid) {
    let mut app = test_app();
    let cfg = destroyer_hull();
    let mut hc = cfg
        .helm_console
        .clone()
        .expect("hull declares [helm_console]");
    for name in omit {
        hc.steering_ai
            .as_mut()
            .expect("hull declares [helm_console.steering_ai]")
            .param
            .remove(*name)
            .unwrap_or_else(|| panic!("the shipped hull must author `{name}` to omit it"));
    }
    let boost = hc
        .boost
        .clone()
        .expect("hull declares [helm_console.boost]");
    let ship = find_ship_entity(&mut app);
    app.world_mut().entity_mut(ship).insert((
        crate::ship_plugin::ShipPhysicsConfigResource(crate::ship::physics::ShipPhysicsConfig {
            max_speed: hc.max_speed,
            max_reverse_speed: hc.max_reverse_speed,
            acceleration: hc.acceleration,
            deceleration: hc.deceleration,
            max_yaw_rate: hc.max_yaw_rate,
            ..crate::ship::physics::ShipPhysicsConfig::new()
        }),
        BoostConfigResource {
            enabled: true,
            multiplier: boost.multiplier,
            steering_multiplier: boost.steering_multiplier,
            active_duration: boost.active_duration,
            recharge_duration: boost.recharge_duration,
        },
    ));
    // Override the Engines/Steering/Boost entries of the ship's keyed
    // `FineSystemAiPolicies` map with the hull's own authored policies,
    // MERGING into (not replacing) the shipped defaults `test_app` attached —
    // the per-axis newtypes overrode one axis at a time before #1209.
    for (system_id, policy) in [
        (
            crate::ship::system_registry::helm_thrust_system_id(),
            hc.engines_ai.as_ref().unwrap().to_policy().unwrap(),
        ),
        (
            crate::ship::system_registry::helm_steering_system_id(),
            hc.steering_ai.as_ref().unwrap().to_policy().unwrap(),
        ),
        (
            crate::ship::system_registry::helm_boost_system_id(),
            hc.boost_ai.as_ref().unwrap().to_policy().unwrap(),
        ),
    ] {
        set_fine_policy(&mut app, ship, system_id, policy);
    }
    set_ship_blackboard_objectives(&mut app, vec![destroy_scored_objective(BOGEY, 80.0)]);
    set_helm_control_source(&mut app, ControlSource::Ai);
    let uuid = uuid::Uuid::new_v4();
    set_bogey(&mut app, uuid, bogey_pos, 0.0, 0.0);
    leave_the_defensive_leg(&mut app, "acquire");
    (app, uuid)
}

/// Spend the ONE tick the composed class doctrine takes to leave its
/// defensive resting leg (issue #878).
///
/// The shared movement fragments boot into `shadow` and unlock the aggressive
/// half on `fact(posture) >= param(press_posture)`. Every Harrow authors the
/// lowest rung (`press_posture = 0.0`), so the host-seeded posture satisfies
/// it on the very first evaluation — but it is still an evaluation, and the
/// fixtures below were written against machines that booted straight into
/// their first travelling leg. Settling it here keeps those tests statements
/// about the MANOEUVRE rather than about the one tick in front of it, and
/// asserting the leg was actually left is what stops the settle hiding a gate
/// that never opened.
fn leave_the_defensive_leg(app: &mut App, into: &str) {
    tick(app);
    assert_eq!(
        steering_state(app),
        into,
        "`press_posture = 0.0` must open the gate on the machine's FIRST \
         evaluation — a Harrow is always pressed, and a hull left shadowing \
         would hold a ring outside its own guns and never start the fight it \
         exists to start"
    );
}

fn steering_state(app: &mut App) -> String {
    app.world_mut()
        .query::<&HelmSteeringAiPolicyState>()
        .single(app.world())
        .expect("ship carries HelmSteeringAiPolicyState")
        .0
        .current
        .clone()
}

fn engines_state(app: &mut App) -> String {
    app.world_mut()
        .query::<&HelmEnginesAiPolicyState>()
        .single(app.world())
        .expect("ship carries HelmEnginesAiPolicyState")
        .0
        .current
        .clone()
}

fn pass_surface(app: &mut App) -> HelmPassSurface {
    *app.world_mut()
        .query::<&HelmPassSurface>()
        .single(app.world())
        .expect("ship carries HelmPassSurface")
}

fn boost_is_active(app: &mut App) -> bool {
    app.world_mut()
        .query::<&ShipBoost>()
        .single(app.world())
        .expect("ship carries ShipBoost")
        .0
        .is_active()
}

/// Fly the ship to a pose directly. Used to place it at the merge and past
/// it — see the module note above.
fn place_ship(app: &mut App, x: f32, z: f32, yaw: f32, speed: f32) {
    set_ship_physics(
        app,
        ShipPhysics {
            x,
            z,
            yaw,
            forward_speed: speed,
            ..Default::default()
        },
    );
}

/// AC5, stated as behaviour rather than as a fact dump: the machine's
/// `acquire → inbound` guard reads `fact(range_to_target)`, so the axis only
/// commits to a run once the target is inside the authored `commit_range`.
///
/// Before #883 the two travel axes were handed `AiFacts::new()` — an EMPTY
/// snapshot — so this guard would have validated at load and then been false
/// for ever, and the machine could never have left its initial state. That
/// this test can distinguish "far" from "near" at all is the proof the
/// travel axes now see seeded facts.
#[test]
fn travel_axis_facts_are_seeded_so_the_commit_range_guard_actually_gates() {
    // Authored `commit_range` is 260; put the bogey well beyond it.
    let (mut app, uuid) = fly_through_app([0.0, 0.0, -900.0]);
    tick_twice(&mut app);
    assert_eq!(
        steering_state(&mut app),
        "acquire",
        "a target beyond commit_range must not start a run"
    );
    assert_eq!(engines_state(&mut app), "acquire");

    // Bring it inside the authored commit range.
    set_bogey(&mut app, uuid, [0.0, 0.0, -150.0], 0.0, 0.0);
    tick(&mut app);
    assert_eq!(
        steering_state(&mut app),
        "inbound",
        "inside commit_range the machine must commit to the run — if this reads \
         `acquire` the travel axis is seeing empty facts again"
    );
    assert_eq!(
        engines_state(&mut app),
        "inbound",
        "Engines runs its OWN copy of the machine and must reach the same leg \
         from the same facts, not by reading Steering's state"
    );
}

/// The doctrine-phase balance tracer (issue #915): one event on the first
/// gated tick carrying the committed initial phase, silence while the phase
/// holds, and exactly one event per committed change — so the headless
/// report can fold per-ship time-in-phase without a per-tick stream.
#[test]
fn doctrine_phase_tracer_emits_initial_phase_and_each_change_once() {
    use bevy::ecs::message::Messages;

    let (mut app, bogey) = fly_through_app([0.0, 0.0, -900.0]);
    // Register the balance sink the tracer writes to. `init_resource` (not
    // `add_message`) so no per-frame double-buffer swap drops events
    // between cursor reads — same trick as the objective-tracer test.
    app.init_resource::<Messages<crate::core::balance::BalanceEvent>>();
    // The tracer names ships by uuid; the bare fixture ship has none.
    let ship = find_ship_entity(&mut app);
    let ship_uuid = uuid::Uuid::new_v4().to_string();
    app.world_mut()
        .entity_mut(ship)
        .insert(crate::entities::spawner::EntityUuid(ship_uuid.clone()));

    let mut cursor = app
        .world()
        .resource::<Messages<crate::core::balance::BalanceEvent>>()
        .get_cursor();
    // Reads every DoctrinePhaseChanged since the last call, as (ship, phase).
    let mut drain_phases = |app: &mut App| -> Vec<(String, String)> {
        let messages = app
            .world()
            .resource::<Messages<crate::core::balance::BalanceEvent>>();
        cursor
            .read(messages)
            .filter_map(|e| match e {
                crate::core::balance::BalanceEvent::DoctrinePhaseChanged { ship, phase } => {
                    Some((ship.clone(), phase.clone()))
                }
                _ => None,
            })
            .collect()
    };

    // Beyond commit_range the machine holds at `acquire`: the FIRST gated
    // tick reports the initial phase, later held ticks report nothing.
    tick_twice(&mut app);
    assert_eq!(engines_state(&mut app), "acquire");
    let first = drain_phases(&mut app);
    assert_eq!(
        first,
        vec![(ship_uuid.clone(), "acquire".to_string())],
        "the initial phase must be reported exactly once"
    );
    tick(&mut app);
    assert!(
        drain_phases(&mut app).is_empty(),
        "a held phase must not re-emit"
    );

    // Inside commit_range the machine commits to the run — one event with
    // the new phase.
    set_bogey(&mut app, bogey, [0.0, 0.0, -150.0], 0.0, 0.0);
    tick(&mut app);
    assert_eq!(engines_state(&mut app), "inbound");
    assert_eq!(
        drain_phases(&mut app),
        vec![(ship_uuid, "inbound".to_string())],
        "a committed change must be reported exactly once"
    );
}

/// AC1: the inbound leg is flown at the authored approach throttle, flat,
/// and Steering tracks the target continuously.
///
/// The throttle assertion is the one that separates this from
/// `helm_destroy`: at 60 units from a target whose `maintain_range` is 40,
/// the shared Destroy arm would be deep in its decel ramp and commanding a
/// small fraction of `target_speed`. The pass commands its full authored
/// approach fraction.
#[test]
fn inbound_leg_holds_the_authored_approach_speed_and_tracks_the_target() {
    let (mut app, uuid) = fly_through_app([0.0, 0.0, -200.0]);
    // Two ticks: the first publishes the pass surface, the second is the
    // first planner pass that consumes it (see `HelmPassSurface`).
    tick_twice(&mut app);
    tick(&mut app);

    let pass = pass_surface(&mut app);
    assert!(pass.active, "the destroyer must be flying an authored pass");
    assert!(!pass.escape, "still inbound");
    assert!(
        (get_thrust_input(&mut app) - pass.approach_speed).abs() < 1e-3,
        "inbound throttle must be the flat authored approach fraction ({}), got {}",
        pass.approach_speed,
        get_thrust_input(&mut app)
    );

    // Dead ahead: nothing to correct.
    place_ship(&mut app, 0.0, 0.0, 0.0, 15.0);
    set_bogey(&mut app, uuid, [0.0, 0.0, -200.0], 0.0, 0.0);
    tick(&mut app);
    tick(&mut app);
    assert!(
        get_steering_input(&mut app).abs() < 0.05,
        "a target dead ahead needs no turn, got {}",
        get_steering_input(&mut app)
    );

    // The target MOVES off the starboard bow: the facing solution is
    // re-derived, so the ship turns after it.
    place_ship(&mut app, 0.0, 0.0, 0.0, 15.0);
    set_bogey(&mut app, uuid, [180.0, 0.0, -100.0], 0.0, 0.0);
    tick(&mut app);
    assert!(
        get_steering_input(&mut app) > 0.0,
        "Steering must keep tracking a MOVING target while inbound, got {}",
        get_steering_input(&mut app)
    );
}

/// Drive one full merge: approach, pass through the closest point, and open
/// out again. Leaves the app in the `escape` leg for the tests below.
fn run_to_escape() -> (App, uuid::Uuid) {
    let (mut app, uuid) = fly_through_app([0.0, 0.0, -200.0]);
    // Commit to the run.
    place_ship(&mut app, 0.0, 0.0, 0.0, 20.0);
    tick_twice(&mut app);
    assert_eq!(steering_state(&mut app), "inbound");

    // THE MERGE: the ship is right on top of the bogey, so the host folds a
    // small `min_range_seen` into every axis's private memory. Still
    // closing, so nothing transitions yet.
    place_ship(&mut app, 0.0, -195.0, 0.0, 20.0);
    tick(&mut app);
    assert_eq!(
        steering_state(&mut app),
        "inbound",
        "at the merge itself the range is still shrinking: not yet closest approach"
    );

    // PAST IT: the ship has flown through and the range is opening again,
    // well past the authored hysteresis.
    place_ship(&mut app, 0.0, -260.0, 0.0, 20.0);
    tick(&mut app);
    (app, uuid)
}

/// AC2: closest approach ends target tracking and commits Engines, Steering
/// and Boost to the heading held at the merge.
///
/// All three axes must arrive together — and they do it by each running
/// their own machine over the same seeded facts, which is why the assertion
/// is over both travel states and the published leg rather than over one
/// shared flag.
#[test]
fn closest_approach_commits_every_axis_to_the_outward_heading() {
    let (mut app, _uuid) = run_to_escape();

    assert_eq!(
        steering_state(&mut app),
        "escape",
        "the closing rate went negative and the range opened past the authored \
         hysteresis: that is closest approach"
    );
    assert_eq!(
        engines_state(&mut app),
        "escape",
        "Engines must reach the escape leg independently, from the same facts"
    );

    let pass = pass_surface(&mut app);
    assert!(pass.escape, "the published leg must be the escape");
    assert!(
        pass.escape_heading_rad.abs() < 1e-3,
        "the frozen heading must be the yaw held AT the merge (0), got {}",
        pass.escape_heading_rad
    );
}

/// AC2's fly-through half, and the reason `hold_committed_heading` exists as
/// a verb rather than as a bare "hold".
///
/// Once committed, the target is swung hard onto the beam. A tracking axis
/// would haul the ship round after it — straight back into the ship it just
/// passed. The committed axis flies its frozen heading and ignores it.
#[test]
fn the_escape_leg_ignores_the_target_and_flies_the_frozen_heading() {
    let (mut app, uuid) = run_to_escape();
    let frozen = pass_surface(&mut app).escape_heading_rad;

    // Hard to starboard — a bearing that would saturate a tracking solution.
    set_bogey(&mut app, uuid, [400.0, 0.0, -260.0], 0.0, 0.0);
    // Put the hull exactly ON the frozen heading first, so the only thing
    // that can command yaw this tick is the target. Left to fly, the boosted
    // escape carries up to one tick of yaw rate as a residual (see the
    // one-tick-offset note on `HelmPassSurface`), and this test would be
    // measuring that convergence rather than the commitment it is about.
    place_ship(&mut app, 0.0, -260.0, frozen, 20.0);
    tick(&mut app);

    assert_eq!(
        steering_state(&mut app),
        "escape",
        "nothing the target does may end the escape leg"
    );
    // Asserted as a SIGN rather than as a magnitude, because the pin above
    // makes the expected value exactly zero and an `abs() < tolerance` band
    // around zero would pass for any regression small enough to be quiet.
    // The bogey is hard to STARBOARD, so a leg that tracked it would command
    // a saturated POSITIVE yaw — which this catches, while still admitting
    // the free-flight case where the hull is converging back onto the frozen
    // heading from the other side.
    assert!(
        get_steering_input(&mut app) <= 0.0,
        "the escape must fly the FROZEN heading, not turn back onto the target \
         off the starboard beam; got steering {}",
        get_steering_input(&mut app)
    );
    assert!(
        (pass_surface(&mut app).escape_heading_rad - frozen).abs() < 1e-3,
        "the frozen heading must not be re-derived while the leg runs"
    );
    // ...and it is still driving away under power.
    assert!(
        get_thrust_input(&mut app) > 0.0,
        "the escape leg never brakes"
    );
}

// ── #918: a committed leg keeps its facing; a travelling one still yields ──

/// A standing channel-3 arc-bearing request for `target`, from a family of
/// FIXED FORE emitters that reach a long way — the shape every hull in the
/// fleet raises, because a fore tube's reach is `speed * lifespan` and its
/// arc is 90 degrees, so it is effectively always in reach and easily out of
/// arc.
///
/// Re-inserted rather than inserted once wherever a test runs several
/// ticks — not because Weapons re-raises it that way. `tick_weapons_arc_
/// request` is DEBOUNCED (`src/console/weapons/mod.rs:651-657`) and only
/// re-fires on a change to the family, target, or usable-arc set, never on
/// every tick the same miss persists. This bare fixture runs no real
/// Weapons system to raise the request at all, so the harness stands it up
/// directly; re-inserting on every tick is what keeps that self-contained
/// rather than leaning on a single insert to persist through however the
/// fixture is wired. It changes nothing a persisting request would not
/// already show — nothing about a decline clears `pending.target` on its
/// own — so a decline can never be mistaken for the request having quietly
/// expired.
fn stand_up_fore_arc_request(app: &mut App, target: uuid::Uuid) {
    let ship = find_ship_entity(app);
    app.world_mut()
        .entity_mut(ship)
        .insert(PendingArcBearingRequest {
            target: Some(target),
            arcs: vec![crate::core::messages::WeaponEmitterArc {
                facing_deg: 0.0,
                arc_deg: 90.0,
                range: 500.0,
            }],
        });
}

/// The yaw the ship's own DOCTRINE solved this tick, decoded from the shared
/// motion plan exactly as `ai_helm_steering` decodes it.
///
/// This is the reference issue #918 asks the sawtooth to be measured
/// against: the doctrine's own solved heading, not a range. A hull carries
/// momentum either way, so a range assertion cannot tell a ring that was
/// overwritten from one that was merely shoved.
fn doctrine_steering(app: &mut App) -> f32 {
    let ship = find_ship_entity(app);
    let plan = app
        .world()
        .resource::<crate::ship::helm_planner::HelmMotionPlan>();
    let sp = plan
        .ships
        .get(&ship)
        .copied()
        .expect("the planner published this ship's motion plan");
    crate::ai::decode_steering_from_facing(sp.motion.desired_facing_local.to_array())
}

/// Whether the ship is still carrying an unanswered arc-bearing request.
fn arc_request_stands(app: &mut App) -> bool {
    let ship = find_ship_entity(app);
    app.world()
        .get::<PendingArcBearingRequest>(ship)
        .and_then(|p| p.target)
        .is_some()
}

/// **Issue #918: the destroyer's `escape` leg holds its frozen heading under
/// a STANDING out-of-arc request — the case #875 documented as inviolable
/// and that nothing enforced.**
///
/// The sibling of
/// `the_escape_leg_ignores_the_target_and_flies_the_frozen_heading`, and the
/// same fixture and pose, with one thing added: a channel-3 request for the
/// bogey that has just been swung hard onto the beam. Before #918 that
/// request replaced the escape's steering with a full bow-on tracking
/// solution — the target could not end the dwell but a gun that could not
/// bear could turn the hull straight back into it, which is the same thing
/// by a different route.
///
/// Both halves are asserted, because either alone would pass on a bug:
/// the admitted yaw must be the yaw the planner solved for the FROZEN
/// heading, and the request must still be STANDING afterwards. A request
/// that had merely been satisfied would leave the first assertion true while
/// proving nothing about precedence.
#[test]
fn the_escape_leg_declines_a_standing_arc_bearing_request() {
    let (mut app, uuid) = run_to_escape();
    let frozen = pass_surface(&mut app).escape_heading_rad;

    // Hard to starboard, exactly as the sibling test does: a bearing that
    // saturates a tracking solution, and one no fore arc can cover.
    set_bogey(&mut app, uuid, [400.0, 0.0, -260.0], 0.0, 0.0);
    place_ship(&mut app, 0.0, -260.0, frozen, 20.0);
    stand_up_fore_arc_request(&mut app, uuid);
    tick(&mut app);

    assert_eq!(
        steering_state(&mut app),
        "escape",
        "the request must not change which leg is flown either"
    );
    assert!(
        arc_request_stands(&mut app),
        "the request must be DECLINED, not consumed: it is still out of arc and \
         well within reach, so the only reason the hull did not turn must be the \
         leg's own declaration"
    );
    let (admitted, solved) = (get_steering_input(&mut app), doctrine_steering(&mut app));
    assert!(
        (admitted - solved).abs() < 1e-3,
        "the escape must fly the heading its own doctrine solved ({solved}), but the \
         admitted yaw was {admitted}: the arc-bearing request overwrote a committed \
         leg"
    );
    assert!(
        admitted <= 0.0,
        "and the sign says which way it was pulled — the bogey is hard to \
         STARBOARD, so a hull obeying the request commands a saturated POSITIVE \
         yaw; got {admitted}"
    );
}

/// **Issue #918's other half, on the SAME hull and the SAME request: a leg
/// that is only travelling still turns to bring the family to bear
/// (#673-#684 preserved).**
///
/// This is the control that makes the test above mean anything. `inbound`
/// leaves `yields_to_arc_requests` at its default, so the identical standing
/// request — same emitter arc, same bogey, same hull, same tick — takes the
/// facing. The two tests differ in exactly one thing: which leg the helm is
/// flying. Nothing about the requester differs, and there is nowhere in the
/// path it could (AGENTS.md #6).
#[test]
fn a_travelling_leg_still_turns_to_bear_under_the_same_request() {
    let (mut app, uuid) = fly_through_app([0.0, 0.0, -200.0]);
    place_ship(&mut app, 0.0, 0.0, 0.0, 20.0);
    tick_twice(&mut app);
    assert_eq!(
        steering_state(&mut app),
        "inbound",
        "the fixture must be on a TRAVELLING leg for this control to mean anything"
    );

    // The bogey dead ahead, so the doctrine's own solution needs no turn at
    // all: any yaw at all is then attributable to the request.
    set_bogey(&mut app, uuid, [0.0, 0.0, -200.0], 0.0, 0.0);
    place_ship(&mut app, 0.0, 0.0, 0.0, 20.0);
    tick(&mut app);
    assert!(
        get_steering_input(&mut app).abs() < 0.05,
        "control condition: dead ahead the inbound leg commands no turn, got {}",
        get_steering_input(&mut app)
    );

    // Now a contact off the starboard beam that no fore arc covers, with the
    // hull still pointed at it... and the travelling leg turns to bear.
    let bearing = uuid::Uuid::new_v4();
    app.insert_resource(crate::ai::server::WorldSnapshot {
        entities: vec![
            crate::ai::AiWorldEntity {
                uuid,
                name: Some(BOGEY.into()),
                position: [0.0, 0.0, -200.0],
                yaw: Some(0.0),
                radius: 3.0,
                size_rating: 3.0,
                movable: true,
                dangerous: true,
                ..Default::default()
            },
            crate::ai::AiWorldEntity {
                uuid: bearing,
                name: Some("beam contact".into()),
                position: [200.0, 0.0, -1.0],
                yaw: Some(0.0),
                radius: 3.0,
                size_rating: 3.0,
                movable: true,
                dangerous: true,
                ..Default::default()
            },
        ],
    });
    stand_up_fore_arc_request(&mut app, bearing);
    tick(&mut app);

    assert!(
        get_steering_input(&mut app) > 0.05,
        "a helm on a travelling leg must still turn to bring a family that cannot \
         bear onto its target — the whole point of #673-#684 — got {}",
        get_steering_input(&mut app)
    );
}

// ── #932: a standing request is WITHDRAWN when its family goes unusable ──

/// Withdraw the standing arc-bearing request through the REAL channel-3
/// wire — `handle_coordination_enqueue`, `process_coordination_lag`, then
/// Helm's `receive_helm_coordination`, all already running in this fixture —
/// rather than poking `PendingArcBearingRequest` directly the way
/// `stand_up_fore_arc_request` stands one up.
///
/// `stand_up_fore_arc_request` pokes state because no real Weapons system
/// runs in this bare-App fixture to raise a request in the first place;
/// withdrawing it the same way would prove nothing about the wire issue
/// #932 actually changed. `tick_weapons_arc_request` — the real emitter,
/// exercised on its own terms in `console::weapons::server_tests` — raises
/// exactly this `ArcBearingWithdraw` when the family it last asked for drains
/// to empty; this helper drives the SAME payload down the SAME bus the lag
/// router delivers and Helm's receiver consumes, so what's under test here is
/// entirely the consuming half.
///
/// Zeroes `coordination_lag_secs` first so the withdrawal is due the same
/// tick it's enqueued (production ships lag it; see the `coord_test_app`
/// pattern in `coordination_systems.rs`'s own tests for the precedent).
/// The lag router and Helm receiver run in `SimSet::Modifiers`, AFTER
/// `ai_helm_steering` in `SimSet::Physics`, so the clear lands too late to
/// affect the tick it's consumed in — callers tick once more to observe a
/// cleared `PendingArcBearingRequest` reflected in steering.
fn withdraw_fore_arc_request(app: &mut App, family: crate::core::messages::WeaponFamily) {
    let ship = find_ship_entity(app);
    {
        let mut cfg = app
            .world_mut()
            .get_mut::<ShipConfigComponent>(ship)
            .expect("ship carries ShipConfigComponent");
        cfg.0.coordination_lag_secs = 0.0;
    }
    app.world_mut()
        .resource_mut::<Messages<crate::ship_plugin::CoordinationEnqueue>>()
        .write(crate::ship_plugin::CoordinationEnqueue {
            source_entity: ship,
            sender_origin: ControlSource::Ai,
            address: crate::core::messages::CoordinationAddress::Station(
                crate::core::messages::StationId(
                    crate::ship::system_registry::HELM_STATION_ID.into(),
                ),
            ),
            payload: crate::core::messages::CoordinationPayload::ArcBearingWithdraw { family },
            presentation: crate::core::messages::CoordinationPresentation::titled(
                "coordination.arc_withdraw.title",
            ),
            sender_label: "Weapons".to_string(),
            sender_system: crate::core::messages::SystemId(String::new()),
        });
    tick(app);
}

/// **Issue #932, the hole #918 documented and left open: a standing
/// request whose emitting family goes unusable is WITHDRAWN, so a later
/// yielding leg has nothing left to honour.**
///
/// Same fixture and same standing request as
/// `a_travelling_leg_still_turns_to_bear_under_the_same_request` — a
/// travelling ("inbound") leg, dead-ahead pursuit target, and a beam
/// contact the fore arc cannot cover — with one thing added: before the
/// leg gets to react to it, the family that raised the request (Torpedoes,
/// standing in for "every tube just emptied") is withdrawn. The leg still
/// yields — nothing about #918's consent changes — but there is no
/// bearing left in `PendingArcBearingRequest` for it to yield TO.
#[test]
fn a_withdrawn_arc_request_does_not_turn_a_yielding_leg() {
    let (mut app, uuid) = fly_through_app([0.0, 0.0, -200.0]);
    place_ship(&mut app, 0.0, 0.0, 0.0, 20.0);
    tick_twice(&mut app);
    assert_eq!(
        steering_state(&mut app),
        "inbound",
        "the fixture must be on a TRAVELLING (yielding) leg for this to mean anything"
    );

    // Dead ahead pursuit target, so the doctrine's own solution needs no
    // turn at all: any yaw at all is attributable to the request.
    let bearing = uuid::Uuid::new_v4();
    app.insert_resource(crate::ai::server::WorldSnapshot {
        entities: vec![
            crate::ai::AiWorldEntity {
                uuid,
                name: Some(BOGEY.into()),
                position: [0.0, 0.0, -200.0],
                yaw: Some(0.0),
                radius: 3.0,
                size_rating: 3.0,
                movable: true,
                dangerous: true,
                ..Default::default()
            },
            crate::ai::AiWorldEntity {
                uuid: bearing,
                name: Some("beam contact".into()),
                position: [200.0, 0.0, -1.0],
                yaw: Some(0.0),
                radius: 3.0,
                size_rating: 3.0,
                movable: true,
                dangerous: true,
                ..Default::default()
            },
        ],
    });
    place_ship(&mut app, 0.0, 0.0, 0.0, 20.0);
    stand_up_fore_arc_request(&mut app, bearing);

    // The torpedo tubes that raised this request just ran dry: withdraw
    // it, exactly as `tick_weapons_arc_request` now does on its own.
    withdraw_fore_arc_request(&mut app, crate::core::messages::WeaponFamily::Torpedoes);
    assert!(
        !arc_request_stands(&mut app),
        "the withdrawal must clear PendingArcBearingRequest"
    );

    // One more tick: `ai_helm_steering` now reads the CLEARED request.
    // Nothing in this bare-App fixture re-raises it — there is no real
    // Weapons system running here to do so.
    tick(&mut app);

    assert!(
        get_steering_input(&mut app).abs() < 0.05,
        "a WITHDRAWN request must not turn a yielding leg — the family that \
         raised it has nothing left to bring to bear; got steering {}",
        get_steering_input(&mut app)
    );
}

/// The positive control for the test above: the SAME fixture, the SAME
/// standing request, but never withdrawn — a yielding leg still turns to
/// bring a still-usable family to bear. Pins that #932's withdrawal is the
/// only thing that changed; a yielding leg's consent (#918) did not.
#[test]
fn an_unwithdrawn_arc_request_still_turns_a_yielding_leg() {
    let (mut app, uuid) = fly_through_app([0.0, 0.0, -200.0]);
    place_ship(&mut app, 0.0, 0.0, 0.0, 20.0);
    tick_twice(&mut app);
    assert_eq!(steering_state(&mut app), "inbound");

    let bearing = uuid::Uuid::new_v4();
    app.insert_resource(crate::ai::server::WorldSnapshot {
        entities: vec![
            crate::ai::AiWorldEntity {
                uuid,
                name: Some(BOGEY.into()),
                position: [0.0, 0.0, -200.0],
                yaw: Some(0.0),
                radius: 3.0,
                size_rating: 3.0,
                movable: true,
                dangerous: true,
                ..Default::default()
            },
            crate::ai::AiWorldEntity {
                uuid: bearing,
                name: Some("beam contact".into()),
                position: [200.0, 0.0, -1.0],
                yaw: Some(0.0),
                radius: 3.0,
                size_rating: 3.0,
                movable: true,
                dangerous: true,
                ..Default::default()
            },
        ],
    });
    place_ship(&mut app, 0.0, 0.0, 0.0, 20.0);
    stand_up_fore_arc_request(&mut app, bearing);
    tick(&mut app);

    assert!(
        get_steering_input(&mut app) > 0.05,
        "an un-withdrawn standing request from a still-usable family must still \
         turn a yielding leg to bear — got {}",
        get_steering_input(&mut app)
    );
}

/// The escape leg outlives its target — which in combat is the ORDINARY
/// case, because the pass is what kills the target.
///
/// `plan_fly_through_pass` never reads `target_pos` on the escape leg, and
/// neither escape state carries a `target_valid < 1` transition (only
/// `inbound` does), so a destroyer that has committed must keep flying the
/// frozen heading at the authored escape throttle with nothing in the world
/// at all. Gating the escape on a resolvable target instead dropped it back
/// to ordinary doctrine travel, which — with no objective geometry left —
/// commands zero thrust and zero yaw: the hull brakes to a standstill in the
/// middle of its escape while the Boost machine, independent of the planner,
/// keeps the drive lit for the remaining dwell.
#[test]
fn the_escape_leg_survives_its_target_dying() {
    let (mut app, _uuid) = run_to_escape();
    let escape_speed = pass_surface(&mut app).escape_speed;
    let frozen = pass_surface(&mut app).escape_heading_rad;

    // The target is destroyed: nothing left in the world to resolve, so the
    // frame has no destroy target and no merged-view entity for it.
    app.insert_resource(crate::ai::server::WorldSnapshot {
        entities: Vec::new(),
    });
    // Put the hull off the frozen heading by well more than the authored
    // deadband, so "still solving against the frozen heading" is observable
    // as a real correction rather than as a deadbanded zero.
    place_ship(&mut app, 0.0, -260.0, frozen + 0.3, 20.0);
    tick_twice(&mut app);

    assert_eq!(
        steering_state(&mut app),
        "escape",
        "only the authored dwell ends the escape — a dead target must not"
    );
    assert!(
        pass_surface(&mut app).active && pass_surface(&mut app).escape,
        "the published surface must still be an active escape leg"
    );
    assert!(
        (get_thrust_input(&mut app) - escape_speed).abs() < 1e-3,
        "the escape must still be flown at the authored escape throttle ({escape_speed}), \
         got {} — a target-gated escape brakes the destroyer to a standstill",
        get_thrust_input(&mut app)
    );
    assert!(
        get_steering_input(&mut app).abs() > 0.05,
        "the escape must still SOLVE against the frozen heading with the hull \
         0.3 rad off it, got steering {}",
        get_steering_input(&mut app)
    );
    assert!(
        (pass_surface(&mut app).escape_heading_rad - frozen).abs() < 1e-3,
        "and the frozen heading itself is untouched by the target's death"
    );
}

/// AC8's boost-out: the authored escape rule lights the drive through the
/// same admitted `SetBoost` a human sends, and only on the escape leg.
#[test]
fn the_escape_leg_boosts_out_and_the_approach_does_not() {
    let (mut app, _uuid) = fly_through_app([0.0, 0.0, -200.0]);
    place_ship(&mut app, 0.0, 0.0, 0.0, 20.0);
    tick_twice(&mut app);
    assert_eq!(steering_state(&mut app), "inbound");
    assert!(
        !boost_is_active(&mut app),
        "the approach is flown at normal speed: boost stays cold"
    );

    let (mut app, _uuid) = run_to_escape();
    tick(&mut app);
    assert_eq!(steering_state(&mut app), "escape");
    assert!(
        boost_is_active(&mut app),
        "the escape leg must engage boost through the shared admitted SetBoost path"
    );
}

/// AC3: shared hazard avoidance BENDS the escape without changing any pass
/// state.
///
/// This is why the frozen heading is expressed as a desired FACING through
/// the motion planner rather than as a raw `SteeringInput` override: the
/// #780 hazard contribution is folded into the same pure arm, so it still
/// composes. A raw override would fly the destroyer straight through the
/// rock.
#[test]
fn a_hazard_bends_the_escape_without_changing_the_pass_state() {
    let (mut app, uuid) = run_to_escape();
    let before_state = steering_state(&mut app);
    let before_heading = pass_surface(&mut app).escape_heading_rad;

    // Clear escape first: nothing to avoid, so no yaw. On the frozen heading
    // exactly, so the baseline is not reading the boosted escape's own
    // one-tick convergence residual (see `HelmPassSurface`).
    set_bogey(&mut app, uuid, [0.0, 0.0, -100.0], 0.0, 0.0);
    place_ship(&mut app, 0.0, -260.0, before_heading, 20.0);
    tick(&mut app);
    // A sign, not a band around zero: the pin above makes the expected value
    // exactly zero, so `abs() < tolerance` would wave through any quiet
    // regression. The bogey is dead ASTERN of the frozen heading, so a leg
    // that tracked it would command a saturated positive yaw to haul the
    // ship round; the negative half stays in scope for the free-flight case.
    assert!(
        get_steering_input(&mut app) <= 0.0,
        "baseline: an unobstructed escape commands no yaw toward the target, got {}",
        get_steering_input(&mut app)
    );

    // Drop a rock onto the projected escape path, keeping the bogey where
    // it was so the only new thing in the world is the hazard.
    app.insert_resource(crate::ai::server::WorldSnapshot {
        entities: vec![
            crate::ai::AiWorldEntity {
                uuid,
                name: Some(BOGEY.into()),
                position: [0.0, 0.0, -100.0],
                yaw: Some(0.0),
                radius: 3.0,
                size_rating: 3.0,
                movable: true,
                dangerous: true,
                ..Default::default()
            },
            crate::ai::AiWorldEntity {
                uuid: uuid::Uuid::new_v4(),
                name: Some("rock".into()),
                position: [4.0, 0.0, -320.0],
                yaw: None,
                radius: 8.0,
                size_rating: 8.0,
                movable: false,
                dangerous: true,
                ..Default::default()
            },
        ],
    });
    tick(&mut app);

    assert!(
        get_steering_input(&mut app).abs() > 0.0,
        "a hazard on the escape path must bend the escape"
    );
    assert_eq!(
        steering_state(&mut app),
        before_state,
        "avoidance is a steering force, NOT an input to the pass state machine"
    );
    assert!(
        pass_surface(&mut app).escape,
        "and the published leg is unchanged"
    );
    assert!(
        (pass_surface(&mut app).escape_heading_rad - before_heading).abs() < 1e-3,
        "bending the escape must not rewrite the committed heading"
    );
}

/// The running range minimum is scoped to the TARGET as well as to the
/// state, so a mid-`inbound` target switch cannot fire a closest approach
/// the destroyer never flew.
///
/// The machine calls closest approach from `range_above_min_seen`, i.e.
/// `range_to_target - memory(min_range_seen)`. Fold that across a target
/// swap and the minimum belongs to a different ship: pick up a new contact
/// further out and the subtraction reads as a huge re-opening, every
/// conjunct of the authored guard passes at once, and the destroyer commits
/// to an escape from a target it has not even closed on yet.
///
/// The two halves below differ ONLY in whether the bogey keeps its uuid, so
/// this pins the identity scoping specifically and not merely "no commit".
#[test]
fn a_mid_inbound_target_switch_does_not_fire_a_spurious_closest_approach() {
    // Positive control: the SAME target, first close and then far astern.
    // That is a genuine fly-through, and it must still commit.
    let (mut app, uuid) = fly_through_app([0.0, 0.0, -50.0]);
    place_ship(&mut app, 0.0, 0.0, 0.0, 20.0);
    tick_twice(&mut app);
    assert_eq!(steering_state(&mut app), "inbound");
    set_bogey(&mut app, uuid, [0.0, 0.0, 200.0], 0.0, 0.0);
    place_ship(&mut app, 0.0, 0.0, 0.0, 20.0);
    tick(&mut app);
    assert_eq!(
        steering_state(&mut app),
        "escape",
        "the same target, now 200 units astern of a ship still driving away \
         from it, IS a closest approach"
    );

    // The real case: a DIFFERENT ship inherits the bogey role mid-run, and
    // it happens to be further out than the minimum the previous target
    // accumulated. Nothing about the destroyer's own run has changed.
    let (mut app, _uuid) = fly_through_app([0.0, 0.0, -50.0]);
    place_ship(&mut app, 0.0, 0.0, 0.0, 20.0);
    tick_twice(&mut app);
    assert_eq!(steering_state(&mut app), "inbound");
    set_bogey(&mut app, uuid::Uuid::new_v4(), [0.0, 0.0, 200.0], 0.0, 0.0);
    place_ship(&mut app, 0.0, 0.0, 0.0, 20.0);
    tick(&mut app);
    assert_eq!(
        steering_state(&mut app),
        "inbound",
        "a target SWITCH restarts the range fold: the old target's minimum must \
         not synthesise a range_above_min_seen spike and commit the escape"
    );
    assert_eq!(
        engines_state(&mut app),
        "inbound",
        "Engines runs its own copy of the machine over its own private memory \
         and must be scoped the same way"
    );
    assert!(
        !pass_surface(&mut app).escape,
        "and no escape leg is published"
    );
}

/// A stateless policy for one travel channel whose single rule is gated on
/// `fact(boost_available)` and nothing else — so whether the axis actuates
/// at all is a direct readout of what the host seeded that fact to.
fn boost_availability_gated_policy(
    channel: &str,
    verb: crate::ai::policy::AiPolicyVerb,
) -> crate::ai::policy::AiPolicy {
    crate::ai::policy::AiPolicy {
        params: crate::world::flags::AiParams::new(),
        rules: vec![crate::ai::policy::AiPolicyRule {
            priority: 10,
            channel: channel.into(),
            when: crate::world::flags::parse_predicate("fact(boost_available) > 0").unwrap(),
            verb,
        }],
        idle: false,
        machine: None,
    }
}

/// A ship chasing a Reach anchor off the starboard bow, with `policy` merged
/// onto the given travel `axis` (issue #1209 — merged into the keyed
/// `FineSystemAiPolicies` map `test_app` already attached, so it OVERRIDES
/// that one axis and leaves the others' shipped defaults, exactly as the
/// per-axis newtype insert did) and boost capability present or absent.
fn availability_fact_app(
    axis: crate::core::messages::SystemId,
    policy: crate::ai::policy::AiPolicy,
    boost_enabled: Option<bool>,
) -> App {
    let mut app = test_app();
    let anchor = "station-alpha";
    set_ship_blackboard_objectives(&mut app, vec![reach_scored_objective(anchor, 10.0)]);
    app.insert_resource(world_config_with_anchor(anchor, [100.0, 0.0, 0.0]));
    set_per_axis_helm_ai(&mut app);
    let ship = find_ship_entity(&mut app);
    set_fine_policy(&mut app, ship, axis, policy);
    if let Some(enabled) = boost_enabled {
        app.world_mut()
            .entity_mut(ship)
            .insert(crate::ship::components::BoostConfigResource {
                enabled,
                ..Default::default()
            });
    }
    app
}

/// The #779 empty-facts trap, one fact narrower: `ai_helm_thrust` and
/// `ai_helm_steering` used to pass a HARDCODED `false` for both availability
/// facts, so a travel-axis guard on `fact(boost_available)` validated at load
/// and then read 0 for ever — silently wrong in exactly the way an absent
/// fact is. They now seed it from the ship's own `BoostConfigResource`, as
/// `ai_policy_state_tick` and `ai_helm_boost` already did.
#[test]
fn a_travel_axis_guard_reads_the_real_boost_availability() {
    // ── Engines ─────────────────────────────────────────────────────────
    let mut available = availability_fact_app(
        crate::ship::system_registry::helm_thrust_system_id(),
        boost_availability_gated_policy(
            crate::entities::config::HELM_LONGITUDINAL_CHANNEL,
            crate::ai::policy::AiPolicyVerb::ActuateDesiredTravel,
        ),
        Some(true),
    );
    tick(&mut available);
    assert!(
        get_thrust_input(&mut available) > 0.0,
        "an Engines guard on fact(boost_available) must fire on a ship that HAS \
         an enabled boost drive; got thrust {}",
        get_thrust_input(&mut available)
    );

    let mut unavailable = availability_fact_app(
        crate::ship::system_registry::helm_thrust_system_id(),
        boost_availability_gated_policy(
            crate::entities::config::HELM_LONGITUDINAL_CHANNEL,
            crate::ai::policy::AiPolicyVerb::ActuateDesiredTravel,
        ),
        None,
    );
    tick(&mut unavailable);
    assert_eq!(
        get_thrust_input(&mut unavailable),
        0.0,
        "and must NOT fire on a ship with no boost drive — the fact is a real \
         reading of capability, not a constant"
    );

    // ── Steering ────────────────────────────────────────────────────────
    let mut available = availability_fact_app(
        crate::ship::system_registry::helm_steering_system_id(),
        boost_availability_gated_policy(
            crate::entities::config::HELM_YAW_CHANNEL,
            crate::ai::policy::AiPolicyVerb::ActuateDesiredFacing,
        ),
        Some(true),
    );
    tick(&mut available);
    assert!(
        get_steering_input(&mut available).abs() > 0.0,
        "a Steering guard on fact(boost_available) must fire on a ship that HAS \
         an enabled boost drive; got steering {}",
        get_steering_input(&mut available)
    );

    let mut unavailable = availability_fact_app(
        crate::ship::system_registry::helm_steering_system_id(),
        boost_availability_gated_policy(
            crate::entities::config::HELM_YAW_CHANNEL,
            crate::ai::policy::AiPolicyVerb::ActuateDesiredFacing,
        ),
        Some(false),
    );
    tick(&mut unavailable);
    assert_eq!(
        get_steering_input(&mut unavailable),
        0.0,
        "a feature-DISABLED boost drive reads unavailable too, exactly as it does \
         on the boost axis itself"
    );
}

/// AC5's reset, on the new axes: a travel axis that is not AI-operated holds
/// its machine at the authored initial state, so the tick AI gains control
/// never resumes a stale mid-pass leg.
#[test]
fn a_human_held_travel_axis_holds_its_machine_at_initial() {
    let (mut app, _uuid) = run_to_escape();
    assert_eq!(steering_state(&mut app), "escape");

    set_helm_control_source(&mut app, ControlSource::Human);
    tick(&mut app);
    // "shadow" IS the authored initial state since issue #878 composed this
    // hull on the class fragment: the shared doctrine boots defensive and the
    // Harrow unlocks the aggressive half by posture. A human-flown axis is
    // reset to what the FILE says, whatever that is, which is the claim.
    assert_eq!(
        steering_state(&mut app),
        "shadow",
        "a human-flown axis resets to the authored initial state"
    );
    assert_eq!(engines_state(&mut app), "shadow");
    assert!(
        !pass_surface(&mut app).active,
        "and the planner stops being offered a pass at all"
    );
}

// ── AC5 through the real damage path (PRD #774, US15) ────────────────────
//
// The reset above is reached by moving a control SOURCE, and
// `stateful_policy_state_resets_when_the_system_is_unavailable_and_on_recovery`
// reaches it by stripping a capability component off the entity. Between
// them they cover the "AI gains control" half of US15 on both a synthetic
// policy and this shipped hull, so nothing below duplicates it.
//
// Neither is how a system goes offline in PLAY. In play the hull takes a
// hit, `sync_console_damage_tiers` reads the new damage tier and folds it
// into `ControlSourceResolver::offline_systems`, and `policy_for(..)
// .operate_ai` goes false as a consequence — and that chain, plus the repair
// that undoes it, is what US15's "repaired-system recovery" names. The test
// below is the only one that drives it.

/// Max HP the fixture below gives each damage-modelled fine helm system.
const HELM_ACTUATOR_MAX_HP: f32 = 20.0;

/// The three fine systems whose control sources gate the destroyer's three
/// authored machines: thrust gates Engines, steering gates Steering, boost
/// gates Boost.
fn helm_actuator_system_ids() -> [crate::core::messages::SystemId; 3] {
    [
        crate::ship::system_registry::helm_thrust_system_id(),
        crate::ship::system_registry::helm_steering_system_id(),
        crate::ship::system_registry::helm_boost_system_id(),
    ]
}

/// Extend the fixture ship's hull with a per-system entry for each of the
/// three helm actuators, leaving every entry it already had untouched.
///
/// Every id is one the shipped destroyer already declares as a `[[system]]`;
/// what its TOML does not author is a `[[hull.system_hull]]` block for them
/// (it carries the scalar `hull_integrity` every NPC hull does). No shipped
/// hull combines the two today — the three that author helm machines are
/// Harrow NPCs, the four that author per-system hull are Alliance player
/// hulls — so putting authored doctrine on the damage path means composing
/// them here.
///
/// The composition stops at the hull entry, which is the one thing a
/// designer would add to the TOML. Tier derivation, `sync_console_damage_tiers`,
/// the offline set and the policy gate are all production code from there on.
fn model_the_helm_actuators_in_the_hull(app: &mut App) {
    let ship = find_ship_entity(app);
    let mut config: Vec<(crate::core::messages::SystemId, f32)> = app
        .world()
        .entity(ship)
        .get::<crate::entities::spawner::EntitySystemHull>()
        .expect("the fixture ship carries a system hull")
        .0
        .entries()
        .map(|(id, _current, max)| (id.clone(), max))
        .collect();
    config.extend(
        helm_actuator_system_ids()
            .into_iter()
            .map(|id| (id, HELM_ACTUATOR_MAX_HP)),
    );
    app.world_mut()
        .entity_mut(ship)
        .insert(crate::entities::spawner::EntitySystemHull(
            crate::ship::damage::SystemHull::from_config(&config),
        ));
}

/// Shoot `system_id` down into the `Disabled` tier — below the disabled
/// threshold but not to zero, because `Destroyed` is unrepairable and the
/// second half of the test repairs.
fn shoot_out(app: &mut App, system_id: &crate::core::messages::SystemId) {
    set_console_hp_direct(app, system_id.clone(), HELM_ACTUATOR_MAX_HP * 0.1);
}

/// Repair `system_id` back to full through the same `SystemHull::restore`
/// the repair-team tick calls.
fn repair_fully(app: &mut App, system_id: &crate::core::messages::SystemId) {
    let ship = find_ship_entity(app);
    let mut entity = app.world_mut().entity_mut(ship);
    let mut hull = entity
        .get_mut::<crate::entities::spawner::EntitySystemHull>()
        .expect("the fixture ship carries a system hull");
    hull.0.restore(system_id, HELM_ACTUATOR_MAX_HP);
}

/// Whether the damage sync has this system in `offline_systems`.
fn is_damage_offline(app: &mut App, system_id: &crate::core::messages::SystemId) -> bool {
    let ship = find_ship_entity(app);
    app.world()
        .entity(ship)
        .get::<ShipSystemControlSources>()
        .expect("the fixture ship carries control sources")
        .0
        .is_offline(system_id)
}

/// US15: a destroyer that is mid-escape when its helm actuators are shot out
/// comes back off the repair flying a FRESH pass, not resuming the one it was
/// halfway through.
///
/// The stale ACTION is what US15 is about, so the assertions are on the
/// admitted actuator surface rather than on a state string. The escape leg is
/// `hold_committed_heading` at the authored `escape_speed`: it ignores the
/// target entirely and flies a heading frozen at a merge that happened before
/// the damage. A destroyer that resumed it would come out of the repair
/// pointing at empty space and accelerating away from a target sitting on its
/// beam. A reset one re-acquires and hauls the bow onto it at the approach
/// throttle. Both throttles and both yaw modes differ, so the two are
/// distinguishable without reading a state name at all.
///
/// Everything up to the damage is FLOWN, not written: `run_to_escape` drives a
/// real approach, merge and break-off, so the non-initial state under test is
/// one the shipped machines actually reached.
#[test]
fn helm_actuators_shot_out_mid_escape_come_back_off_the_repair_on_a_fresh_pass() {
    let (mut app, uuid) = run_to_escape();
    model_the_helm_actuators_in_the_hull(&mut app);

    assert_eq!(
        steering_state(&mut app),
        "escape",
        "precondition: a leg the machine flew its way into"
    );
    assert_eq!(engines_state(&mut app), "escape");
    assert_eq!(boost_policy_state(&mut app).current, "escape");
    let frozen = pass_surface(&mut app).escape_heading_rad;
    let approach_speed = pass_surface(&mut app).approach_speed;
    let escape_speed = pass_surface(&mut app).escape_speed;
    assert!(
        (escape_speed - approach_speed).abs() > 1e-3,
        "precondition: the two legs must be flown at different authored \
         throttles ({escape_speed} vs {approach_speed}), or the throttle \
         assertion below cannot tell a fresh pass from a resumed escape"
    );

    // ── Shot out ─────────────────────────────────────────────────────────
    // Two ticks, and both are load-bearing: `sync_console_damage_tiers` runs
    // in `SimSet::Damage` and `ai_policy_state_tick` in `SimSet::Physics`, so
    // the offline flag written on the first tick is what the machine reads on
    // the second.
    for system_id in helm_actuator_system_ids() {
        shoot_out(&mut app, &system_id);
    }
    place_ship(&mut app, 0.0, -260.0, frozen, 20.0);
    tick_twice(&mut app);

    for system_id in helm_actuator_system_ids() {
        assert!(
            is_damage_offline(&mut app, &system_id),
            "hull damage must put `{}` into offline_systems — if it does not, \
             nothing below is exercising the damage path at all",
            system_id.0
        );
    }
    assert_eq!(
        steering_state(&mut app),
        "shadow",
        "a shot-out steering actuator must reset its machine to the authored \
         initial state (the class doctrine's defensive leg since issue #878), \
         not park it mid-escape"
    );
    assert_eq!(
        engines_state(&mut app),
        "shadow",
        "Engines runs its own copy of the machine and resets on its own gate"
    );
    assert_eq!(
        boost_policy_state(&mut app).current,
        "shadow",
        "and so does Boost"
    );
    assert!(
        !pass_surface(&mut app).active,
        "with the travel axes offline the planner is offered no pass at all"
    );

    // ── Repaired ─────────────────────────────────────────────────────────
    for system_id in helm_actuator_system_ids() {
        repair_fully(&mut app, &system_id);
    }
    // A target hard on the starboard beam, well inside the authored
    // `commit_range`, with the hull pinned exactly on the heading the dead
    // escape froze. Pinned rather than free-flown so the expected steering of
    // a resumed escape is exactly zero — an `abs() < tolerance` band would
    // wave through any quiet regression — and so the re-acquired run cannot
    // open enough range to break off again while the assertions run.
    set_bogey(&mut app, uuid, [200.0, 0.0, -260.0], 0.0, 0.0);
    for _ in 0..3 {
        place_ship(&mut app, 0.0, -260.0, frozen, 20.0);
        tick(&mut app);
    }

    for system_id in helm_actuator_system_ids() {
        assert!(
            !is_damage_offline(&mut app, &system_id),
            "the repair must take `{}` back out of offline_systems",
            system_id.0
        );
    }
    assert!(
        get_steering_input(&mut app) > 0.5,
        "the repaired destroyer must turn onto the bogey off its starboard \
         beam; a resumed escape flies the frozen heading and commands no \
         positive yaw at all, got steering {}",
        get_steering_input(&mut app)
    );
    assert!(
        (get_thrust_input(&mut app) - approach_speed).abs() < 1e-3,
        "and it must run in at the authored approach throttle ({approach_speed}), \
         not at the stale escape's {escape_speed}; got {}",
        get_thrust_input(&mut app)
    );
    assert!(
        !pass_surface(&mut app).escape,
        "the published leg must be a fresh run-in, not the escape the damage \
         interrupted"
    );
}

// ── The shield-recovery standoff orbit (issue #788) ──────────────────────
//
// Same posture as the fly-through tests above: the SHIPPED hull's authored
// policies driven through a real ticking app, asserting only on observable
// things — admitted actuator inputs, the ship's boost state, the committed
// policy state, the published pass surface.
//
// Two things are held constant across the long dwells below (the escape is
// an authored 7 seconds, which is 210 shared AI ticks): the ship's pose and
// its shield fraction. Both would otherwise drift — the hull keeps flying,
// and `tick_shields` keeps regenerating — and a test that let them drift
// would be measuring the drift rather than the doctrine.

/// How far the bogey below can shoot. Not the authored margin and not a
/// round number, so a safe ring computed as "reach + margin" is
/// distinguishable from one that quietly used only one of them.
const BOGEY_DIRECT_FIRE_REACH: f32 = 90.0;

/// The destroyer's authored `safe_range_margin`, restated so the expected
/// ring below is arithmetic on named values rather than a magic constant.
fn authored_steering_param(name: &str) -> f32 {
    let cfg = destroyer_hull();
    cfg.helm_console
        .as_ref()
        .and_then(|hc| hc.steering_ai.as_ref())
        .and_then(|ai| ai.param.get(name).copied())
        .unwrap_or_else(|| panic!("the shipped hull must author `{name}`"))
}

/// As [`authored_steering_param`], for the BOOST axis's own param table —
/// the machine that owns when the drive is lit, and so the one that authors
/// `boost_min_speed_fraction`.
fn authored_boost_param(name: &str) -> f32 {
    let cfg = destroyer_hull();
    cfg.helm_console
        .as_ref()
        .and_then(|hc| hc.boost_ai.as_ref())
        .and_then(|ai| ai.param.get(name).copied())
        .unwrap_or_else(|| panic!("the shipped hull's boost axis must author `{name}`"))
}

/// The hull's authored `max_speed`, so a test can turn an authored speed
/// FRACTION into the world-units speed `place_ship` takes.
fn authored_max_speed() -> f32 {
    destroyer_hull()
        .helm_console
        .as_ref()
        .expect("hull declares [helm_console]")
        .max_speed
}

/// A bogey that can shoot back — the reach a standoff ring is derived from.
fn set_armed_bogey(app: &mut App, uuid: uuid::Uuid, pos: [f32; 3], reach: f32) {
    app.insert_resource(crate::ai::server::WorldSnapshot {
        entities: vec![crate::ai::AiWorldEntity {
            uuid,
            name: Some(BOGEY.into()),
            position: pos,
            yaw: Some(0.0),
            forward_speed: 0.0,
            radius: 3.0,
            size_rating: 3.0,
            movable: true,
            dangerous: true,
            direct_fire_range: reach,
            ..Default::default()
        }],
    });
}

/// Force this ship's shields to `fraction` of capacity, online.
///
/// Written through the real `ShipShields` component rather than through a
/// fact, so the guard under test reads the same surface production reads.
fn set_shield_fraction(app: &mut App, fraction: f32) {
    let ship = find_ship_entity(app);
    let mut entity = app.world_mut().entity_mut(ship);
    let mut shields = entity
        .get_mut::<crate::ship::shields::ShipShields>()
        .expect("the test ship carries ShipShields");
    for facing in &mut shields.0.facings {
        facing.hp = (facing.max_hp as f32 * fraction).round() as i32;
        facing.offline_remaining = 0.0;
    }
}

fn shield_fraction(app: &mut App) -> f32 {
    app.world_mut()
        .query::<&crate::ship::shields::ShipShields>()
        .single(app.world())
        .expect("ship carries ShipShields")
        .0
        .fraction()
}

/// Advance the shared AI-policy clock by roughly `secs`, pinning the ship's
/// pose and shield fraction every tick.
///
/// `+3` covers the clock's own quantisation and the one-tick offset between
/// publishing the pass surface and the planner consuming it.
fn hold_and_tick(app: &mut App, secs: f32, pose: (f32, f32, f32, f32), shields: f32) {
    let ticks = (secs * 30.0).ceil() as usize + 3;
    for _ in 0..ticks {
        place_ship(app, pose.0, pose.1, pose.2, pose.3);
        set_shield_fraction(app, shields);
        tick(app);
    }
}

/// Where the destroyer sits out the escape dwell in [`run_to_recovery`],
/// with the bogey at `z = -200`: 120 units astern of it.
///
/// The distance is pinned between two lines, and the fixture is only a
/// RECOVERY fixture while it stays between them:
///
/// * beyond the bogey's own [`BOGEY_DIRECT_FIRE_REACH`] (90), so
///   `inside_threat_range` reads false and the escape counts as having
///   worked. Inside it — where this fixture sat before issue #789 — a
///   stationary destroyer is by definition an escape that gained nothing
///   under the enemy's guns, so the machine correctly takes the PRESSED
///   branch instead and every assertion below is about the wrong doctrine;
/// * inside the safe ring by more than `safe_ring_tolerance`
///   (`90 + 120 - 25 = 185`), so the distance history is full of breaches
///   when recovery begins and re-entry is not already satisfied.
const RECOVERY_DWELL_Z: f32 = -320.0;

/// Fly a complete pass against an ARMED bogey and sit out the authored
/// escape dwell with the shields collapsed, leaving the machine in
/// `recover` and the ship well inside the safe ring.
fn run_to_recovery() -> (App, uuid::Uuid) {
    run_to_recovery_omitting(&[])
}

/// Fly one complete merge against an ARMED bogey of the given `reach` and
/// stop the instant the escape commits.
///
/// The shared front half of every dwell fixture below: what separates a
/// recovery from a pressed run is only how the destroyer spends the escape
/// dwell that starts here, so sharing the approach keeps that the *only*
/// difference between them.
fn run_to_armed_escape(omit: &[&str], reach: f32) -> (App, uuid::Uuid) {
    let (mut app, uuid) = fly_through_app_omitting([0.0, 0.0, -200.0], omit);
    set_armed_bogey(&mut app, uuid, [0.0, 0.0, -200.0], reach);
    place_ship(&mut app, 0.0, 0.0, 0.0, 20.0);
    tick_twice(&mut app);
    assert_eq!(steering_state(&mut app), "inbound");
    place_ship(&mut app, 0.0, -195.0, 0.0, 20.0);
    tick(&mut app);
    place_ship(&mut app, 0.0, -260.0, 0.0, 20.0);
    tick(&mut app);
    assert_eq!(steering_state(&mut app), "escape");
    (app, uuid)
}

/// As [`run_to_recovery`], but flying a hull whose STEERING policy is
/// missing the named recovery `param`s.
fn run_to_recovery_omitting(omit: &[&str]) -> (App, uuid::Uuid) {
    let (mut app, uuid) = run_to_armed_escape(omit, BOGEY_DIRECT_FIRE_REACH);
    // Sit out the dwell with the shields gone, parked at `RECOVERY_DWELL_Z`
    // — see that constant for why the distance matters in two directions at
    // once.
    hold_and_tick(&mut app, 7.2, (0.0, RECOVERY_DWELL_Z, 0.0, 20.0), 0.0);
    (app, uuid)
}

/// The "decline rather than invent" gate covers all SIX recovery scalars,
/// not only the four the pass surface reads for itself.
///
/// `safe_ring_tolerance` and `safe_distance_window_ticks` are consumed by
/// `seed_recovery_facts` instead, and a hull that omits either can never
/// satisfy `fact(safe_distance_held)`: without the window the history keeps
/// its `Default` capacity of zero, so `is_full` — and therefore
/// `all_at_least` — is false for ever; without the tolerance the fold falls
/// through to its `_ => false` arm. The AC6 re-entry conjunct then cannot be
/// met and the hull flies the standoff ring indefinitely, which is a
/// strictly WORSE failure than the documented one. So either name missing,
/// on its own, must decline the whole arm.
///
/// The shipped hull reaches `recover` and publishes `recover = true` at this
/// exact point (asserted by the tests below), so nothing here passes for
/// want of getting that far.
#[test]
fn a_hull_omitting_either_history_scalar_declines_the_recovery_arm() {
    for omitted in [SAFE_RING_TOLERANCE_PARAM, SAFE_DISTANCE_WINDOW_TICKS_PARAM] {
        let (mut app, _uuid) = run_to_recovery_omitting(&[omitted]);

        // The MACHINE still enters the authored recovery state: the guard
        // that takes it there reads shields, not these scalars. What must
        // not happen is the HOST flying an orbit it can never fly out of.
        assert_eq!(
            steering_state(&mut app),
            "recover",
            "omitting `{omitted}` must not change which state is entered"
        );
        let pass = pass_surface(&mut app);
        assert!(
            !pass.recover,
            "omitting `{omitted}` must decline the recovery arm outright; \
             publishing `recover` without it orbits for ever, because \
             `safe_distance_held` can never be satisfied"
        );
        assert!(
            !pass.reengage,
            "the whole arm declines together, not half of it"
        );

        // And it stays declined. The shipped hull is holding its ring
        // through this dwell, so a run that keeps ticking must not quietly
        // start orbiting a few ticks later.
        hold_and_tick(&mut app, 3.0, (0.0, RECOVERY_DWELL_Z, 0.0, 20.0), 0.0);
        let pass = pass_surface(&mut app);
        assert!(
            !pass.recover && !pass.reengage,
            "omitting `{omitted}` must keep declining the arm, not orbit"
        );
    }
}

/// AC1's consequence at the doctrine level, and the anti-trap for an
/// unseeded fact: `fact(shield_fraction)` is genuinely read, so the escape
/// hands off to recovery ONLY when the pass actually cost the destroyer its
/// shields.
///
/// The negative control is the load-bearing half. A guard on a fact nobody
/// seeds parses fine and reads false for ever — which here would look like a
/// destroyer that simply never recovers, with nothing failing. The two runs
/// below differ in exactly one thing: the shield fraction.
#[test]
fn the_escape_hands_off_to_recovery_only_when_the_shields_collapsed() {
    let (mut app, _uuid) = run_to_recovery();
    assert_eq!(
        steering_state(&mut app),
        "recover",
        "shields at zero when the dwell expired: the destroyer must break off"
    );
    assert_eq!(
        engines_state(&mut app),
        "recover",
        "Engines runs its own copy of the machine and must reach the same leg \
         from the same facts, not by reading Steering's state"
    );

    // Negative control: identical run, healthy shields.
    let (mut app, uuid) = fly_through_app([0.0, 0.0, -200.0]);
    set_armed_bogey(&mut app, uuid, [0.0, 0.0, -200.0], BOGEY_DIRECT_FIRE_REACH);
    place_ship(&mut app, 0.0, 0.0, 0.0, 20.0);
    tick_twice(&mut app);
    place_ship(&mut app, 0.0, -195.0, 0.0, 20.0);
    tick(&mut app);
    place_ship(&mut app, 0.0, -260.0, 0.0, 20.0);
    tick(&mut app);
    assert_eq!(steering_state(&mut app), "escape");
    hold_and_tick(&mut app, 7.2, (0.0, -260.0, 0.0, 20.0), 1.0);
    // It re-acquires and — parked 60 units from the bogey, well inside the
    // authored `commit_range` — commits to the next run immediately. Which
    // of the two pass states it lands in is a detail; that it is back on the
    // pass cycle rather than orbiting is the point.
    assert!(
        ["acquire", "inbound"].contains(&steering_state(&mut app).as_str()),
        "with its shields intact the destroyer turns straight back in — if this \
         reads `recover` the shield guard is not reading the ship's shields; got {}",
        steering_state(&mut app)
    );
}

/// AC2: the safe ring is the TARGET's own longest usable direct-fire range
/// plus this hull's authored margin — not an authored distance, and not a
/// property of the destroyer at all.
///
/// Asserted by changing only the bogey's reach and watching the published
/// ring move with it, which no constant could do.
#[test]
fn the_safe_ring_derives_from_the_targets_direct_fire_reach_plus_the_margin() {
    let margin = authored_steering_param(SAFE_RANGE_MARGIN_PARAM);
    let (mut app, uuid) = run_to_recovery();

    let pass = pass_surface(&mut app);
    assert!(pass.recover, "the published leg must be the recovery orbit");
    assert!(
        (pass.safe_range - (BOGEY_DIRECT_FIRE_REACH + margin)).abs() < 1e-3,
        "the ring must be the target's reach ({BOGEY_DIRECT_FIRE_REACH}) plus the \
         authored margin ({margin}), got {}",
        pass.safe_range
    );

    // A longer-ranged opponent pushes the ring out by exactly the change in
    // ITS reach. Nothing about the destroyer changed.
    set_armed_bogey(&mut app, uuid, [0.0, 0.0, -200.0], 400.0);
    hold_and_tick(&mut app, 0.2, (0.0, -260.0, 0.0, 20.0), 0.0);
    assert!(
        (pass_surface(&mut app).safe_range - (400.0 + margin)).abs() < 1e-3,
        "the ring must follow the target's reach, got {}",
        pass_surface(&mut app).safe_range
    );

    // ...and an unarmed target collapses it to the margin alone, rather than
    // to an invented distance.
    set_armed_bogey(&mut app, uuid, [0.0, 0.0, -200.0], 0.0);
    hold_and_tick(&mut app, 0.2, (0.0, -260.0, 0.0, 20.0), 0.0);
    assert!((pass_surface(&mut app).safe_range - margin).abs() < 1e-3);
}

/// AC3 at the host: the recovery leg is flown at the authored ORBIT
/// throttle, under power and turning — not stopped, and not simply pointed
/// away.
///
/// The throttle assertion is what separates the orbit from the two
/// alternatives the issue rules out: a station-keeper would be braking
/// toward zero at the ring, and a retreat would be flying the escape
/// throttle straight down the outward bearing with no turn at all.
#[test]
fn the_recovery_leg_is_flown_as_a_powered_turning_orbit() {
    let (mut app, _uuid) = run_to_recovery();
    let orbit_speed = authored_steering_param(ORBIT_SPEED_PARAM);
    let pass = pass_surface(&mut app);
    assert!(pass.recover);

    assert!(
        (get_thrust_input(&mut app) - orbit_speed).abs() < 1e-3,
        "the ring is flown at the authored orbit throttle ({orbit_speed}), got {}",
        get_thrust_input(&mut app)
    );
    assert!(
        get_steering_input(&mut app).abs() > 0.0,
        "an orbit turns; a retreat does not"
    );
    assert!(
        pass.orbit_direction == 1.0 || pass.orbit_direction == -1.0,
        "the circulation direction must be a definite choice, got {}",
        pass.orbit_direction
    );
}

/// AC4: the circulation direction is drawn from a
/// (world, ship, system, transition, occurrence) key, so it reproduces
/// exactly for a given seed — and is not simply a constant.
#[test]
fn the_orbit_direction_is_deterministic_from_the_seed_without_being_constant() {
    fn direction_for(seed: u64, ship: uuid::Uuid) -> f32 {
        let (mut app, bogey) = fly_through_app([0.0, 0.0, -200.0]);
        app.insert_resource(crate::sim_rng::SimRng::new(
            seed,
            crate::sim_rng::SeedSource::Cli,
        ));
        let entity = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(entity)
            .insert(crate::entities::spawner::EntityUuid(ship.to_string()));
        set_armed_bogey(&mut app, bogey, [0.0, 0.0, -200.0], BOGEY_DIRECT_FIRE_REACH);
        place_ship(&mut app, 0.0, 0.0, 0.0, 20.0);
        tick_twice(&mut app);
        place_ship(&mut app, 0.0, -195.0, 0.0, 20.0);
        tick(&mut app);
        place_ship(&mut app, 0.0, -260.0, 0.0, 20.0);
        tick(&mut app);
        hold_and_tick(&mut app, 7.2, (0.0, RECOVERY_DWELL_Z, 0.0, 20.0), 0.0);
        assert_eq!(steering_state(&mut app), "recover");
        pass_surface(&mut app).orbit_direction
    }

    let ship = uuid::Uuid::from_u128(0x1234_5678_9abc_def0_1234_5678_9abc_def0);
    // Reproducible: same world seed, same ship, same answer. This is the
    // property a replayed `--seed` run depends on.
    assert_eq!(direction_for(4242, ship), direction_for(4242, ship));

    // Not a constant: over a handful of seeds both directions occur. A
    // hardcoded `+1` would pass every other assertion in this file.
    let directions: Vec<f32> = [1, 2, 3, 4, 5, 6, 7, 8]
        .into_iter()
        .map(|seed| direction_for(seed, ship))
        .collect();
    assert!(
        directions.contains(&1.0) && directions.contains(&-1.0),
        "the direction must genuinely vary with the seed, got {directions:?}"
    );
}

/// AC6: re-entry needs BOTH the authored shield fraction and a MAINTAINED
/// safe distance, and takes neither alone.
///
/// Three runs from the same starting point, differing only in which half is
/// satisfied. The "distance" half is what the bounded history window buys:
/// the ship is at range in the third run for long enough to fill it.
#[test]
fn re_entry_takes_neither_half_of_the_gate_alone() {
    let reentry_fraction = authored_steering_param("reentry_shield_fraction");
    let window_secs = authored_steering_param(SAFE_DISTANCE_WINDOW_TICKS_PARAM) / 30.0 + 0.5;

    // (a) Shields fully restored, but parked well INSIDE the ring.
    let (mut app, _uuid) = run_to_recovery();
    hold_and_tick(&mut app, window_secs, (0.0, -260.0, 0.0, 20.0), 1.0);
    assert!(
        shield_fraction(&mut app) >= reentry_fraction,
        "precondition: the shields really are back"
    );
    assert_eq!(
        steering_state(&mut app),
        "recover",
        "full shields inside the enemy's reach is not a recovery: the destroyer \
         must keep its ring"
    );

    // (b) Out beyond the ring for the whole window, but the shields are
    // still short of the authored fraction.
    let (mut app, _uuid) = run_to_recovery();
    hold_and_tick(&mut app, window_secs, (0.0, -700.0, 0.0, 20.0), 0.5);
    assert!(
        shield_fraction(&mut app) < reentry_fraction,
        "precondition: the shields are still short"
    );
    assert_eq!(
        steering_state(&mut app),
        "recover",
        "a maintained standoff with half its shields is not a recovery either"
    );

    // (c) Both: out beyond the ring for the whole window AND shields back.
    let (mut app, _uuid) = run_to_recovery();
    hold_and_tick(&mut app, window_secs, (0.0, -700.0, 0.0, 20.0), 1.0);
    assert_eq!(
        steering_state(&mut app),
        "reenter",
        "both halves satisfied: the destroyer turns back in"
    );
    assert_eq!(
        engines_state(&mut app),
        "reenter",
        "and every axis agrees, from the same facts"
    );
}

/// AC5/AC6, the "maintained" in maintained safe distance: ONE tick at range
/// does not open the gate. The window is authored and bounded, so the
/// destroyer must actually hold the ring for its full span.
#[test]
fn one_tick_at_range_is_not_a_maintained_safe_distance() {
    let (mut app, _uuid) = run_to_recovery();
    // Shields back, and a single tick out beyond the ring.
    hold_and_tick(&mut app, 0.05, (0.0, -700.0, 0.0, 20.0), 1.0);
    assert_eq!(
        steering_state(&mut app),
        "recover",
        "a couple of ticks at range is not a held distance — if this re-enters, \
         the history window is not being consulted"
    );
}

/// AC5: the history is BOUNDED. Ticking for many multiples of the authored
/// window must never let it grow past its capacity — the property that keeps
/// a scenario running for an hour from accumulating a per-ship buffer of
/// every range it ever saw.
#[test]
fn the_distance_history_never_grows_past_its_authored_bound() {
    let (mut app, _uuid) = run_to_recovery();
    let capacity = authored_steering_param(SAFE_DISTANCE_WINDOW_TICKS_PARAM) as usize;
    hold_and_tick(&mut app, 12.0, (0.0, -700.0, 0.0, 20.0), 0.0);
    let history = app
        .world_mut()
        .query::<&HelmRecoveryHistory>()
        .single(app.world())
        .expect("ship carries HelmRecoveryHistory")
        .clone();
    assert_eq!(
        history.ranges.capacity(),
        capacity,
        "the window's capacity is the authored value"
    );
    assert!(
        history.ranges.len() <= capacity,
        "the window must stay bounded: {} samples in a window of {capacity}",
        history.ranges.len()
    );
    assert!(history.ranges.is_full());
}

/// AC1/AC8, interrupted regeneration seen from the doctrine: shields that
/// are knocked back down mid-recovery keep the destroyer on its ring. The
/// gate is a level, not an edge, so a ship that briefly touched the
/// threshold and was then hit again does not get to re-enter.
#[test]
fn regeneration_interrupted_mid_recovery_keeps_the_destroyer_on_its_ring() {
    let window_secs = authored_steering_param(SAFE_DISTANCE_WINDOW_TICKS_PARAM) / 30.0 + 0.5;
    let (mut app, _uuid) = run_to_recovery();

    // Out at range with the shields climbing, but knocked back to zero
    // before they ever reach the authored fraction.
    hold_and_tick(&mut app, window_secs, (0.0, -700.0, 0.0, 20.0), 0.6);
    assert_eq!(steering_state(&mut app), "recover");
    hold_and_tick(&mut app, 0.5, (0.0, -700.0, 0.0, 20.0), 0.0);
    assert_eq!(
        steering_state(&mut app),
        "recover",
        "a shield ramp that was interrupted has not recovered"
    );

    // Let it actually finish this time.
    hold_and_tick(&mut app, 0.5, (0.0, -700.0, 0.0, 20.0), 1.0);
    assert_eq!(
        steering_state(&mut app),
        "reenter",
        "and once it does finish, with the distance still held, re-entry follows"
    );
}

/// AC7: normal re-entry cuts thrust, pivots onto the target WITHOUT boost,
/// and then begins another normal-speed pass.
#[test]
fn re_entry_cuts_thrust_pivots_cold_and_starts_another_normal_speed_pass() {
    let window_secs = authored_steering_param(SAFE_DISTANCE_WINDOW_TICKS_PARAM) / 30.0 + 0.5;
    let (mut app, uuid) = run_to_recovery();
    hold_and_tick(&mut app, window_secs, (0.0, -700.0, 0.0, 20.0), 1.0);
    assert_eq!(steering_state(&mut app), "reenter");

    // Put the bogey hard off the beam so a real pivot is demanded and a
    // "hold the last steering command" fallback would be visible as zero.
    set_armed_bogey(
        &mut app,
        uuid,
        [500.0, 0.0, -700.0],
        BOGEY_DIRECT_FIRE_REACH,
    );
    place_ship(&mut app, 0.0, -700.0, 0.0, 20.0);
    set_shield_fraction(&mut app, 1.0);
    tick(&mut app);
    tick(&mut app);

    assert!(
        pass_surface(&mut app).reengage,
        "the published leg must be the re-entry pivot"
    );
    assert_eq!(
        get_thrust_input(&mut app),
        0.0,
        "the pivot cuts thrust — that is what the authored reengage_speed of 0 means"
    );
    assert!(
        get_steering_input(&mut app) > 0.0,
        "and it turns onto the target off the starboard beam, got {}",
        get_steering_input(&mut app)
    );
    assert!(
        !boost_is_active(&mut app),
        "the pivot is flown COLD: no recovery state authors a boost rule"
    );

    // ...and the pivot's authored dwell hands off to an ordinary
    // normal-speed pass, not to another escape.
    let pivot_secs = authored_steering_param("reenter_pivot_secs");
    for _ in 0..((pivot_secs * 30.0).ceil() as usize + 3) {
        place_ship(&mut app, 0.0, -700.0, 0.0, 20.0);
        set_shield_fraction(&mut app, 1.0);
        tick(&mut app);
    }
    assert_eq!(
        steering_state(&mut app),
        "acquire",
        "the pivot ends in the ordinary approach state, so the next run is a \
         normal-speed pass"
    );
    assert!(
        !boost_is_active(&mut app),
        "an approach never boosts: acquire authors no boost rule"
    );
}

// ── The pressed short-pass fallback (issue #789) ─────────────────────────
//
// Same posture again: the SHIPPED hull's authored policies driven through a
// real ticking app, asserting only on the committed policy state, the
// published pass surface, the admitted actuator inputs, and the ship's boost
// state.
//
// The whole section turns on ONE distinction the fixtures have to keep
// honest, so it is worth stating plainly. A destroyer whose escape WORKED
// recovers; one whose escape FAILED presses. "Failed" is two things at once
// — it gained no ground AND it is still inside the guns — so every negative
// control below changes exactly one of those and holds the other, plus the
// shields, plus the dwell, constant.

/// Where the destroyer sits out the escape dwell to read as PRESSED: 60
/// units astern of the bogey, i.e. well INSIDE its
/// [`BOGEY_DIRECT_FIRE_REACH`] of 90, and stationary — an escape that ran
/// its full authored dwell and finished no further from the guns than it
/// started.
///
/// The mirror of [`RECOVERY_DWELL_Z`], and the two differ in one thing only.
const PRESSED_DWELL_Z: f32 = -260.0;

/// A bogey that outranges the whole test arena.
///
/// Exists so a control can open real distance and STILL be inside the
/// target's reach when the dwell ends. Against the ordinary 90-unit bogey
/// those two are physically inseparable — gaining ground from inside 90
/// units takes you outside them — so isolating the progress conjunct on its
/// own needs guns long enough that leaving them is not the same event as
/// getting away.
const LONG_REACH: f32 = 400.0;

/// As [`run_to_pressed`], but flying a hull whose STEERING policy is missing
/// the named `param`s.
fn run_to_pressed_omitting(omit: &[&str]) -> (App, uuid::Uuid) {
    let (mut app, uuid) = run_to_armed_escape(omit, BOGEY_DIRECT_FIRE_REACH);
    hold_and_tick(&mut app, 7.2, (0.0, PRESSED_DWELL_Z, 0.0, 20.0), 0.0);
    (app, uuid)
}

/// Fly a complete pass against an armed bogey and sit out the escape dwell
/// pinned inside its reach with the shields gone, leaving the machine in
/// `pressed_pivot`.
fn run_to_pressed() -> (App, uuid::Uuid) {
    run_to_pressed_omitting(&[])
}

/// Sit out `secs` of shared AI ticks while the ship OPENS the range from a
/// bogey at the origin end by `per_tick` world units every tick, holding its
/// shields at `shields`.
///
/// The counterpart of [`hold_and_tick`]: that one pins a pose so a test is
/// not measuring drift, this one moves it deliberately so a test can measure
/// a TREND. Both pin the shields for the same reason.
fn withdraw_and_tick(app: &mut App, secs: f32, start_z: f32, per_tick: f32, shields: f32) {
    let ticks = (secs * 30.0).ceil() as usize + 3;
    for i in 0..ticks {
        place_ship(app, 0.0, start_z - per_tick * i as f32, 0.0, 20.0);
        set_shield_fraction(app, shields);
        tick(app);
    }
}

/// This ship's planar distance from `pos`, so a control can assert its own
/// geometric precondition instead of asserting it in a comment.
fn range_from(app: &mut App, pos: [f32; 3]) -> f32 {
    let physics = *app
        .world_mut()
        .query_filtered::<&ShipPhysics, With<Ship>>()
        .single(app.world())
        .expect("ship carries ShipPhysics");
    let (dx, dz) = (physics.x - pos[0], physics.z - pos[2]);
    (dx * dx + dz * dz).sqrt()
}

/// AC1, and the anti-trap for the two new facts: pressed detection is a
/// comparison of authored minimum separation PROGRESS across an authored
/// history window, taken only while inside the target's own effective threat
/// range.
///
/// Both conjuncts get their own matched control, because either one alone
/// would be a fact nobody seeded reading false for ever — which would look
/// exactly like a destroyer that simply never presses, with nothing failing.
///
/// (a) and (b) differ in ONE thing: whether the destroyer moved. Same bogey,
/// same reach, same shield collapse, same dwell, and (b) ends its dwell
/// STILL inside those guns — asserted, not assumed — so the threat-range
/// conjunct is identical in both and only the progress reading differs.
/// (c) then holds the progress at zero and moves the threat line instead.
#[test]
fn only_an_escape_that_gains_no_ground_under_the_guns_presses_the_destroyer() {
    // (a) Pinned inside the reach for the whole dwell: the escape failed.
    let (mut app, _uuid) = run_to_armed_escape(&[], LONG_REACH);
    hold_and_tick(&mut app, 7.2, (0.0, PRESSED_DWELL_Z, 0.0, 20.0), 0.0);
    assert_eq!(
        steering_state(&mut app),
        "pressed_pivot",
        "an escape that ran its full dwell and gained nothing, inside the target's \
         own reach, must abandon recovery"
    );
    assert_eq!(
        engines_state(&mut app),
        "pressed_pivot",
        "Engines runs its own copy of the machine and must reach the same leg from \
         the same facts, not by reading Steering's state"
    );

    // (b) The one-variable control: the same run, opening ground steadily
    // across the window the detector measures, and still under those guns
    // when the dwell expires.
    //
    // The per-tick step is DERIVED from the two authored scalars it has to
    // beat rather than hand-picked against today's values: enough ground per
    // tick that a full `pressed_window_ticks` window nets twice
    // `pressed_min_progress`. Retuning either param moves this with it
    // instead of quietly turning the control into a second copy of (a).
    let per_tick = 2.0 * authored_steering_param(PRESSED_MIN_PROGRESS_PARAM)
        / authored_steering_param(PRESSED_WINDOW_TICKS_PARAM);
    let (mut app, _uuid) = run_to_armed_escape(&[], LONG_REACH);
    withdraw_and_tick(&mut app, 7.2, -205.0, per_tick, 0.0);
    assert!(
        range_from(&mut app, [0.0, 0.0, -200.0]) < LONG_REACH,
        "precondition: this control must still be INSIDE the target's reach, or it \
         is testing the threat conjunct instead of the progress one — got {}",
        range_from(&mut app, [0.0, 0.0, -200.0])
    );
    assert_eq!(
        steering_state(&mut app),
        "recover",
        "an escape that kept opening real distance WORKED: the destroyer takes the \
         ordinary recovery doctrine even though it is still in range"
    );

    // (c) The other conjunct, alone: standing still again, but out beyond a
    // reach that can no longer touch it. Distance that does not matter is
    // not distance worth measuring.
    let (mut app, _uuid) = run_to_recovery();
    assert!(
        range_from(&mut app, [0.0, 0.0, -200.0]) > BOGEY_DIRECT_FIRE_REACH,
        "precondition: this control must be OUTSIDE the target's reach"
    );
    assert_eq!(
        steering_state(&mut app),
        "recover",
        "a destroyer sitting still beyond the enemy's guns is not pinned — if this \
         reads `pressed_pivot`, the threat-range conjunct is not being read"
    );
}

/// AC2: taking a hit — even one that collapses the shields outright — does
/// not by itself press the destroyer.
///
/// Structurally it cannot: there is no hit or damage EVENT fact anywhere in
/// this codebase, only `shield_fraction` as a level, so a detector built on
/// separation alone has nothing to fire on. This is the explicit negative
/// control for that, in the shape the recovery hand-off test uses — one run,
/// one fact moved, and the damage arriving as a discrete event partway
/// through an escape that is otherwise going perfectly well.
///
/// The state is sampled on EVERY tick rather than only at the end, because
/// "never pressed" is the claim; a run that dipped into the pressed loop and
/// came back out would satisfy an end-state assertion and still be wrong.
#[test]
fn a_shield_collapse_alone_never_presses_the_destroyer() {
    let (mut app, _uuid) = run_to_armed_escape(&[], LONG_REACH);

    // The first half of the escape goes perfectly: shields up, ground being
    // opened steadily.
    let mut visited: Vec<String> = Vec::new();
    let mut z = -205.0_f32;
    for tick_index in 0..222 {
        // THE HIT, once, at a single instant: full shields to none.
        let shields = if tick_index < 90 { 1.0 } else { 0.0 };
        place_ship(&mut app, 0.0, z, 0.0, 20.0);
        set_shield_fraction(&mut app, shields);
        tick(&mut app);
        z -= 1.5;
        visited.push(steering_state(&mut app));
    }

    assert!(
        !visited.iter().any(|s| s.starts_with("pressed")),
        "a destroyer that is opening ground the whole time must never press, \
         whatever its shields do; it visited {:?}",
        visited.iter().collect::<std::collections::BTreeSet<_>>()
    );
    assert_eq!(
        steering_state(&mut app),
        "recover",
        "the collapse still costs it the pass — it breaks off to the ordinary \
         standoff — but that is the shield gate doing its job, not the pressed one"
    );
}

/// AC4: the pressed pivot is a STATIONARY turn flown with the drive lit, and
/// the drive is cancelled before the normal-speed pass that follows.
///
/// The cancel is the load-bearing half and it is authored as an absence —
/// `pressed_pass` declares no boost rule at all, so the channel holds and
/// `ai_helm_boost`'s on-change release fires. An absence is exactly the kind
/// of content that gets helpfully filled in, so it is asserted here through
/// the ship's real boost state as well as pinned in the hull's parse tests.
#[test]
fn the_pressed_pivot_boosts_a_stationary_turn_and_the_short_pass_does_not() {
    let (mut app, _uuid) = run_to_pressed();
    assert_eq!(steering_state(&mut app), "pressed_pivot");

    let pass = pass_surface(&mut app);
    assert!(
        pass.reengage,
        "the pivot is published as the re-engage leg, so the host pairs it with the \
         authored reengage_speed"
    );
    assert!(!pass.recover, "and emphatically not as a standoff orbit");
    assert_eq!(
        get_thrust_input(&mut app),
        authored_steering_param(REENGAGE_SPEED_PARAM),
        "a STATIONARY pivot: the throttle is the authored re-engage fraction"
    );
    assert!(
        boost_is_active(&mut app),
        "the pivot is flown with the drive lit — that is what buys the extra yaw \
         rate, and it is the one place outside the escape that boosts"
    );

    // The authored pivot dwell expires into the short pass...
    let pivot_secs = authored_steering_param(PRESSED_PIVOT_SECS_PARAM);
    hold_and_tick(
        &mut app,
        pivot_secs + 0.2,
        (0.0, PRESSED_DWELL_Z, 0.0, 20.0),
        0.0,
    );
    assert_eq!(
        steering_state(&mut app),
        "pressed_pass",
        "the pivot ends on its own authored dwell"
    );
    assert!(
        !boost_is_active(&mut app),
        "...and the drive is CANCELLED for it: the short pass is a normal-speed \
         pass, and it is recharging for the escape at the end of it"
    );
}

/// AC5/AC7: the short pass runs in at NORMAL speed, tracks the target, and
/// breaks off into a straight boost-out escape on its own shorter authored
/// hysteresis.
///
/// The break-off is the half that makes the pass *short*, and it is asserted
/// against a matched control rather than against a number: the same
/// re-opening — chosen to sit between the two authored hysteresis values —
/// commits the pressed pass and does NOT commit an ordinary inbound leg.
#[test]
fn the_short_pass_runs_in_at_normal_speed_and_breaks_off_sooner() {
    let pressed_hysteresis = authored_steering_param(PRESSED_HYSTERESIS_PARAM);
    let ordinary_hysteresis = authored_steering_param("closest_approach_hysteresis");
    // Between the two, so it can only be read one way.
    let reopen_by = (pressed_hysteresis + ordinary_hysteresis) / 2.0;
    assert!(
        pressed_hysteresis < reopen_by && reopen_by < ordinary_hysteresis,
        "the hull must author a SHORTER pressed hysteresis for this pair to mean \
         anything: {pressed_hysteresis} vs {ordinary_hysteresis}"
    );

    let (mut app, uuid) = run_to_pressed();
    let pivot_secs = authored_steering_param(PRESSED_PIVOT_SECS_PARAM);
    hold_and_tick(
        &mut app,
        pivot_secs + 0.2,
        (0.0, PRESSED_DWELL_Z, 0.0, 20.0),
        0.0,
    );
    assert_eq!(steering_state(&mut app), "pressed_pass");

    // ── The motion ──────────────────────────────────────────────────────
    // Abeam the bogey, so a real turn onto it is demanded and a "hold the
    // last steering command" fallback would show up as zero. The range is
    // unchanged, so nothing here can trip the break-off below.
    let pass = pass_surface(&mut app);
    place_ship(&mut app, 60.0, -200.0, 0.0, 20.0);
    tick(&mut app);
    tick(&mut app);
    assert!(
        pass.active && !pass.escape && !pass.recover && !pass.reengage,
        "the short pass is published as an ordinary inbound leg — that is what \
         makes it a normal-speed attack pass and not a fifth kind of manoeuvre"
    );
    assert!(
        (get_thrust_input(&mut app) - pass.approach_speed).abs() < 1e-3,
        "the short pass runs in at the authored APPROACH throttle ({}), not the \
         escape throttle, got {}",
        pass.approach_speed,
        get_thrust_input(&mut app)
    );
    assert!(
        get_steering_input(&mut app) < 0.0,
        "and it tracks the target, which is off the port beam, got {}",
        get_steering_input(&mut app)
    );
    assert!(
        !boost_is_active(&mut app),
        "still cold: a pass is not an escape"
    );

    // ── The break-off ───────────────────────────────────────────────────
    // Back astern of the bogey and driving away from it, so the closing rate
    // is negative, then let the range re-open by `reopen_by`.
    place_ship(&mut app, 0.0, PRESSED_DWELL_Z, 0.0, 20.0);
    tick(&mut app);
    assert_eq!(
        steering_state(&mut app),
        "pressed_pass",
        "an opening rate alone is not a closest approach on either doctrine"
    );
    place_ship(&mut app, 0.0, PRESSED_DWELL_Z - reopen_by, 0.0, 20.0);
    tick(&mut app);
    assert_eq!(
        steering_state(&mut app),
        "escape",
        "{reopen_by} units of re-opening is past the SHORT pass's authored \
         hysteresis of {pressed_hysteresis}: the jab is over and the destroyer \
         commits to another boost-out"
    );
    assert!(
        pass_surface(&mut app).escape,
        "and the published leg is the escape, flown from a heading frozen at this \
         merge"
    );

    // ── The boost-out itself ────────────────────────────────────────────
    // "Commits to another boost-out" is a claim about the DRIVE, not only
    // about the state, so it is asserted through the ship's real boost
    // state — and asserted as the authored behaviour rather than as
    // "instantly". The escape's own rule carries
    // `fact(speed_fraction) >= param(boost_min_speed_fraction)`, so a jab
    // that broke off before the hull had rebuilt speed relights LATE, not
    // never. Both sides of that authored line are checked, because only the
    // pair distinguishes "waiting for speed" from "never lights at all".
    let min_fraction = authored_boost_param("boost_min_speed_fraction");
    let min_speed = min_fraction * authored_max_speed();
    place_ship(
        &mut app,
        0.0,
        PRESSED_DWELL_Z - reopen_by,
        0.0,
        min_speed * 0.5,
    );
    tick(&mut app);
    assert!(
        !boost_is_active(&mut app),
        "under the authored {min_fraction} speed fraction the escape holds the \
         drive: boost is an escape accelerant, not a launch assist"
    );
    place_ship(
        &mut app,
        0.0,
        PRESSED_DWELL_Z - reopen_by,
        0.0,
        min_speed + 1.0,
    );
    tick(&mut app);
    assert!(
        boost_is_active(&mut app),
        "once past the authored fraction the escape out of a pressed pass lights \
         the drive like any other escape — the jab ends in a real attempt to leave"
    );
    assert_eq!(
        steering_state(&mut app),
        "escape",
        "and nothing about the relight ends the escape leg"
    );

    // The matched control: the identical re-opening on an ORDINARY inbound
    // leg is short of ITS authored hysteresis and commits nothing.
    let (mut app, _uuid) = fly_through_app([0.0, 0.0, -200.0]);
    set_armed_bogey(&mut app, uuid, [0.0, 0.0, -200.0], BOGEY_DIRECT_FIRE_REACH);
    place_ship(&mut app, 0.0, PRESSED_DWELL_Z, 0.0, 20.0);
    tick_twice(&mut app);
    assert_eq!(steering_state(&mut app), "inbound");
    place_ship(&mut app, 0.0, PRESSED_DWELL_Z - reopen_by, 0.0, 20.0);
    tick(&mut app);
    assert_eq!(
        steering_state(&mut app),
        "inbound",
        "the same {reopen_by} units is short of the ordinary {ordinary_hysteresis}-unit \
         hysteresis — if this commits too, the pressed pass is not actually shorter"
    );
}

/// AC3: while pressed, the destroyer waits for neither of the things the
/// recovery doctrine waits for.
///
/// Both of recovery's re-entry conjuncts are handed to it mid-loop —
/// shields fully restored — and it neither switches to the standoff ring nor
/// jumps to the re-entry pivot. It finishes its own authored pivot dwell and
/// makes its pass, because being pinned is not a thing that gets better by
/// waiting.
#[test]
fn pressed_behaviour_waits_on_neither_the_shield_threshold_nor_the_ring() {
    let (mut app, _uuid) = run_to_pressed();
    let reentry_fraction = authored_steering_param("reentry_shield_fraction");

    // Hand it the shield half of the recovery gate outright.
    let mut visited: Vec<String> = Vec::new();
    let pivot_secs = authored_steering_param(PRESSED_PIVOT_SECS_PARAM);
    for _ in 0..((pivot_secs * 30.0).ceil() as usize + 3) {
        place_ship(&mut app, 0.0, PRESSED_DWELL_Z, 0.0, 20.0);
        set_shield_fraction(&mut app, 1.0);
        tick(&mut app);
        visited.push(steering_state(&mut app));
        assert!(
            !pass_surface(&mut app).recover,
            "a pressed destroyer never publishes the standoff orbit"
        );
    }
    assert!(
        shield_fraction(&mut app) >= reentry_fraction,
        "precondition: the shields really are back past the re-entry threshold"
    );
    assert!(
        !visited.iter().any(|s| s == "recover" || s == "reenter"),
        "restoring the shields must not pull a pressed destroyer into the recovery \
         doctrine mid-loop; it visited {visited:?}"
    );
    assert_eq!(
        steering_state(&mut app),
        "pressed_pass",
        "it finishes the pivot it started and makes its pass"
    );
}

/// AC6: the pressed loop is a response, not a mode. The moment one of its
/// escapes actually opens ground, the destroyer is back on the ordinary
/// recovery doctrine.
///
/// This is the round trip end to end — recovery abandoned, pivot, short
/// pass, escape, recovery resumed — so it also pins that the pressed states
/// hand back to the SAME `escape` state the ordinary pass uses rather than
/// to a private copy of it.
#[test]
fn a_successful_escape_out_of_the_pressed_loop_resumes_the_recovery_doctrine() {
    let reopen_by = authored_steering_param(PRESSED_HYSTERESIS_PARAM) + 1.0;
    let (mut app, _uuid) = run_to_pressed();
    assert_eq!(steering_state(&mut app), "pressed_pivot");

    // Pivot → short pass → break-off into another escape attempt.
    let pivot_secs = authored_steering_param(PRESSED_PIVOT_SECS_PARAM);
    hold_and_tick(
        &mut app,
        pivot_secs + 0.2,
        (0.0, PRESSED_DWELL_Z, 0.0, 20.0),
        0.0,
    );
    assert_eq!(steering_state(&mut app), "pressed_pass");
    place_ship(&mut app, 0.0, PRESSED_DWELL_Z - reopen_by, 0.0, 20.0);
    tick(&mut app);
    assert_eq!(steering_state(&mut app), "escape");

    // THIS escape works: it ends the dwell out beyond the target's reach,
    // with the shields still gone.
    hold_and_tick(&mut app, 7.2, (0.0, RECOVERY_DWELL_Z, 0.0, 20.0), 0.0);
    assert_eq!(
        steering_state(&mut app),
        "recover",
        "an escape that succeeded hands off to the ordinary standoff orbit, however \
         many failed ones came before it"
    );
    assert_eq!(
        engines_state(&mut app),
        "recover",
        "and every axis comes back together, from the same facts"
    );
    assert!(
        pass_surface(&mut app).recover,
        "the published leg is the orbit again"
    );
}

/// "Decline rather than invent", on all four pressed scalars.
///
/// Each is genuinely load-bearing on its own and each fails differently if
/// admitted alone: without `pressed_window_ticks` the progress window keeps
/// its `Default` capacity of zero and can never report a trend; without
/// `pressed_min_progress` there is no line to compare that trend against;
/// without `pressed_pivot_secs` the pivot never ends; without
/// `pressed_closest_approach_hysteresis` the short pass never breaks off. A
/// hull admitted into the arm on three of the four would stall inside it —
/// strictly worse than never entering — so the host gates on all four
/// together and the hull flies its ordinary recovery doctrine instead.
///
/// The shipped hull presses at this exact point (asserted above), so nothing
/// here passes for want of getting that far.
#[test]
fn a_hull_omitting_any_pressed_scalar_declines_the_whole_pressed_arm() {
    for omitted in PRESSED_PARAMS {
        let (mut app, _uuid) = run_to_pressed_omitting(&[omitted]);
        assert_eq!(
            steering_state(&mut app),
            "recover",
            "omitting `{omitted}` must decline the pressed arm outright and leave the \
             hull on the ordinary recovery doctrine"
        );
        assert_eq!(
            engines_state(&mut app),
            "recover",
            "omitting `{omitted}` declines it on EVERY axis — the host's gate is over \
             the one shared fact snapshot, so the three machines cannot disagree"
        );

        // And it stays declined: a run that keeps ticking must not start
        // pressing a few ticks later.
        hold_and_tick(&mut app, 3.0, (0.0, PRESSED_DWELL_Z, 0.0, 20.0), 0.0);
        assert!(
            !steering_state(&mut app).starts_with("pressed"),
            "omitting `{omitted}` must keep declining the arm"
        );
    }
}

/// ...and on all six RECOVERY scalars too, which is the same trap one level
/// up.
///
/// The pressed pivot is not a fifth kind of manoeuvre: it is flown as
/// `FlyThroughLeg::Reengage`, and the planner only flies that leg when
/// `HelmPassSurface::reengage` is published, which `build_pass_surface` only
/// does when the whole recovery six are authored. A hull admitted into the
/// pressed arm on the four pressed scalars alone would therefore enter
/// `pressed_pivot` and have the planner fall through to the INBOUND leg —
/// boosted, at full approach throttle, turning hard, straight at the ship
/// that has it pinned. That is strictly worse than the doctrine travel such
/// a hull flew before the pressed arm existed, so the arm declines outright.
///
/// Nothing in content validation ties the `pivot_to_reengage` verb to those
/// scalars, so the host's gate is the only thing that can catch it — and
/// this is the test that holds the gate in place.
#[test]
fn a_hull_omitting_any_recovery_scalar_declines_the_pressed_arm_too() {
    for omitted in RECOVERY_PARAMS {
        let (mut app, _uuid) = run_to_pressed_omitting(&[omitted]);
        assert_eq!(
            steering_state(&mut app),
            "recover",
            "omitting `{omitted}` must decline the pressed arm outright — a hull that \
             cannot publish the re-engage leg cannot fly the pressed pivot"
        );
        assert_eq!(
            engines_state(&mut app),
            "recover",
            "omitting `{omitted}` declines it on EVERY axis, from the one shared fact \
             snapshot"
        );

        // And it stays declined, for the same reason the pressed-scalar
        // case does: a run that keeps ticking must not start pressing a few
        // ticks later.
        hold_and_tick(&mut app, 3.0, (0.0, PRESSED_DWELL_Z, 0.0, 20.0), 0.0);
        assert!(
            !steering_state(&mut app).starts_with("pressed"),
            "omitting `{omitted}` must keep declining the arm"
        );
    }
}

// ── The Harrow Cruiser broadside orbit (issue #790) ──────────────────────
//
// Same posture as the destroyer block above: these drive the SHIPPED hull's
// authored policies through a real ticking app, so they fail on the content
// as well as on the code, and every assertion is about something observable
// — an admitted actuator input, the published pass surface, the committed
// policy state, or the ship's own flown range.
//
// Unlike the destroyer's tests, the orbit ones deliberately let the ship
// FLY rather than pinning its pose each tick. A spiral is a claim about how
// the range changes over time, and pinning the pose would make that claim
// untestable.

fn cruiser_hull() -> crate::entities::config::EntityConfig {
    crate::entities::config::EntityConfig::from_toml(
        crate::entities::include_resolve::resolve_from_disk(
            "assets/entities/ship_harrow_cruiser.toml",
        )
        .expect("ship_harrow_cruiser must resolve")
        .toml
        .as_str(),
    )
    .expect("the shipped cruiser hull must parse")
}

/// The cruiser's authored Steering `param`s, so expectations below are
/// arithmetic on named values rather than magic numbers.
fn cruiser_steering_param(name: &str) -> f32 {
    cruiser_hull()
        .helm_console
        .as_ref()
        .and_then(|hc| hc.steering_ai.as_ref())
        .and_then(|ai| ai.param.get(name).copied())
        .unwrap_or_else(|| panic!("the shipped cruiser must author `{name}`"))
}

/// A ship carrying the shipped cruiser's two authored policy machines and
/// its physics envelope — the same components `entities::spawner` would
/// attach — hunting a single named bogey.
///
/// The cruiser authors no boost drive and no boost doctrine, so nothing
/// boost-shaped is inserted here either: the fixture mirrors the hull.
fn broadside_app(bogey_pos: [f32; 3]) -> (App, uuid::Uuid) {
    broadside_app_omitting(bogey_pos, &[])
}

/// As [`broadside_app`], but with the named STEERING `param`s stripped from
/// the hull before its policy is built — the partially-authored hull
/// AGENTS.md #11 says must decline rather than invent.
///
/// Each name must actually be present to begin with, so this cannot quietly
/// pass by "removing" a param the hull renamed out from under it.
fn broadside_app_omitting(bogey_pos: [f32; 3], omit: &[&str]) -> (App, uuid::Uuid) {
    let mut app = test_app();
    let cfg = cruiser_hull();
    let mut hc = cfg
        .helm_console
        .clone()
        .expect("hull declares [helm_console]");
    for name in omit {
        hc.steering_ai
            .as_mut()
            .expect("hull declares [helm_console.steering_ai]")
            .param
            .remove(*name)
            .unwrap_or_else(|| panic!("the shipped hull must author `{name}` to omit it"));
    }
    let ship = find_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(ship)
        .insert((crate::ship_plugin::ShipPhysicsConfigResource(
            crate::ship::physics::ShipPhysicsConfig {
                max_speed: hc.max_speed,
                max_reverse_speed: hc.max_reverse_speed,
                acceleration: hc.acceleration,
                deceleration: hc.deceleration,
                max_yaw_rate: hc.max_yaw_rate,
                ..crate::ship::physics::ShipPhysicsConfig::new()
            },
        ),));
    // Override just Engines/Steering in the ship's keyed `FineSystemAiPolicies`
    // map, MERGING into the shipped defaults `test_app` attached (issue #1209).
    for (system_id, policy) in [
        (
            crate::ship::system_registry::helm_thrust_system_id(),
            hc.engines_ai.as_ref().unwrap().to_policy().unwrap(),
        ),
        (
            crate::ship::system_registry::helm_steering_system_id(),
            hc.steering_ai.as_ref().unwrap().to_policy().unwrap(),
        ),
    ] {
        set_fine_policy(&mut app, ship, system_id, policy);
    }
    set_ship_blackboard_objectives(&mut app, vec![destroy_scored_objective(BOGEY, 80.0)]);
    set_helm_control_source(&mut app, ControlSource::Ai);
    let uuid = uuid::Uuid::new_v4();
    set_bogey(&mut app, uuid, bogey_pos, 0.0, 0.0);
    leave_the_defensive_leg(&mut app, "acquire");
    (app, uuid)
}

/// The bogey sits at the origin for every orbit fixture, so a "range to the
/// target" reading is just the ship's own distance from the origin.
const ORBIT_BOGEY: [f32; 3] = [0.0, 0.0, 0.0];

fn ship_pose(app: &mut App) -> ShipPhysics {
    *app.world_mut()
        .query_filtered::<&ShipPhysics, With<Ship>>()
        .single(app.world())
        .expect("ship carries ShipPhysics")
}

/// Planar distance from the ship to the bogey at [`ORBIT_BOGEY`].
fn range_to_bogey(app: &mut App) -> f32 {
    let p = ship_pose(app);
    (p.x * p.x + p.z * p.z).sqrt()
}

/// Shortest signed angular step from `previous` to `now`, radians. Summing
/// these is what turns a wrapping bearing into a swept angle.
fn wrapped_delta(now: f32, previous: f32) -> f32 {
    let mut d = now - previous;
    while d > std::f32::consts::PI {
        d -= std::f32::consts::TAU;
    }
    while d < -std::f32::consts::PI {
        d += std::f32::consts::TAU;
    }
    d
}

/// The ship's bearing around the bogey, radians. Its RATE of change is the
/// circulation: a positive drift is one way round the ring, a negative one
/// the other, and that is what "clockwise or anticlockwise" means
/// observably.
fn bearing_around_bogey(app: &mut App) -> f32 {
    let p = ship_pose(app);
    simmath::atan2(p.x, p.z)
}

/// Put the cruiser into its orbit state, flying, at `start_range` from the
/// bogey.
///
/// The ship starts abeam of its own heading (placed down `-Z`, facing `+X`)
/// so it begins with a tangential component rather than head-on, which is
/// the pose the approach leg would have delivered it in anyway.
fn run_to_orbit_omitting(start_range: f32, omit: &[&str]) -> (App, uuid::Uuid) {
    let (mut app, uuid) = broadside_app_omitting(ORBIT_BOGEY, omit);
    let speed = cruiser_hull().helm_console.as_ref().unwrap().max_speed
        * cruiser_steering_param(COMBAT_ORBIT_SPEED_PARAM);
    place_ship(
        &mut app,
        0.0,
        -start_range,
        std::f32::consts::FRAC_PI_2,
        speed,
    );
    // Two ticks: the first publishes the pass surface, the second is the
    // first planner pass that consumes it (see `HelmPassSurface`).
    tick_twice(&mut app);
    (app, uuid)
}

fn run_to_orbit(start_range: f32) -> (App, uuid::Uuid) {
    run_to_orbit_omitting(start_range, &[])
}

/// Let the ship actually fly for roughly `secs` of SIMULATED flight,
/// touching nothing.
///
/// The physics step is capped at [`HELM_AI_MAX_DT_SECS`] (1/30 s) regardless
/// of how long the fixture's `Time` says a frame took, so flown seconds are
/// counted in physics steps rather than in the fixture's 200 ms frames.
/// Getting this wrong makes a convergence test read as a failure to converge.
fn fly_for(app: &mut App, secs: f32) {
    for _ in 0..((secs / HELM_AI_MAX_DT_SECS).ceil() as usize) {
        tick(app);
    }
}

/// Flown seconds needed for the orbit to reach its steady radius from any of
/// the fixtures below — measured, not guessed: the worst case starts inside
/// the ring facing the wrong way round and takes about 28 s to settle.
const ORBIT_SETTLE_SECS: f32 = 40.0;

/// Sum the ship's swept bearing around the bogey over `secs` of flight. The
/// SIGN is the circulation and the magnitude is how far round it got.
fn swept_bearing(app: &mut App, secs: f32) -> f32 {
    let mut previous = bearing_around_bogey(app);
    let mut swept = 0.0_f32;
    for _ in 0..((secs / HELM_AI_MAX_DT_SECS).ceil() as usize) {
        tick(app);
        let now = bearing_around_bogey(app);
        swept += wrapped_delta(now, previous);
        previous = now;
    }
    swept
}

/// AC1's precondition and AC2's first half: the cruiser commits to the
/// orbit and the host publishes the combat-orbit leg with the hull's OWN
/// authored ring, throttle and gain.
///
/// The `engage_range` half is the anti-trap for an unseeded fact: before the
/// travel axes were seeded, a `fact(range_to_target)` guard validated at
/// load and read false for ever. That this test can distinguish "far" from
/// "near" is the proof the guard actually gates.
#[test]
fn the_cruiser_commits_to_the_orbit_inside_its_authored_engage_range() {
    let engage = cruiser_steering_param("engage_range");

    // Well outside the authored engage range: still closing, not circling.
    let (mut app, _uuid) = broadside_app([0.0, 0.0, -(engage * 3.0)]);
    place_ship(&mut app, 0.0, engage * 3.0, 0.0, 0.0);
    tick_twice(&mut app);
    assert_eq!(
        steering_state(&mut app),
        "acquire",
        "a target beyond engage_range must not start an orbit"
    );
    assert!(
        !pass_surface(&mut app).combat_orbit,
        "and the host must not publish the orbit leg for a ship still closing"
    );

    // Inside it.
    let (mut app, _uuid) = run_to_orbit(engage * 0.5);
    assert_eq!(
        steering_state(&mut app),
        "orbit",
        "inside engage_range the machine must commit to the ring — if this reads \
         `acquire` the travel axis is seeing empty facts"
    );
    assert_eq!(
        engines_state(&mut app),
        "orbit",
        "Engines runs its OWN copy of the machine and must reach the same leg from \
         the same facts, not by reading Steering's state"
    );

    let pass = pass_surface(&mut app);
    assert!(pass.active, "the cruiser must be flying an authored leg");
    assert!(pass.combat_orbit, "and that leg is the combat orbit");
    assert!(
        !pass.recover && !pass.reengage && !pass.escape,
        "the combat orbit is its own leg — it must not masquerade as the \
         shield-recovery standoff, whose ring is derived from the TARGET's reach"
    );
    assert_eq!(
        pass.combat_orbit_range,
        cruiser_steering_param(COMBAT_ORBIT_RANGE_PARAM),
        "the ring is the hull's OWN authored fighting radius"
    );
    assert_eq!(
        pass.combat_orbit_speed,
        cruiser_steering_param(COMBAT_ORBIT_SPEED_PARAM)
    );
    assert_eq!(
        pass.combat_orbit_spiral_gain,
        cruiser_steering_param(COMBAT_ORBIT_SPIRAL_GAIN_PARAM)
    );
    // The two rings are DIFFERENT rings and neither may masquerade as the
    // other. Since issue #878 this hull composes the class doctrine, whose
    // defensive `shadow` leg genuinely holds a standoff derived from the
    // TARGET's reach plus the authored margin — so `safe_range` is published
    // now where it used to be an unauthored zero. What the assertion above
    // still guards is the thing that mattered: the FIGHTING ring is the hull's
    // own authored radius and not this one.
    assert!(
        pass.safe_range > 0.0
            && pass.safe_range != cruiser_steering_param(COMBAT_ORBIT_RANGE_PARAM),
        "the class standoff ring is derived from the target's reach and must \
         stay distinct from the hull's own fighting radius, got {}",
        pass.safe_range
    );
}

/// AC2, the continuous half: the ring is flown UNDER POWER and TURNING, at
/// the authored orbit throttle.
///
/// The throttle assertion is what separates an orbit from the two things it
/// is not: a station-keeper would be braking toward zero at the ring, and a
/// retreat would be running the outward bearing with no turn at all.
#[test]
fn the_orbit_is_flown_as_a_powered_continuous_turn() {
    let orbit_speed = cruiser_steering_param(COMBAT_ORBIT_SPEED_PARAM);
    let ring = cruiser_steering_param(COMBAT_ORBIT_RANGE_PARAM);
    let (mut app, _uuid) = run_to_orbit(ring);
    tick(&mut app);

    assert!(
        (get_thrust_input(&mut app) - orbit_speed).abs() < 1e-3,
        "the ring is flown at the authored orbit throttle ({orbit_speed}), got {}",
        get_thrust_input(&mut app)
    );
    let pass = pass_surface(&mut app);
    assert!(
        pass.orbit_direction == 1.0 || pass.orbit_direction == -1.0,
        "the circulation direction must be a definite choice, got {}",
        pass.orbit_direction
    );

    // Continuous TANGENTIAL movement: the bearing around the target keeps
    // advancing, always the same way. This is the observable form of
    // "continuous tangential movement" — a ship that stopped, or that flew
    // straight at or away from the target, would sweep no bearing at all.
    //
    // Measured after the orbit has settled, because the fixture drops the
    // cruiser onto the ring already flying, and the direction it is dealt
    // may be the opposite of the one it happens to be pointing: the first
    // half-turn is the ship getting onto its chosen circulation, not the
    // circulation itself.
    fly_for(&mut app, ORBIT_SETTLE_SECS);
    assert_eq!(steering_state(&mut app), "orbit");
    let direction = pass_surface(&mut app).orbit_direction;

    let range_before = range_to_bogey(&mut app);
    let swept = swept_bearing(&mut app, 8.0);
    assert!(
        swept * direction > 0.0,
        "the cruiser must circle the way it was dealt (direction {direction}),              but it swept {swept} rad"
    );
    assert!(
        swept.abs() > 0.5,
        "eight seconds on the ring must sweep real bearing, got {swept} rad"
    );
    // ...and it swept that bearing while HOLDING the ring rather than by
    // running past the target: tangential, not radial.
    let range_after = range_to_bogey(&mut app);
    for (label, range) in [("before", range_before), ("after", range_after)] {
        assert!(
            (range - ring).abs() < ring * 0.25,
            "the settled orbit must stay near the authored ring ({ring}); {label}                  the sweep it was at {range}"
        );
    }
}

/// **Issue #918: the settled ring flies the facing its own doctrine solved,
/// tick after tick, under a request that stands for every one of them.**
///
/// The measurement is the admitted yaw against `doctrine_steering` — the
/// planner's own solution for this tick, decoded exactly as
/// `ai_helm_steering` decodes it — and deliberately NOT against the range to
/// the bogey. A hull carries momentum either way, so a radius that looks
/// steady proves nothing about whether the steering command was the
/// doctrine's; and #876 measured a ring that held a plausible radius for a
/// while precisely as Channel 3 was flying it.
///
/// A settled ring is the WORST case for this and that is why it is the one
/// tested: the tangent puts the bogey on the beam by construction, so a
/// fixed fore arc can never bear and the request never self-satisfies. The
/// request is RE-INSERTED every tick by `stand_up_fore_arc_request` — not
/// because Weapons re-raises it that way (`tick_weapons_arc_request` is
/// debounced, `src/console/weapons/mod.rs:651-657`, and only re-fires on a
/// family/target/arc-set change) but because this bare fixture runs no real
/// Weapons system to raise it at all — and asserted still standing at the
/// end — a run that quietly satisfied it would show a clean heading and
/// mean nothing.
#[test]
fn the_settled_ring_keeps_its_solved_facing_under_a_standing_arc_request() {
    let ring = cruiser_steering_param(COMBAT_ORBIT_RANGE_PARAM);
    let (mut app, uuid) = run_to_orbit(ring);
    assert_eq!(steering_state(&mut app), "orbit");
    fly_for(&mut app, ORBIT_SETTLE_SECS);
    assert_eq!(
        steering_state(&mut app),
        "orbit",
        "the fixture must still be on the ring before the request goes up"
    );

    let mut ring_ticks = 0_usize;
    let mut overwritten = 0_usize;
    let mut worst = 0.0_f32;
    for _ in 0..((10.0 / HELM_AI_MAX_DT_SECS) as usize) {
        stand_up_fore_arc_request(&mut app, uuid);
        tick(&mut app);
        if steering_state(&mut app) != "orbit" {
            continue;
        }
        ring_ticks += 1;
        let error = (get_steering_input(&mut app) - doctrine_steering(&mut app)).abs();
        worst = worst.max(error);
        if error > 1e-3 {
            overwritten += 1;
        }
    }

    assert!(
        ring_ticks > 250,
        "only {ring_ticks} of ~300 ticks were flown on the ring: this run did not \
         measure the leg it is about"
    );
    assert_eq!(
        overwritten, 0,
        "{overwritten} of {ring_ticks} ring ticks were flown at a yaw the doctrine \
         did not solve (worst {worst}). That is the sawtooth: a bow-on tracking \
         solution written over the tangent after the planner had already solved it"
    );
    assert!(
        arc_request_stands(&mut app),
        "the request must still be STANDING — declined every tick rather than \
         satisfied. A ring that had satisfied it would hold a clean heading for a \
         reason that has nothing to do with this issue"
    );
}

/// AC2, the spiral half: the cruiser MAINTAINS the authored range from
/// either side of it.
///
/// Two runs, identical but for which side of the ring the cruiser starts on.
/// Both must converge toward the ring, which is what distinguishes a spiral
/// correction from a bare tangent (which would hold whatever radius it
/// started at for ever) and from a retreat or a charge.
#[test]
fn the_orbit_spirals_onto_the_authored_ring_from_inside_and_outside() {
    let ring = cruiser_steering_param(COMBAT_ORBIT_RANGE_PARAM);

    for (label, start) in [("inside", ring * 0.4), ("outside", ring * 2.5)] {
        let (mut app, _uuid) = run_to_orbit(start);
        assert_eq!(steering_state(&mut app), "orbit");
        let before = range_to_bogey(&mut app);
        let error_before = (before - ring).abs();

        fly_for(&mut app, ORBIT_SETTLE_SECS);

        let after = range_to_bogey(&mut app);
        let error_after = (after - ring).abs();
        assert!(
            error_after < error_before,
            "starting {label} the ring ({before} vs {ring}), the spiral must close \
             the radial error, but it went from {error_before} to {error_after}"
        );
        assert_eq!(
            steering_state(&mut app),
            "orbit",
            "and it corrects INSIDE the orbit state — the spiral is the leg, not a \
             separate manoeuvre the hull has to enter"
        );
    }
}

/// AC1: the circulation direction is drawn from a
/// (world, ship, system, transition, occurrence) key, so it reproduces
/// exactly for a given seed — and is not simply a constant.
///
/// The negative half is the load-bearing one: the hull DECLARES
/// `orbit_direction = 1.0` in its authored memory, so a host that never drew
/// would publish `+1.0` every time and pass every other assertion in this
/// file.
#[test]
fn the_combat_orbit_direction_is_deterministic_from_the_seed_without_being_constant() {
    fn direction_for(seed: u64, ship: uuid::Uuid) -> f32 {
        let ring = cruiser_steering_param(COMBAT_ORBIT_RANGE_PARAM);
        let (mut app, _bogey) = broadside_app(ORBIT_BOGEY);
        app.insert_resource(crate::sim_rng::SimRng::new(
            seed,
            crate::sim_rng::SeedSource::Cli,
        ));
        let entity = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(entity)
            .insert(crate::entities::spawner::EntityUuid(ship.to_string()));
        place_ship(&mut app, 0.0, -ring, std::f32::consts::FRAC_PI_2, 9.0);
        tick_twice(&mut app);
        assert_eq!(steering_state(&mut app), "orbit");
        pass_surface(&mut app).orbit_direction
    }

    let ship = uuid::Uuid::from_u128(0x1234_5678_9abc_def0_1234_5678_9abc_def0);
    // Reproducible: same world seed, same ship, same answer. This is the
    // property a replayed `--seed` run depends on.
    assert_eq!(direction_for(4242, ship), direction_for(4242, ship));

    // Not a constant: over a handful of seeds both directions occur.
    let directions: Vec<f32> = [1, 2, 3, 4, 5, 6, 7, 8]
        .into_iter()
        .map(|seed| direction_for(seed, ship))
        .collect();
    assert!(
        directions.contains(&1.0) && directions.contains(&-1.0),
        "the direction must genuinely vary with the seed, got {directions:?}"
    );
}

/// AC3: a hazard detour BENDS the orbit and never exits it, and the same
/// circulation resumes once the hazard is clear.
///
/// Three things are asserted and each has its own failure mode:
///
/// * the steering command genuinely changes while the obstacle is there —
///   without this the rest of the test would pass on a hazard the ship never
///   noticed;
/// * the committed policy state and the drawn direction are untouched — a
///   transition guarded on urgency would exit the orbit, and RE-entering it
///   would re-draw the direction, so flying past debris would randomise
///   which way the cruiser circles;
/// * the bearing keeps advancing the same way afterwards — the resume.
#[test]
fn a_hazard_detour_bends_the_orbit_without_changing_its_direction() {
    let ring = cruiser_steering_param(COMBAT_ORBIT_RANGE_PARAM);
    let (mut app, bogey) = run_to_orbit(ring);
    // Settle onto the ring first: a cruiser still hauling itself round onto
    // its chosen circulation is commanding saturated steering, and a
    // saturated command cannot be observed to bend.
    fly_for(&mut app, ORBIT_SETTLE_SECS);
    assert_eq!(steering_state(&mut app), "orbit");
    let direction = pass_surface(&mut app).orbit_direction;
    let clean_steering = get_steering_input(&mut app);
    assert!(
        clean_steering.abs() < 1.0,
        "precondition: the settled orbit must have steering authority to spare,              or a detour could not show up in the command at all (got {clean_steering})"
    );

    // Drop an obstacle right on the ship's projected path. Deliberately NOT
    // the target — the orbit's own centre is excluded from the avoidance
    // scan, because circling a thing you are also fleeing is incoherent.
    let pose = ship_pose(&mut app);
    let hazard = uuid::Uuid::from_u128(0x0b57ac1e);
    app.insert_resource(crate::ai::server::WorldSnapshot {
        entities: vec![
            crate::ai::AiWorldEntity {
                uuid: bogey,
                name: Some(BOGEY.into()),
                position: ORBIT_BOGEY,
                yaw: Some(0.0),
                forward_speed: 0.0,
                radius: 3.0,
                size_rating: 3.0,
                movable: true,
                dangerous: true,
                ..Default::default()
            },
            crate::ai::AiWorldEntity {
                uuid: hazard,
                name: Some("rock".into()),
                position: [
                    pose.x + simmath::sin(pose.yaw) * 12.0,
                    0.0,
                    pose.z - simmath::cos(pose.yaw) * 12.0,
                ],
                yaw: None,
                forward_speed: 0.0,
                radius: 25.0,
                size_rating: 25.0,
                movable: false,
                dangerous: false,
                ..Default::default()
            },
        ],
    });
    tick(&mut app);
    tick(&mut app);

    assert!(
        (get_steering_input(&mut app) - clean_steering).abs() > 1e-3,
        "precondition: the obstacle must actually bend the steering solution \
         (clean {clean_steering}, detour {})",
        get_steering_input(&mut app)
    );
    assert_eq!(
        steering_state(&mut app),
        "orbit",
        "a detour must not exit the orbit — nothing in this doctrine is guarded on \
         a hazard reading"
    );
    assert_eq!(engines_state(&mut app), "orbit");
    let during = pass_surface(&mut app);
    assert!(during.combat_orbit, "the published leg is still the orbit");
    assert_eq!(
        during.orbit_direction, direction,
        "and the circulation direction survives the detour untouched"
    );

    // Clear the hazard: the cruiser resumes the SAME circulation.
    set_bogey(&mut app, bogey, ORBIT_BOGEY, 0.0, 0.0);
    tick(&mut app);
    tick(&mut app);
    let resumed = pass_surface(&mut app);
    assert!(resumed.combat_orbit);
    assert_eq!(
        resumed.orbit_direction, direction,
        "clearing the hazard must not re-draw the direction either"
    );
    assert_eq!(steering_state(&mut app), "orbit");

    let mut previous = bearing_around_bogey(&mut app);
    let mut swept = 0.0_f32;
    for _ in 0..30 {
        tick(&mut app);
        let now = bearing_around_bogey(&mut app);
        swept += wrapped_delta(now, previous);
        previous = now;
    }
    assert!(
        swept * direction > 0.0,
        "after the detour the cruiser must go on circling the way it chose \
         (direction {direction}), but it swept {swept} rad"
    );
}

/// "Decline rather than invent", on all three combat-orbit scalars.
///
/// Each fails differently if admitted alone, and each failure is worse than
/// not flying the arm: without `combat_orbit_range` the host would solve a
/// tangent of a ring of radius zero, which is a spiral straight into the
/// target; without `combat_orbit_speed` the cruiser would sit at zero
/// throttle inside a hostile's guns; without `combat_orbit_spiral_gain` it
/// would fly the bare tangent and hold whatever radius it happened to arrive
/// at, for ever. So the host gates on all three together and the hull falls
/// back to ordinary doctrine travel — a behaviour a designer can see.
///
/// The shipped hull orbits at this exact point (asserted above), so nothing
/// here passes for want of getting that far.
#[test]
fn a_hull_omitting_any_combat_orbit_scalar_declines_the_whole_arm() {
    let ring = cruiser_steering_param(COMBAT_ORBIT_RANGE_PARAM);
    for omitted in COMBAT_ORBIT_PARAMS {
        let (mut app, _uuid) = run_to_orbit_omitting(ring, &[omitted]);
        // The machine still enters the state — the verb parses and resolves
        // with or without the params. What must not happen is the HOST
        // flying a ring it has no numbers for.
        assert_eq!(
            steering_state(&mut app),
            "orbit",
            "omitting `{omitted}` must not change which state is entered"
        );
        let pass = pass_surface(&mut app);
        assert!(
            !pass.combat_orbit,
            "omitting `{omitted}` must decline the combat-orbit arm outright"
        );
        assert_eq!(
            pass.combat_orbit_range, 0.0,
            "the whole arm declines together, not part of it"
        );

        // And it stays declined: a run that keeps flying must not quietly
        // start orbiting a few ticks later.
        fly_for(&mut app, 3.0);
        assert!(
            !pass_surface(&mut app).combat_orbit,
            "omitting `{omitted}` must keep declining the arm"
        );
    }
}

// ── The cruiser's shield-opportunity torpedo phase (issue #791) ──────────
//
// The orbit fixtures above only ever needed the bogey as a row in the
// `WorldSnapshot`, because ring geometry is solved from the merged view. The
// torpedo phase is different in two ways, and both drive the fixtures below:
//
// * the arc of the TARGET that faces this ship is resolved through that
//   target's own `Transform` + `ShipShields`, through the same
//   `attacker_bearing_relative` → `facing_index_for_bearing` pair damage
//   takes — so the bogey has to be a real entity, not a snapshot row;
// * "the salvo has resolved" is `TorpedoSystem::in_flight` being empty on
//   this ship's OWN component, so the cruiser has to carry one.
//
// The result is a fixture that composes the helm and weapons conventions,
// which is what AC7 spans.

/// Give the cruiser the shipped hull's real torpedo system — the same
/// `from_configs` call `entities::spawner` makes — so tube capacity, the
/// magazine and `in_flight` are the hull's own and not a fixture's idea of
/// them.
fn attach_cruiser_torpedoes(app: &mut App) {
    let cfg = cruiser_hull();
    let torpedoes = cfg
        .torpedoes
        .as_ref()
        .expect("the shipped cruiser must carry a torpedo magazine");
    let ship = find_ship_entity(app);
    app.world_mut()
        .entity_mut(ship)
        .insert(crate::console::weapons::TorpedoSystemResource(
            crate::weapons::torpedo::TorpedoSystem::from_configs(
                &torpedoes.tubes,
                torpedoes.to_runtime(),
            ),
        ));
}

/// Spawn the bogey as a real ECS entity carrying shields, alongside the
/// snapshot row `set_bogey` already wrote.
///
/// Deliberately carries NO `Ship`/`LocalShip` marker and no `ShipPhysics`:
/// every helper above resolves the cruiser through a `With<Ship>` filter, and
/// a second marked ship would make them ambiguous. A target with no physics
/// reads yaw 0, which is exactly what `ai_torpedo_auto_fire` does for the
/// same case.
fn spawn_bogey_entity(app: &mut App, uuid: uuid::Uuid, pos: [f32; 3]) -> Entity {
    app.world_mut()
        .spawn((
            crate::entities::spawner::EntityUuid(uuid.to_string()),
            Transform::from_xyz(pos[0], pos[1], pos[2]),
            crate::server_app::ShipShields(crate::weapons::shield::ShieldSystem::default(), 0.5),
        ))
        .id()
}

/// Which of the bogey's arcs currently faces the cruiser, resolved the way
/// the host resolves it: the bearing of the attacker in the TARGET's frame,
/// through the target's own priority-tiered router.
///
/// The tests below flip arcs BY THIS INDEX rather than by a hardcoded one, so
/// they keep meaning "the arc that faces us" if the shield layout is ever
/// re-authored — and so the negative case ("some other arc is down") can be
/// expressed at all.
fn bogey_facing_index(app: &mut App, bogey: Entity) -> usize {
    let pose = ship_pose(app);
    let transform = *app
        .world()
        .get::<Transform>(bogey)
        .expect("bogey carries a Transform");
    let shields = app
        .world()
        .get::<crate::server_app::ShipShields>(bogey)
        .expect("bogey carries ShipShields");
    let incoming = crate::weapons::shield::attacker_bearing_relative(
        pose.x,
        pose.z,
        transform.translation.x,
        transform.translation.z,
        0.0,
    );
    shields.0.facing_index_for_bearing(incoming)
}

/// Knock one of the bogey's arcs offline (or bring it back).
fn set_bogey_arc_online(app: &mut App, bogey: Entity, index: usize, online: bool) {
    let mut shields = app
        .world_mut()
        .get_mut::<crate::server_app::ShipShields>(bogey)
        .expect("bogey carries ShipShields");
    shields.0.facings[index].offline_remaining = if online { 0.0 } else { 30.0 };
}

/// How many rounds the cruiser has in the air, written directly. Standing in
/// for `handle_fire_torpedo` (which is not in this fixture's schedule): what
/// the doctrine reads is the count, and writing it is the smallest thing that
/// makes the salvo half of AC4 observable.
fn set_torpedoes_in_flight(app: &mut App, n: usize) {
    let ship = find_ship_entity(app);
    let mut entity = app.world_mut().entity_mut(ship);
    let mut torpedoes = entity
        .get_mut::<crate::console::weapons::TorpedoSystemResource>()
        .expect("the cruiser carries a torpedo system");
    torpedoes.0.in_flight.clear();
    for i in 0..n {
        torpedoes
            .0
            .in_flight
            .push(crate::weapons::torpedo::Torpedo {
                uuid: format!("salvo-{i}"),
                x: 0.0,
                y: 0.0,
                z: 0.0,
                heading: 0.0,
                pitch: 0.0,
                lifespan_remaining: 5.0,
                target_uuid: None,
                source_uuid: None,
                tube_id: "bow_port".into(),
                shield_pierce: 0.0,
            });
    }
}

/// Bring every tube to its authored `volley_max` and take the rounds out of
/// the magazine, exactly as a completed load cycle would.
///
/// Standing in for the loader, which is not in this fixture's schedule, and
/// it has to stand in because the entry guard asks `tubes_full`: a cruiser
/// fresh off `from_configs` has empty tubes and a 9-second-per-round load
/// time, so without this no fixture below would ever reach the phase at all.
/// That is the doctrine working — a reloading cruiser keeps circling — but it
/// makes "loaded" a precondition every opportunity fixture has to establish
/// rather than assume.
fn fill_the_tubes(app: &mut App) {
    let ship = find_ship_entity(app);
    let mut entity = app.world_mut().entity_mut(ship);
    let mut torpedoes = entity
        .get_mut::<crate::console::weapons::TorpedoSystemResource>()
        .expect("the cruiser carries a torpedo system");
    let mut drawn = 0;
    for tube in &mut torpedoes.0.tubes {
        drawn += tube.volley_max.saturating_sub(tube.loaded_count);
        tube.loaded_count = tube.volley_max;
        tube.load_state = crate::weapons::torpedo::TubeLoadState::Unloaded;
    }
    torpedoes.0.torpedoes_remaining = torpedoes.0.torpedoes_remaining.saturating_sub(drawn);
}

/// Empty every tube without touching the magazine — the tube half of the
/// state a cruiser is in the instant after it launches.
fn empty_the_tubes(app: &mut App) {
    let ship = find_ship_entity(app);
    let mut entity = app.world_mut().entity_mut(ship);
    let mut torpedoes = entity
        .get_mut::<crate::console::weapons::TorpedoSystemResource>()
        .expect("the cruiser carries a torpedo system");
    for tube in &mut torpedoes.0.tubes {
        tube.loaded_count = 0;
        tube.load_state = crate::weapons::torpedo::TubeLoadState::Unloaded;
    }
}

/// The cruiser settled on its fighting ring, with a live shielded bogey
/// entity, its own tubes, and a salvo loaded in them. Returns the app, the
/// bogey's uuid and the bogey's entity.
fn opportunity_app_omitting(omit: &[&str]) -> (App, uuid::Uuid, Entity) {
    let ring = cruiser_steering_param(COMBAT_ORBIT_RANGE_PARAM);
    let (mut app, uuid) = run_to_orbit_omitting(ring, omit);
    attach_cruiser_torpedoes(&mut app);
    fill_the_tubes(&mut app);
    let bogey = spawn_bogey_entity(&mut app, uuid, ORBIT_BOGEY);
    tick_twice(&mut app);
    (app, uuid, bogey)
}

fn opportunity_app() -> (App, uuid::Uuid, Entity) {
    opportunity_app_omitting(&[])
}

/// Signed bearing from the cruiser's own bow to the bogey, radians. Zero is
/// bow-on; the sign is which side the target sits.
fn bearing_to_bogey(app: &mut App) -> f32 {
    let p = ship_pose(app);
    crate::ai::target_relative_motion(
        [p.x, p.y, p.z],
        p.yaw,
        p.forward_speed,
        ORBIT_BOGEY,
        Some(0.0),
        0.0,
    )
    .bearing_rad
}

/// AC1: a down target-facing shield arc breaks the orbit into a bow-on hold,
/// and the hold cuts thrust.
///
/// The negative half comes first and is the anti-trap for an unseeded fact:
/// a `fact(target_facing_shield_down)` name that were never seeded would
/// parse, validate, and read false for ever, so the cruiser would circle
/// exactly as it does today and every #790 assertion would still pass. That
/// this fixture can distinguish "arc up" from "arc down" is the proof.
#[test]
fn a_downed_target_arc_breaks_the_orbit_into_a_bow_on_hold_with_thrust_cut() {
    let (mut app, _uuid, bogey) = opportunity_app();

    // Healthy shields: the cruiser keeps circling.
    assert_eq!(steering_state(&mut app), "orbit");
    assert!(pass_surface(&mut app).combat_orbit);
    assert!(
        !pass_surface(&mut app).torpedo_bearing,
        "a target behind a healthy arc offers no opportunity"
    );

    // Knock the arc that actually faces us offline.
    let facing = bogey_facing_index(&mut app, bogey);
    set_bogey_arc_online(&mut app, bogey, facing, false);
    tick_twice(&mut app);

    assert_eq!(
        steering_state(&mut app),
        "torpedo_run",
        "a down facing arc must break the orbit — if this reads `orbit` the \
         shield fact is not reaching the transition guard"
    );
    assert_eq!(
        engines_state(&mut app),
        "torpedo_run",
        "Engines runs its OWN copy of the machine and must reach the phase from \
         the same shared facts, not by reading Steering's state"
    );

    let pass = pass_surface(&mut app);
    assert!(pass.active);
    assert!(pass.torpedo_bearing, "the published leg is the bow hold");
    assert!(
        !pass.combat_orbit && !pass.recover && !pass.reengage && !pass.escape,
        "the bow hold is its own leg — it must not masquerade as the ring it \
         just left, nor as the recovery pivot whose geometry it shares"
    );
    assert_eq!(
        pass.torpedo_bearing_speed,
        cruiser_steering_param(TORPEDO_BEARING_SPEED_PARAM),
        "the hold flies the hull's OWN authored throttle"
    );

    // The thrust axis actually cuts, and the ship actually slows.
    let entry_speed = ship_pose(&mut app).forward_speed;
    assert!(
        entry_speed > 0.0,
        "precondition: the cruiser must have been moving, or 'cuts thrust' is \
         unobservable"
    );
    fly_for(&mut app, 2.0);
    assert_eq!(steering_state(&mut app), "torpedo_run");
    assert_eq!(
        get_thrust_input(&mut app),
        cruiser_steering_param(TORPEDO_BEARING_SPEED_PARAM),
        "the commanded throttle is the authored bow-hold fraction"
    );
    assert!(
        ship_pose(&mut app).forward_speed < entry_speed,
        "and the hull is genuinely slowing: {} vs {entry_speed}",
        ship_pose(&mut app).forward_speed
    );

    // ...and it swings its bow onto the target rather than holding the beam
    // aspect the ring left it in. The hull's authored `max_yaw_rate` is 0.30
    // rad/s and the orbit hands the phase a target roughly abeam, so this is
    // a manoeuvre that takes several seconds by construction.
    let entry_bearing = bearing_to_bogey(&mut app).abs();
    fly_for(&mut app, 8.0);
    assert_eq!(steering_state(&mut app), "torpedo_run");
    let held_bearing = bearing_to_bogey(&mut app).abs();
    assert!(
        held_bearing < entry_bearing && held_bearing < 0.2,
        "the bow must come onto the target ({entry_bearing} → {held_bearing} rad)"
    );
}

/// AC1's other half: the arc that matters is the one that FACES this ship,
/// resolved through the target's own router.
///
/// A cruiser that opened the phase on "any arc is down" would break its orbit
/// for a hole in the far side of the enemy and then hold its bow on a healthy
/// shield until something else moved. This is the assertion that the belief
/// and the shot agree about which arc is in the way.
#[test]
fn only_the_arc_that_faces_the_cruiser_opens_the_opportunity() {
    let (mut app, _uuid, bogey) = opportunity_app();
    let facing = bogey_facing_index(&mut app, bogey);

    // Knock out every OTHER arc.
    let arcs = app
        .world()
        .get::<crate::server_app::ShipShields>(bogey)
        .expect("bogey carries ShipShields")
        .0
        .facings
        .len();
    assert!(arcs > 1, "precondition: the bogey must have several arcs");
    for index in 0..arcs {
        if index != facing {
            set_bogey_arc_online(&mut app, bogey, index, false);
        }
    }
    fly_for(&mut app, 1.0);
    assert_eq!(
        steering_state(&mut app),
        "orbit",
        "arcs on the far side of the target are no opportunity at all"
    );
    assert!(!pass_surface(&mut app).torpedo_bearing);

    // Now the one that does face us.
    set_bogey_arc_online(&mut app, bogey, facing, false);
    tick_twice(&mut app);
    assert_eq!(steering_state(&mut app), "torpedo_run");
}

/// Leave the cruiser in the state it is in after its last salvo: nothing in
/// the tubes and nothing left in the magazine to reload them with.
fn spend_the_magazine(app: &mut App) {
    let ship = find_ship_entity(app);
    let mut entity = app.world_mut().entity_mut(ship);
    let mut torpedoes = entity
        .get_mut::<crate::console::weapons::TorpedoSystemResource>()
        .expect("the cruiser carries a torpedo system");
    torpedoes.0.torpedoes_remaining = 0;
    for tube in &mut torpedoes.0.tubes {
        tube.loaded_count = 0;
        tube.load_state = crate::weapons::torpedo::TubeLoadState::Unloaded;
    }
}

/// Put `rounds` back in the magazine — the positive control for the two
/// tests below, so neither can pass by simply never reaching the phase.
fn restock_the_magazine(app: &mut App, rounds: u32) {
    let ship = find_ship_entity(app);
    let mut entity = app.world_mut().entity_mut(ship);
    let mut torpedoes = entity
        .get_mut::<crate::console::weapons::TorpedoSystemResource>()
        .expect("the cruiser carries a torpedo system");
    torpedoes.0.torpedoes_remaining = rounds;
}

/// Mark one of the cruiser's fine systems damage-offline, the way
/// `sync_console_damage_tiers` does when a console crosses into
/// Disabled/Destroyed.
fn knock_system_offline(app: &mut App, system_id: &str) {
    let ship = find_ship_entity(app);
    let mut entity = app.world_mut().entity_mut(ship);
    let mut sources = entity
        .get_mut::<ShipSystemControlSources>()
        .expect("the cruiser carries control sources");
    sources
        .0
        .set_offline(crate::core::messages::SystemId(system_id.into()), true);
}

/// The doctrine-quality guard #790's premise depends on: a cruiser that
/// CANNOT launch does not give up its broadside orbit to try.
///
/// The end of the line for this hull's armament: the state it is left in
/// after its last salvo, tubes empty and no rounds left to refill them with.
/// The magazine is 8 rounds against a 4-round salvo, so the cruiser gets two
/// torpedo runs and then never another, and from that point on every arc it
/// collapses is somebody else's opportunity.
///
/// With no armament conjunct on the entry guard at all the cruiser could not
/// see that: from the third arc collapse onward it broke the ring, cut thrust
/// and held its nose on the enemy for the rest of the fight. Measured over a
/// 180 s headless `combat_test` run, 287 ticks in `torpedo_run` against 87 in
/// `orbit` — the interruption had become the hull's normal combat mode, which
/// is precisely what #790 says it must not be.
///
/// The restock at the end is the anti-vacuity half: the same arc, the same
/// target, the same tick cadence, and the only things that changed are the
/// rounds.
#[test]
fn a_spent_magazine_keeps_the_cruiser_in_its_orbit() {
    let (mut app, _uuid, bogey) = opportunity_app();
    spend_the_magazine(&mut app);

    // The opportunity opens, and it is a real one — the arc that faces us is
    // down. The only thing missing is anything to shoot through it with.
    let facing = bogey_facing_index(&mut app, bogey);
    set_bogey_arc_online(&mut app, bogey, facing, false);
    fly_for(&mut app, 2.0);

    assert_eq!(
        steering_state(&mut app),
        "orbit",
        "a cruiser with an empty magazine must keep circling: breaking the ring \
         costs it the broadside and buys a salvo it cannot load"
    );
    assert_eq!(
        engines_state(&mut app),
        "orbit",
        "the travel axis reads the same fact and must reach the same conclusion"
    );
    let pass = pass_surface(&mut app);
    assert!(pass.combat_orbit, "the ring is still the manoeuvre");
    assert!(
        !pass.torpedo_bearing,
        "and the bow hold was never published"
    );

    // Anti-vacuity: rounds back and a salvo loaded from them, same
    // everything else, phase opens. Both halves are needed because both
    // halves of "can shoot into it" are guarded — restocking alone leaves
    // the hull in its reload window, which is the case the test below owns.
    restock_the_magazine(&mut app, 8);
    fill_the_tubes(&mut app);
    tick_twice(&mut app);
    assert_eq!(
        steering_state(&mut app),
        "torpedo_run",
        "with the magazine restocked the SAME opportunity must open — otherwise \
         this test is only asserting that the phase never runs"
    );
}

/// The case that is `tubes_fillable`'s alone, and the reason it stays
/// conjoined beside `tubes_full` rather than being replaced by it: a tube
/// that has been shot out.
///
/// The fixture arrives here with a full salvo loaded, so `tubes_full` reads
/// TRUE throughout — being loaded is a fact about the rounds in the tubes and
/// says nothing about whether the tube can still fire them. Only
/// `tubes_fillable` looks at the fine system, and `handle_fire_torpedo` gates
/// a launch on exactly that, so a cruiser without this conjunct would break
/// its orbit and hold its nose out for a salvo the launcher will decline.
///
/// One dead tube is enough: the salvo doctrine this hull authors is
/// all-or-nothing, so the launch gate is an ALL-tubes reading however healthy
/// the other tube is.
#[test]
fn a_shot_out_tube_keeps_the_cruiser_in_its_orbit() {
    let (mut app, _uuid, bogey) = opportunity_app();
    knock_system_offline(&mut app, "torpedo-tube-bow-port");

    let facing = bogey_facing_index(&mut app, bogey);
    set_bogey_arc_online(&mut app, bogey, facing, false);
    fly_for(&mut app, 2.0);

    assert_eq!(
        steering_state(&mut app),
        "orbit",
        "one dead tube makes a full salvo unreachable, so the opportunity is \
         not one"
    );
    assert!(!pass_surface(&mut app).torpedo_bearing);
}

/// The RELEASE half of the case above, which the entry guard cannot cover
/// and the salvo-spent resume structurally cannot see.
///
/// `tubes_full` reads the rounds, not the tubes. Destroying a tube leaves
/// `loaded_count` exactly where it was, so a hull that loses a tube AFTER
/// the phase opens still reads `tubes_full` true, has nothing in the air,
/// and — against a target that carries no arc to raise — has every exit
/// shut. It sits bow-on, thrust cut, for a salvo `handle_fire_torpedo` will
/// decline, which is the same trap the salvo-spent bound was added to close,
/// reopened by a lucky hit.
///
/// The bound that catches it is reachability, which the entry guard already
/// asks and which now sits on an exit as well. The target is deliberately
/// shieldless so the window-closed resume is unable to fire at all: whatever
/// releases the hull here can only be the hull's own armament.
#[test]
fn a_tube_shot_out_mid_phase_releases_the_cruiser_back_to_its_orbit() {
    let (mut app, _uuid, bogey) = opportunity_app();
    app.world_mut()
        .entity_mut(bogey)
        .remove::<crate::server_app::ShipShields>();
    tick_twice(&mut app);
    assert_eq!(
        steering_state(&mut app),
        "torpedo_run",
        "precondition: an unshielded target opens the phase"
    );

    // Battery intact and loaded: the phase holds, and it holds indefinitely
    // — this target will never close the window for it.
    fly_for(&mut app, 3.0);
    assert_eq!(steering_state(&mut app), "torpedo_run");
    assert!(
        tubes_are_full(&mut app),
        "precondition: a full battery is what makes the salvo-spent resume \
         unable to end this phase"
    );

    // A hit takes a tube out while rounds are still in the air.
    knock_system_offline(&mut app, "torpedo-tube-bow-port");
    set_torpedoes_in_flight(&mut app, 2);
    fly_for(&mut app, 2.0);
    assert_eq!(
        steering_state(&mut app),
        "torpedo_run",
        "the battery-lost resume takes the same in-flight conjunct as the \
         other two: a hull does not turn away from rounds it has committed, \
         whatever has happened to the tube that fired them"
    );

    // The rounds resolve. Nothing is committed, the battery can never be
    // filled again, and there is no arc to come back.
    set_torpedoes_in_flight(&mut app, 0);
    fly_for(&mut app, 2.0);
    assert!(
        tubes_are_full(&mut app),
        "the rounds are still sitting in the dead tube — which is precisely \
         why `tubes_full` cannot be the reading that ends this phase"
    );
    assert_eq!(
        steering_state(&mut app),
        "orbit",
        "a cruiser whose battery has been shot out must go back to raking \
         with the beams it still has — with a full battery, nothing in the \
         air and a target with no arc to raise, reachability is the ONLY \
         thing left that can release the hull"
    );
    assert_eq!(
        engines_state(&mut app),
        "orbit",
        "the travel axis is bounded by the same reading, not by Steering's state"
    );
    let pass = pass_surface(&mut app);
    assert!(
        pass.combat_orbit && !pass.torpedo_bearing,
        "and the published leg is the ring again, with thrust restored"
    );

    // ...and it stays out: the entry guard asks reachability too, so a dead
    // tube keeps the phase shut rather than chattering the hull in and out.
    fly_for(&mut app, 4.0);
    assert_eq!(steering_state(&mut app), "orbit");
}

/// The dominant-state bug, and the reason the entry guard asks the
/// LAUNCHER's question rather than the reachability one.
///
/// This is the reload window: tubes empty, magazine full, every fine system
/// healthy. `tubes_fillable` is TRUE throughout it — a whole salvo is
/// perfectly reachable, in 18 seconds — and it is true throughout the
/// initial load-up too, so a cruiser guarded on reachability alone breaks
/// its ring for arc collapses it cannot possibly shoot into. Measured over a
/// 400 sim-second `combat_test` run before `tubes_full` was conjoined: 506
/// bow-on ticks against 431 orbiting, and only 29 of the 506 — 5.7% — with
/// the tubes actually full. The "brief interruption" was the majority of
/// resolved manoeuvre time and almost none of it could have ended in a shot.
///
/// The restock/spend pair at the end is the anti-vacuity half, and it is
/// deliberately the OTHER axis of the same question: same arc, same target,
/// same cadence, magazine untouched, and the only thing that changes is
/// whether the rounds are in the tubes.
#[test]
fn a_cruiser_mid_reload_keeps_its_orbit_however_full_the_magazine_is() {
    let (mut app, _uuid, bogey) = opportunity_app();
    // The state the hull is in for 18 seconds after every salvo: nothing in
    // the tubes, plenty in the magazine.
    empty_the_tubes(&mut app);
    restock_the_magazine(&mut app, 8);

    let facing = bogey_facing_index(&mut app, bogey);
    set_bogey_arc_online(&mut app, bogey, facing, false);
    fly_for(&mut app, 3.0);

    assert_eq!(
        steering_state(&mut app),
        "orbit",
        "a reloading cruiser must keep circling: the tubes cannot fill inside \
         the window, so breaking the ring buys a shot that will not be taken"
    );
    assert_eq!(
        engines_state(&mut app),
        "orbit",
        "the travel axis reads the same facts and must reach the same conclusion"
    );
    let pass = pass_surface(&mut app);
    assert!(pass.combat_orbit, "the ring is still the manoeuvre");
    assert!(
        !pass.torpedo_bearing,
        "and the bow hold was never published"
    );

    // Anti-vacuity: load the salvo and nothing else. The magazine was
    // already full, so `tubes_fillable` did not change — only `tubes_full`
    // did, which is the whole claim.
    fill_the_tubes(&mut app);
    tick_twice(&mut app);
    assert_eq!(
        steering_state(&mut app),
        "torpedo_run",
        "with the salvo loaded the SAME opportunity must open — otherwise this \
         test is only asserting that the phase never runs"
    );
}

/// AC3: a shield that recovers before anything has launched aborts the
/// opportunity, and the cruiser resumes its orbit.
#[test]
fn a_recovered_shield_aborts_the_opportunity_before_launch() {
    let (mut app, _uuid, bogey) = opportunity_app();
    let facing = bogey_facing_index(&mut app, bogey);
    set_bogey_arc_online(&mut app, bogey, facing, false);
    tick_twice(&mut app);
    assert_eq!(steering_state(&mut app), "torpedo_run");
    assert_eq!(
        torpedoes_in_flight(&mut app),
        0,
        "precondition: this is the BEFORE-launch abort"
    );

    set_bogey_arc_online(&mut app, bogey, facing, true);
    tick_twice(&mut app);
    assert_eq!(
        steering_state(&mut app),
        "orbit",
        "with nothing in the air, a recovered arc ends the opportunity"
    );
    assert_eq!(engines_state(&mut app), "orbit");
    let pass = pass_surface(&mut app);
    assert!(
        pass.combat_orbit && !pass.torpedo_bearing,
        "and the published leg goes back to the ring"
    );
}

/// The cruiser's own count of rounds in the air, read off the live component
/// the doctrine reads.
fn torpedoes_in_flight(app: &mut App) -> usize {
    app.world_mut()
        .query::<&crate::console::weapons::TorpedoSystemResource>()
        .single(app.world())
        .expect("the cruiser carries a torpedo system")
        .0
        .in_flight
        .len()
}

/// AC4: once a salvo is away the cruiser stays bow-on until every round has
/// hit, missed or expired — even after the shield it was shooting at comes
/// back.
///
/// The recovered-shield half is the load-bearing one. A shield regenerates
/// while the rounds are flying essentially every time, so an exit guarded on
/// the shield alone would turn the cruiser away mid-salvo in almost every
/// real engagement — and the abort test above would still pass.
#[test]
fn a_salvo_in_flight_holds_the_cruiser_bow_on_after_the_shield_recovers() {
    let (mut app, _uuid, bogey) = opportunity_app();
    let facing = bogey_facing_index(&mut app, bogey);
    set_bogey_arc_online(&mut app, bogey, facing, false);
    tick_twice(&mut app);
    assert_eq!(steering_state(&mut app), "torpedo_run");

    // A salvo is away, and the arc recovers behind it.
    set_torpedoes_in_flight(&mut app, 2);
    set_bogey_arc_online(&mut app, bogey, facing, true);
    fly_for(&mut app, 2.0);
    assert_eq!(
        steering_state(&mut app),
        "torpedo_run",
        "a recovered shield must NOT release the hull while its own rounds are \
         still in the air"
    );
    assert!(pass_surface(&mut app).torpedo_bearing);

    // One round resolves; one is still flying. Still committed.
    set_torpedoes_in_flight(&mut app, 1);
    fly_for(&mut app, 1.0);
    assert_eq!(
        steering_state(&mut app),
        "torpedo_run",
        "the commitment is to the whole salvo, not to its first round"
    );

    // The last one hits, misses or expires — `in_flight` is empty either way.
    set_torpedoes_in_flight(&mut app, 0);
    tick_twice(&mut app);
    assert_eq!(
        steering_state(&mut app),
        "orbit",
        "with the salvo resolved and the arc back, the phase is over"
    );
    assert!(pass_surface(&mut app).combat_orbit);
}

/// Fire a REAL salvo out of the cruiser's own tubes, through the same
/// `TorpedoSystem::launch` call `handle_fire_torpedo` makes.
///
/// Deliberately not [`set_torpedoes_in_flight`], which writes `in_flight`
/// directly and so stages a salvo that is fully airborne from its first tick.
/// A real burst launch is not: round 0 of each tube goes immediately and the
/// rest are left as a [`crate::weapons::torpedo::TubeBurstState`] waiting on
/// `burst_interval_secs`. Returns `(airborne, owed)`.
fn launch_a_real_salvo(app: &mut App) -> (usize, u32) {
    let ship = find_ship_entity(app);
    let mut entity = app.world_mut().entity_mut(ship);
    let mut torpedoes = entity
        .get_mut::<crate::console::weapons::TorpedoSystemResource>()
        .expect("the cruiser carries a torpedo system");
    let ids: Vec<String> = torpedoes.0.tubes.iter().map(|t| t.id.clone()).collect();
    for id in &ids {
        let result = torpedoes
            .0
            .launch(id, format!("salvo-{id}"), 0.0, 0.0, 0.0, 0.0, None, None);
        assert!(
            matches!(
                result,
                crate::weapons::torpedo::LaunchResult::Launched { .. }
            ),
            "precondition: tube '{id}' must have a volley loaded to fire, got {result:?}"
        );
    }
    (
        torpedoes.0.in_flight.len(),
        torpedoes.0.burst_states.iter().map(|b| b.pending).sum(),
    )
}

/// Rounds this ship has COMMITTED to a burst but not yet put in the air.
fn torpedoes_pending(app: &mut App) -> u32 {
    let ship = find_ship_entity(app);
    app.world()
        .get::<crate::console::weapons::TorpedoSystemResource>(ship)
        .expect("the cruiser carries a torpedo system")
        .0
        .burst_states
        .iter()
        .map(|b| b.pending)
        .sum()
}

/// Whether every tube is at its `volley_max` — the reading the doctrine's
/// `tubes_full` fact takes, read here so a fixture can assert on the state
/// that arms the salvo-spent resume.
fn tubes_are_full(app: &mut App) -> bool {
    let ship = find_ship_entity(app);
    let torpedoes = app
        .world()
        .get::<crate::console::weapons::TorpedoSystemResource>(ship)
        .expect("the cruiser carries a torpedo system");
    !torpedoes.0.tubes.is_empty()
        && torpedoes
            .0
            .tubes
            .iter()
            .all(|t| t.loaded_count >= t.volley_max)
}

/// Advance the cruiser's own torpedo system by `dt` through the real
/// `TorpedoSystem::tick`, so a burst pays its owed rounds out by production
/// code rather than by fixture fiat. This fixture's schedule carries no
/// weapons plugin, so nothing else moves the burst timer.
fn tick_the_torpedoes(app: &mut App, dt: f32) {
    let ship = find_ship_entity(app);
    let mut entity = app.world_mut().entity_mut(ship);
    let mut torpedoes = entity
        .get_mut::<crate::console::weapons::TorpedoSystemResource>()
        .expect("the cruiser carries a torpedo system");
    let mut n = 0_u32;
    torpedoes
        .0
        .tick(dt, &std::collections::HashMap::new(), &mut || {
            n += 1;
            format!("burst-{n}")
        });
}

/// AC4's other half, and the one the test above structurally cannot see.
///
/// That fixture writes `in_flight` directly, so its salvo is airborne all at
/// once and the count it asserts on can only fall. A real salvo can fall to
/// zero and come back: a burst launch puts one round per tube in the air and
/// leaves the rest pending on `burst_interval_secs` (0.35 s), and the
/// airborne rounds can resolve inside that gap. They do — the cruiser enters
/// the phase with thrust cut and the target closing, and an instrumented
/// `combat_test` run measured a salvo's first pair away at t=172.10 and both
/// resolved by t=172.33.
///
/// What made that a bug rather than a curiosity is what else is true on the
/// launch tick: firing empties the tubes, so `tubes_full` is already false
/// and the salvo-spent resume is armed with `torpedoes_in_flight` the only
/// conjunct still holding it shut. Counting the airborne half alone released
/// the hull mid-salvo, and the owed rounds then left the tubes in `orbit`
/// with the bow swinging away — measured at `|bearing| = 0.230` rad and
/// `in_arc = 0`, i.e. thrown outside the tubes' own 24-degree cone. Counting
/// the owed rounds holds the bow on and puts them away in arc.
#[test]
fn a_pending_burst_holds_the_cruiser_bow_on_between_its_own_rounds() {
    let (mut app, _uuid, bogey) = opportunity_app();
    let facing = bogey_facing_index(&mut app, bogey);
    set_bogey_arc_online(&mut app, bogey, facing, false);
    tick_twice(&mut app);
    assert_eq!(steering_state(&mut app), "torpedo_run");

    // A real salvo, through the launcher's own call.
    let (airborne, owed) = launch_a_real_salvo(&mut app);
    assert_eq!(airborne, 2, "one round per tube leaves immediately");
    assert!(
        owed > 0,
        "precondition: the hull must OWE rounds, or there is no burst to be \
         released in the middle of"
    );
    assert!(
        !tubes_are_full(&mut app),
        "precondition: firing empties the tubes, so the salvo-spent resume is \
         already armed and `torpedoes_in_flight` is the only conjunct holding \
         the phase"
    );

    // The airborne pair resolves before the burst timer elapses — the
    // measured case, and the one that used to release the hull.
    set_torpedoes_in_flight(&mut app, 0);
    assert_eq!(
        torpedoes_pending(&mut app),
        owed,
        "precondition: nothing is in the air and the burst still owes every \
         round it was scheduled with"
    );
    fly_for(&mut app, 1.0);
    assert_eq!(
        steering_state(&mut app),
        "torpedo_run",
        "a hull that has committed rounds to a burst must hold its bow on \
         until they have actually left the tubes — releasing here fires the \
         back half of its own salvo out of arc"
    );
    assert_eq!(
        engines_state(&mut app),
        "torpedo_run",
        "the travel axis reads the same fact and must reach the same conclusion"
    );
    assert!(pass_surface(&mut app).torpedo_bearing);

    // The timer elapses and the owed rounds actually leave the tubes.
    tick_the_torpedoes(&mut app, 0.4);
    assert_eq!(torpedoes_pending(&mut app), 0, "the burst has paid out");
    assert_eq!(
        torpedoes_in_flight(&mut app),
        owed as usize,
        "...and every owed round is now airborne"
    );
    fly_for(&mut app, 1.0);
    assert_eq!(
        steering_state(&mut app),
        "torpedo_run",
        "still committed — the same rounds, now counted on the other side of \
         the ledger"
    );

    // ...and those resolve too. Anti-vacuity: nothing owed and nothing
    // airborne must end the phase, or this test only asserts it never ends.
    set_torpedoes_in_flight(&mut app, 0);
    tick_twice(&mut app);
    assert_eq!(
        steering_state(&mut app),
        "orbit",
        "with the WHOLE salvo resolved the phase is over"
    );
    assert!(pass_surface(&mut app).combat_orbit);
}

/// The phase is BOUNDED against a target that can never close the window.
///
/// `target_facing_shield_down` reads `1.0` for a target that resolves but
/// carries no `[shields]` at all — a station, a probe, any hull authored
/// without the block — and it reads it for as long as that target lives,
/// correctly: there is genuinely no arc in the way and there never will be.
/// (Asteroids are a different case and a safe one: they resolve to no
/// transform-carrying row here, so the fact reads `0.0` and the phase is
/// never entered.)
///
/// So the window-closed exit can never fire against such a target, and with
/// `target_valid` the only other way out the cruiser would hold its bow on a
/// station, thrust cut, until one of them died. The bound is the salvo-spent
/// exit, drawn on the hull's own armament: it fires whatever the target does.
#[test]
fn a_shieldless_target_does_not_trap_the_cruiser_bow_on() {
    let (mut app, _uuid, bogey) = opportunity_app();

    // Turn the bogey into a resolvable target with NO shield system, leaving
    // everything else — position, uuid, the snapshot row — exactly as the
    // orbit fixture set it. Only the bogey's component goes: this is a
    // statement about what the TARGET carries, not about the cruiser.
    app.world_mut()
        .entity_mut(bogey)
        .remove::<crate::server_app::ShipShields>();
    tick_twice(&mut app);

    // The opportunity opens, and it opens permanently: nothing about this
    // target will ever make `target_facing_shield_down` read zero.
    assert_eq!(
        steering_state(&mut app),
        "torpedo_run",
        "an unshielded target is a real opportunity — every arc is 'down'"
    );
    fly_for(&mut app, 5.0);
    assert_eq!(
        steering_state(&mut app),
        "torpedo_run",
        "and with a salvo loaded and nothing in the air the hull stays committed"
    );

    // The salvo goes. Emptying the tubes is what `handle_fire_torpedo` does
    // to them, and this fixture's schedule does not run it.
    empty_the_tubes(&mut app);
    set_torpedoes_in_flight(&mut app, 2);
    fly_for(&mut app, 1.0);
    assert_eq!(
        steering_state(&mut app),
        "torpedo_run",
        "rounds in the air still hold the hull, exactly as against a shielded \
         target"
    );

    // ...and the rounds resolve. There is still no shield to come back, so
    // this release can only be the armament bound.
    set_torpedoes_in_flight(&mut app, 0);
    fly_for(&mut app, 2.0);
    assert_eq!(
        steering_state(&mut app),
        "orbit",
        "the cruiser must go back to raking a shieldless target once its salvo \
         is spent — with the window-closed exit unable to fire against a target \
         that has no arc to raise, this is the ONLY bound on the phase, and \
         without it the hull holds its nose on a station for the rest of the run"
    );
    assert_eq!(
        engines_state(&mut app),
        "orbit",
        "the travel axis is bounded by the same reading, not by Steering's state"
    );
    let pass = pass_surface(&mut app);
    assert!(
        pass.combat_orbit && !pass.torpedo_bearing,
        "and the published leg is the ring again, with thrust restored"
    );

    // It stays out: the reload is 9 seconds a round, so the phase cannot
    // immediately re-open and chatter the ship between the two states.
    fly_for(&mut app, 4.0);
    assert_eq!(
        steering_state(&mut app),
        "orbit",
        "and it stays on the ring while it reloads rather than chattering"
    );
}

/// AC1/AC4, the tracking half: the bow hold follows a target that KEEPS
/// MOVING, because the facing solution is re-derived from its live position
/// every tick.
///
/// This is the property that separates `hold_torpedo_bearing` from the
/// frozen-heading escape leg, and it is the whole reason a fixed forward
/// tube can be aimed at all.
#[test]
fn the_bow_hold_tracks_a_moving_target() {
    let (mut app, uuid, bogey) = opportunity_app();
    let facing = bogey_facing_index(&mut app, bogey);
    set_bogey_arc_online(&mut app, bogey, facing, false);
    // Settle onto the bow-on solution first, so what follows is the hold
    // reacting to the target rather than the hull still swinging round.
    fly_for(&mut app, 6.0);
    assert_eq!(steering_state(&mut app), "torpedo_run");
    let settled = bearing_to_bogey(&mut app).abs();
    assert!(
        settled < 0.2,
        "precondition: the hull must have come bow-on first, got {settled} rad"
    );

    // Jink the target well off the current bow line, on BOTH sides in turn:
    // each move must command a turn back toward it, and the sign must follow
    // the target rather than being a fixed bias.
    let pose = ship_pose(&mut app);
    for side in [1.0_f32, -1.0] {
        let offset = [
            pose.x + side * 60.0 + simmath::sin(pose.yaw) * 60.0,
            0.0,
            pose.z - simmath::cos(pose.yaw) * 60.0,
        ];
        set_bogey(&mut app, uuid, offset, 0.0, 0.0);
        app.world_mut()
            .entity_mut(bogey)
            .insert(Transform::from_xyz(offset[0], offset[1], offset[2]));
        // Moving the bogey moves which of ITS arcs faces the cruiser, so the
        // arc knocked offline before the loop is no longer the one the guard
        // reads. Re-derive and re-knock, or the opportunity closes the moment
        // the target jinks: `target_facing_shield_down` goes to 0, the phase
        // aborts back to `orbit`, and both assertions below are then
        // satisfied by `hold_combat_orbit` — a bow-hold test that never
        // exercises the bow hold. (Measured: without this the `side = -1`
        // iteration ran entirely in `orbit`.)
        let moved_facing = bogey_facing_index(&mut app, bogey);
        set_bogey_arc_online(&mut app, bogey, moved_facing, false);
        tick_twice(&mut app);

        // The tripwire for exactly that: everything below is about the HOLD,
        // so the hold has to still be the resolved state.
        assert_eq!(
            steering_state(&mut app),
            "torpedo_run",
            "the bow hold must survive the target jinking — a fallback to \
             `orbit` would satisfy the tracking assertions below off the \
             wrong verb"
        );

        let bearing = crate::ai::target_relative_motion(
            {
                let p = ship_pose(&mut app);
                [p.x, p.y, p.z]
            },
            ship_pose(&mut app).yaw,
            0.0,
            offset,
            Some(0.0),
            0.0,
        )
        .bearing_rad;
        let steering = get_steering_input(&mut app);
        assert!(
            steering * bearing > 0.0,
            "the hold must turn TOWARD the target's live position (bearing \
             {bearing} rad, commanded steering {steering})"
        );
        // ...and following it actually closes the angle.
        let before = bearing.abs();
        fly_for(&mut app, 4.0);
        let after = crate::ai::target_relative_motion(
            {
                let p = ship_pose(&mut app);
                [p.x, p.y, p.z]
            },
            ship_pose(&mut app).yaw,
            0.0,
            offset,
            Some(0.0),
            0.0,
        )
        .bearing_rad
        .abs();
        assert!(
            after < before,
            "tracking must close the angle on a moved target ({before} → {after})"
        );
    }
}

/// AC6, and the reason the phase is a genuinely distinct STATE rather than a
/// flag on the orbit: coming back re-enters `orbit`, and entering an orbiting
/// state is what makes the host re-draw the circulation direction from the
/// seeded key.
///
/// A flag layered on the existing state would leave `hold_combat_orbit`
/// resolved throughout, so nothing would ever be re-entered and the cruiser
/// would circle the same way for the whole engagement — which is precisely
/// what a seeded per-entry choice exists to avoid.
///
/// Asserted over a spread of seeds rather than one: a re-draw legitimately
/// lands on the same side about half the time, so "it changed for THIS seed"
/// is not the property. "It can change at all" is.
#[test]
fn resuming_the_orbit_after_a_torpedo_run_redraws_the_circulation() {
    fn round_trip(seed: u64) -> (f32, f32) {
        let (mut app, _uuid, bogey) = opportunity_app();
        app.insert_resource(crate::sim_rng::SimRng::new(
            seed,
            crate::sim_rng::SeedSource::Cli,
        ));
        let entity = find_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(entity)
            .insert(crate::entities::spawner::EntityUuid(
                uuid::Uuid::from_u128(0x9111_2222_3333_4444_5555_6666_7777_8888).to_string(),
            ));
        // Re-enter the ring once under this seed so the "before" reading is
        // a real draw rather than the hull's authored declaration.
        let facing = bogey_facing_index(&mut app, bogey);
        set_bogey_arc_online(&mut app, bogey, facing, false);
        tick_twice(&mut app);
        set_bogey_arc_online(&mut app, bogey, facing, true);
        tick_twice(&mut app);
        assert_eq!(steering_state(&mut app), "orbit");
        let first = pass_surface(&mut app).orbit_direction;

        // ...and again.
        let facing = bogey_facing_index(&mut app, bogey);
        set_bogey_arc_online(&mut app, bogey, facing, false);
        tick_twice(&mut app);
        assert_eq!(steering_state(&mut app), "torpedo_run");
        set_bogey_arc_online(&mut app, bogey, facing, true);
        tick_twice(&mut app);
        assert_eq!(steering_state(&mut app), "orbit");
        (first, pass_surface(&mut app).orbit_direction)
    }

    // Reproducible for a given seed — the property a replayed run depends on.
    assert_eq!(round_trip(31), round_trip(31));

    // ...and genuinely re-drawn: over a spread of seeds, at least one round
    // trip comes back circling the other way. A flag on the orbit state, or
    // a host that only drew on the FIRST entry, could never produce this.
    let flipped = (1_u64..=12).any(|seed| {
        let (before, after) = round_trip(seed);
        before != after
    });
    assert!(
        flipped,
        "resuming the orbit after a torpedo run must re-draw the circulation \
         direction, but it came back the same way for every seed tried"
    );
}

/// "Decline rather than invent", on the bow hold's own scalar.
///
/// `torpedo_bearing_speed` is authored `0.0` on this hull, which is exactly
/// the value an unauthored param would be mistaken for — so the gate is over
/// the NAME. A hull that omits it must fly its ordinary leg rather than
/// coasting to a halt in front of an enemy on a number nobody chose.
///
/// The shipped hull holds its bow at this exact point (asserted above), so
/// nothing here passes for want of getting that far.
#[test]
fn a_hull_omitting_the_bow_hold_throttle_declines_the_whole_arm() {
    for omitted in TORPEDO_BEARING_PARAMS {
        let (mut app, _uuid, bogey) = opportunity_app_omitting(&[omitted]);
        let facing = bogey_facing_index(&mut app, bogey);
        set_bogey_arc_online(&mut app, bogey, facing, false);
        tick_twice(&mut app);

        // The machine still enters the state — the verb parses and resolves
        // with or without the param. What must not happen is the HOST flying
        // a leg it has no throttle for.
        assert_eq!(
            steering_state(&mut app),
            "torpedo_run",
            "omitting `{omitted}` must not change which state is entered"
        );
        let pass = pass_surface(&mut app);
        assert!(
            !pass.torpedo_bearing,
            "omitting `{omitted}` must decline the bow-hold arm outright"
        );
        assert_eq!(
            pass.torpedo_bearing_speed, 0.0,
            "the whole arm declines together, not part of it"
        );

        // And it stays declined.
        fly_for(&mut app, 3.0);
        assert!(
            !pass_surface(&mut app).torpedo_bearing,
            "omitting `{omitted}` must keep declining the arm"
        );
    }
}

// ── The Harrow Battleship artillery position (issue #792) ────────────────
//
// Same posture as the cruiser block above: these drive the SHIPPED hull's
// authored policies through a real ticking app, so they fail on the content
// as well as on the code, and every assertion is about something observable
// — an admitted actuator input, the published pass surface, the committed
// policy state, or the ship's own flown range.
//
// The ships here are allowed to FLY rather than being posed each tick,
// because every claim below is a claim about what a position does over time:
// "holds station" is only observable as a range that stops changing, and
// "pivots onto a lead" is only observable as a bearing that converges on one.

fn warhawk_hull() -> crate::entities::config::EntityConfig {
    crate::entities::config::EntityConfig::from_toml(
        crate::entities::include_resolve::resolve_from_disk(
            "assets/entities/ship_harrow_warhawk.toml",
        )
        .expect("ship_harrow_warhawk must resolve")
        .toml
        .as_str(),
    )
    .expect("the shipped battleship hull must parse")
}

/// The battleship's authored Steering `param`s, so expectations below are
/// arithmetic on named values rather than magic numbers.
fn warhawk_steering_param(name: &str) -> f32 {
    warhawk_hull()
        .helm_console
        .as_ref()
        .and_then(|hc| hc.steering_ai.as_ref())
        .and_then(|ai| ai.param.get(name).copied())
        .unwrap_or_else(|| panic!("the shipped battleship must author `{name}`"))
}

/// The bolt whose flight speed the artillery hold leads by — the hull's
/// longest-reaching blaster bank, resolved exactly as the host resolves it.
fn warhawk_artillery_bank() -> crate::entities::config::BlasterBankConfig {
    let cfg = warhawk_hull();
    let wc = cfg
        .weapons_console
        .as_ref()
        .expect("the battleship declares [weapons_console]");
    wc.blaster_banks
        .iter()
        .max_by(|a, b| a.range.total_cmp(&b.range))
        .expect("the battleship carries an artillery bank")
        .clone()
}

/// A ship carrying the shipped battleship's two authored policy machines, its
/// physics envelope, and its artillery bank — the same components
/// `entities::spawner` would attach — hunting a single named bogey, with the
/// named STEERING `param`s optionally stripped from the hull before its
/// policy is built (the partially-authored hull AGENTS.md #11 says must
/// decline rather than invent).
///
/// The bank is attached because the LEAD SPEED is a host reading of it. A
/// fixture without one would publish a zero lead speed, the predictive
/// solution would silently degrade to "aim at where it is", and the aim test
/// below would pass by measuring the wrong thing.
///
/// The battleship authors no boost drive and no boost doctrine, so nothing
/// boost-shaped is inserted here either: the fixture mirrors the hull.
///
/// The IMPULSE drive, by contrast, is attached — and it is attached because
/// leaving it out was how #792's blocking defect hid. `entities::spawner`
/// gives an `ImpulseConfigResource` to every hull that declares a
/// `[helm_console]`, and the impulse autopilot in `integrate_ship_physics`
/// hard-overrides commanded throttle with `thrust = 1.0`. A fixture that
/// omitted the drive measured this doctrine in a world without the one
/// component capable of discarding it, so "holds station" could pass here
/// while the shipped hull sailed straight through its own gun line. The three
/// pieces below are the spawner's, verbatim in shape: the per-hull drive
/// config off `[helm_console]`, the authored `[helm_console.impulse_ai]`
/// policy (falling back to the canonical unconditional permit exactly as the
/// spawner does, so a hull that stopped authoring one is measured on the
/// default it would really get), and a `BehaviourSection` — `ai_helm_impulse`
/// reads `use_impulse` off the doctrine entry matching the top objective, so
/// without one the drive is unreachable and the fixture is back to lying.
///
/// That doctrine entry is deliberately in the SCENARIO shape rather than the
/// hull's own: a bare Destroy with no `use_impulse`, which is what
/// `assets/worlds/duel.toml` and `combat_test.toml`'s wave 8 hand this hull
/// when they replace its doctrine list wholesale, and which
/// `effective_use_impulse()` resolves to TRUE. It is the permissive case, so
/// anything that holds here holds for the hull's own doctrine too.
///
/// Each omitted name must actually be present to begin with, so this cannot
/// quietly pass by "removing" a param the hull renamed out from under it.
fn artillery_app_omitting(bogey_pos: [f32; 3], omit: &[&str]) -> (App, uuid::Uuid) {
    let mut app = test_app();
    let cfg = warhawk_hull();
    let mut hc = cfg
        .helm_console
        .clone()
        .expect("hull declares [helm_console]");
    for name in omit {
        hc.steering_ai
            .as_mut()
            .expect("hull declares [helm_console.steering_ai]")
            .param
            .remove(*name)
            .unwrap_or_else(|| panic!("the shipped hull must author `{name}` to omit it"));
    }
    let banks: Vec<crate::weapons::blaster::BlasterSystem> = cfg
        .weapons_console
        .as_ref()
        .expect("hull declares [weapons_console]")
        .blaster_banks
        .iter()
        .map(|b| crate::weapons::blaster::BlasterSystem::new(b.to_runtime()))
        .collect();
    let ship = find_ship_entity(&mut app);
    app.world_mut().entity_mut(ship).insert((
        crate::ship_plugin::ShipPhysicsConfigResource(crate::ship::physics::ShipPhysicsConfig {
            max_speed: hc.max_speed,
            max_reverse_speed: hc.max_reverse_speed,
            acceleration: hc.acceleration,
            deceleration: hc.deceleration,
            max_yaw_rate: hc.max_yaw_rate,
            ..crate::ship::physics::ShipPhysicsConfig::new()
        }),
        crate::console::weapons::BlasterSystemResource(banks),
        ImpulseConfigResource {
            charge_duration: hc.impulse_charge_duration,
            speed_multiplier: hc.impulse_speed_multiplier,
            acceleration_multiplier: hc.impulse_acceleration_multiplier,
            engage_distance: hc.impulse_engage_distance,
            cancel_distance: hc.impulse_cancel_distance,
            steering_multiplier: cfg
                .helm_capability
                .as_ref()
                .map(|cap| cap.impulse.steering_multiplier)
                .unwrap_or(crate::ship::impulse::IMPULSE_STEERING_MULTIPLIER_DEFAULT),
        },
    ));
    // Override Engines/Steering/Impulse in the ship's keyed
    // `FineSystemAiPolicies` map, MERGING into the shipped defaults `test_app`
    // attached (issue #1209). The impulse entry is present for the reason the
    // doc comment gives — leaving the impulse drive out was how #792 hid — it
    // just lands in the map now rather than in its own component.
    for (system_id, policy) in [
        (
            crate::ship::system_registry::helm_thrust_system_id(),
            hc.engines_ai.as_ref().unwrap().to_policy().unwrap(),
        ),
        (
            crate::ship::system_registry::helm_steering_system_id(),
            hc.steering_ai.as_ref().unwrap().to_policy().unwrap(),
        ),
        (
            crate::ship::system_registry::helm_impulse_system_id(),
            hc.impulse_ai
                .as_ref()
                .expect("the hull authors `[helm_console.impulse_ai]`")
                .to_policy()
                .expect("authored impulse policy decodes"),
        ),
    ] {
        set_fine_policy(&mut app, ship, system_id, policy);
    }
    let objective = destroy_scored_objective(BOGEY, 80.0);
    // The scenario-shaped doctrine entry the drive's `use_impulse` gate reads
    // — id-matched to the objective above, because that is how the two meet in
    // production. `use_impulse` is left unauthored on purpose (see the doc
    // comment): that is the permissive default every shipped scenario hands
    // this hull.
    set_behaviour_section(
        &mut app,
        crate::entities::config::BehaviourConfig {
            doctrine: vec![crate::entities::config::DoctrineObjective {
                id: objective.id.clone(),
                directive_kind: Some("Destroy".into()),
                base_priority: 80.0,
                target_speed: 0.9,
                maintain_range: 25.0,
                ..Default::default()
            }],
            ..Default::default()
        },
    );
    set_ship_blackboard_objectives(&mut app, vec![objective]);
    set_helm_control_source(&mut app, ControlSource::Ai);
    let uuid = uuid::Uuid::new_v4();
    set_bogey(&mut app, uuid, bogey_pos, 0.0, 0.0);
    // The Tactical lock `ai_target_selection` would have published. The helm's
    // travel axes resolve their target from the Destroy directive's own name
    // and so never needed it, but `ai_helm_impulse` resolves through the lock
    // alone — the last of the three things whose absence made this fixture a
    // world the impulse drive could not act in.
    set_ship_combat_lock(&mut app, uuid);
    leave_the_defensive_leg(&mut app, "acquire");
    (app, uuid)
}

/// Put the battleship at `start_range` from the bogey at the origin, pointed
/// straight at it and coasting inbound at its doctrine throttle — the pose
/// the approach would have delivered it in.
fn run_to_artillery_omitting(start_range: f32, omit: &[&str]) -> (App, uuid::Uuid) {
    let (mut app, uuid) = artillery_app_omitting(ORBIT_BOGEY, omit);
    let speed = warhawk_hull().helm_console.as_ref().unwrap().max_speed;
    // Down `+Z` from the bogey at the origin, facing `-Z` — which is straight
    // at it, since ship forward is `(sin yaw, -cos yaw)`.
    place_ship(&mut app, 0.0, start_range, 0.0, speed);
    // Two ticks: the first publishes the pass surface, the second is the
    // first planner pass that consumes it (see `HelmPassSurface`).
    tick_twice(&mut app);
    (app, uuid)
}

fn run_to_artillery(start_range: f32) -> (App, uuid::Uuid) {
    run_to_artillery_omitting(start_range, &[])
}

/// AC1/AC2: the range band is TWO thresholds, and the gap between them is
/// hysteresis rather than slack.
///
/// Four readings, and each one is a different claim:
///
/// * beyond the outer edge the hull is repositioning, not holding;
/// * crossing the outer edge INWARD does not stop it — the run-in continues
///   through the band, which is the half a single threshold cannot express;
/// * reaching the inner edge stops it;
/// * and once holding, drifting back out past the inner edge does NOT restart
///   it. Only clearing the OUTER edge does.
///
/// The first reading is also the anti-trap for an unseeded fact: before the
/// travel axes were seeded a `fact(range_to_target)` guard validated at load
/// and read false for ever. That this test can distinguish "far" from "near"
/// at all is the proof the guard actually gates.
#[test]
fn the_artillery_band_is_two_thresholds_with_hysteresis_between_them() {
    let max = warhawk_steering_param(MAX_ARTILLERY_RANGE_PARAM);
    let hold = warhawk_steering_param(ARTILLERY_HOLD_RANGE_PARAM);
    assert!(
        hold < max,
        "precondition: the band must have a gap, or every reading below is \
         the same reading"
    );

    // Beyond the outer edge: closing.
    let (mut app, uuid) = run_to_artillery(max * 1.5);
    assert_eq!(
        steering_state(&mut app),
        "reposition",
        "a target beyond the artillery envelope must be closed on — if this \
         reads `acquire` the travel axis is seeing empty facts"
    );
    assert_eq!(
        engines_state(&mut app),
        "reposition",
        "Engines runs its OWN copy of the machine and must reach the same leg \
         from the same facts, not by reading Steering's state"
    );
    assert!(
        !pass_surface(&mut app).artillery_hold,
        "and the host must not publish the hold leg for a ship still closing"
    );

    // INSIDE the outer edge but outside the inner one: still closing. This is
    // the reading that fails if the two thresholds are collapsed into one.
    let between = (max + hold) * 0.5;
    set_ship_physics(
        &mut app,
        ShipPhysics {
            z: between,
            ..Default::default()
        },
    );
    set_bogey(&mut app, uuid, ORBIT_BOGEY, 0.0, 0.0);
    tick_twice(&mut app);
    assert_eq!(
        steering_state(&mut app),
        "reposition",
        "inside `max_artillery_range` but outside `artillery_hold_range` the \
         run-in must continue: the ENTRY threshold is the inner one"
    );

    // The inner edge stops it.
    let (mut app, uuid) = run_to_artillery(hold * 0.99);
    assert_eq!(
        steering_state(&mut app),
        "hold",
        "reaching the inner edge must take up the firing position"
    );
    assert_eq!(engines_state(&mut app), "hold");

    // ...and drifting back out past the INNER edge does not restart it.
    let between = (max + hold) * 0.5;
    set_ship_physics(
        &mut app,
        ShipPhysics {
            z: between,
            ..Default::default()
        },
    );
    set_bogey(&mut app, uuid, ORBIT_BOGEY, 0.0, 0.0);
    tick_twice(&mut app);
    assert_eq!(
        steering_state(&mut app),
        "hold",
        "the EXIT threshold is the outer one — a hull that left the moment it \
         drifted past the entry threshold would chatter across the band"
    );

    // Only clearing the outer edge does.
    set_ship_physics(
        &mut app,
        ShipPhysics {
            z: max * 1.2,
            ..Default::default()
        },
    );
    set_bogey(&mut app, uuid, ORBIT_BOGEY, 0.0, 0.0);
    tick_twice(&mut app);
    assert_eq!(
        steering_state(&mut app),
        "reposition",
        "beyond `max_artillery_range` the hull must start repositioning again"
    );
    assert_eq!(engines_state(&mut app), "reposition");
}

/// AC3, the translational half: inside the band the hull commands the authored
/// hold throttle, actually comes to a stop, and STAYS at the range it stopped
/// at.
#[test]
fn the_firing_position_holds_station_rather_than_travelling() {
    let hold = warhawk_steering_param(ARTILLERY_HOLD_RANGE_PARAM);
    let (mut app, _uuid) = run_to_artillery(hold * 0.99);
    assert_eq!(steering_state(&mut app), "hold");

    let pass = pass_surface(&mut app);
    assert!(pass.active, "the battleship must be flying an authored leg");
    assert!(pass.artillery_hold, "and that leg is the artillery hold");
    assert!(
        !pass.combat_orbit
            && !pass.recover
            && !pass.reengage
            && !pass.escape
            && !pass.torpedo_bearing,
        "the artillery hold is its own leg — it must not masquerade as a ring \
         it never flies nor as the bow hold, which leads by nothing"
    );
    assert_eq!(
        pass.artillery_hold_speed,
        warhawk_steering_param(ARTILLERY_HOLD_SPEED_PARAM),
        "the hold flies the hull's OWN authored throttle"
    );

    let entry_speed = ship_pose(&mut app).forward_speed;
    assert!(
        entry_speed > 0.0,
        "precondition: the battleship must have been moving, or 'holds \
         station' is unobservable"
    );
    // One more tick before reading the throttle: the surface asserted above
    // was published at the END of this tick, and the planner consumes the
    // PREVIOUS tick's surface (see `HelmPassSurface`'s one-tick offset).
    tick(&mut app);
    assert_eq!(
        get_thrust_input(&mut app),
        warhawk_steering_param(ARTILLERY_HOLD_SPEED_PARAM),
        "the commanded throttle is the authored hold fraction"
    );

    // It genuinely stops, and then genuinely stays.
    fly_for(&mut app, 12.0);
    assert_eq!(steering_state(&mut app), "hold");
    let settled = ship_pose(&mut app).forward_speed;
    assert!(
        settled.abs() < 0.1,
        "the hull must come to rest in its firing position, got {settled}"
    );
    let settled_range = range_to_bogey(&mut app);
    fly_for(&mut app, 12.0);
    assert_eq!(steering_state(&mut app), "hold");
    assert!(
        (range_to_bogey(&mut app) - settled_range).abs() < 0.5,
        "and the range must stop changing: {} vs {settled_range}",
        range_to_bogey(&mut app)
    );
}

/// AC2/AC3 against the drive that used to discard them: a run-in that starts
/// OUTSIDE the artillery envelope must end inside the band.
///
/// Every other test in this block poses the hull at or near its holding
/// radius and measures what it does from there. That skips the one geometry
/// where the impulse drive engages — the autopilot only lights up beyond
/// `impulse_engage_distance` (200 by parse default) with the bow on the
/// target, which is precisely the pose an artillery run-in arrives in. From
/// there it holds `thrust = 1.0` until the target is inside
/// `impulse_cancel_distance` (40), overriding the `SetThrust{0.0}` the hold
/// commands the whole way down. The hull entered `hold` at 180, said stop,
/// and did not stop.
///
/// So this flies the approach rather than posing it, and asserts the
/// stopping point — which is where the defect is legible as a number: 180 if
/// the doctrine is flown, ~40 if the drive is flying the hull instead. The
/// idle phase is asserted alongside it because it names the CAUSE rather than
/// the symptom, and would still fail if a future change re-permitted the
/// channel while some other brake happened to stop the ship in the band.
#[test]
fn the_run_in_from_outside_the_envelope_stops_inside_the_band() {
    let max = warhawk_steering_param(MAX_ARTILLERY_RANGE_PARAM);
    let hold = warhawk_steering_param(ARTILLERY_HOLD_RANGE_PARAM);

    // Well beyond the envelope, bow on the target, at cruise — the pose the
    // impulse autopilot engages from.
    let (mut app, _uuid) = run_to_artillery(max * 1.5);
    assert_eq!(
        steering_state(&mut app),
        "reposition",
        "precondition: a target beyond the envelope must start a run-in"
    );

    // Long enough to cover the run-in (120 units at 9 units/s) several times
    // over, so this measures where the hull SETTLES rather than where it
    // happened to be. The drive is sampled every tick rather than read at the
    // end, because it CANCELS itself on arrival — a final reading of `Idle`
    // is what both the healthy hull and the broken one show.
    let mut drive_ever_engaged = None;
    for _ in 0..((60.0 / HELM_AI_MAX_DT_SECS).ceil() as usize) {
        tick(&mut app);
        let phase = get_ship_impulse(&mut app).phase;
        if phase != crate::ship::impulse::ImpulsePhase::Idle && drive_ever_engaged.is_none() {
            drive_ever_engaged = Some((phase, range_to_bogey(&mut app)));
        }
    }

    assert_eq!(
        steering_state(&mut app),
        "hold",
        "the run-in must end in the firing position"
    );
    assert_eq!(
        drive_ever_engaged, None,
        "the battleship must never engage its impulse drive: the autopilot \
         replaces commanded throttle with full thrust, so an engaged drive \
         discards the hold's `SetThrust{{0.0}}` for as long as it runs"
    );
    let settled = ship_pose(&mut app).forward_speed;
    assert!(
        settled.abs() < 0.1,
        "and it must actually be stopped, got {settled}"
    );
    let range = range_to_bogey(&mut app);
    assert!(
        (range - hold).abs() < hold * 0.1,
        "the hull must come to rest at its authored holding radius ({hold}); \
         got {range}. A reading near the drive's `impulse_cancel_distance` is \
         the autopilot having flown the hull through its own gun line"
    );
}

/// AC5: the battleship holds rather than retreating when the player closes.
///
/// Stated as the property that would break if a `maintain_range`-style
/// standoff crept into the doctrine: the target is walked from the inner edge
/// of the band all the way to point-blank, and at every step the hull must
/// still be in `hold`, must still command the authored throttle, and must
/// never command REVERSE.
#[test]
fn the_battleship_holds_rather_than_retreating_when_the_target_closes() {
    let hold = warhawk_steering_param(ARTILLERY_HOLD_RANGE_PARAM);
    let (mut app, uuid) = run_to_artillery(hold * 0.99);
    assert_eq!(steering_state(&mut app), "hold");
    fly_for(&mut app, 12.0);
    let station = range_to_bogey(&mut app);

    // Walk the bogey in. Each step is a real approach, not a teleport to the
    // end: a doctrine that only backed off below some inner limit would slip
    // through a single point-blank reading.
    for fraction in [0.75_f32, 0.5, 0.25, 0.05] {
        let pose = ship_pose(&mut app);
        let closer = [pose.x * (1.0 - fraction), 0.0, pose.z * (1.0 - fraction)];
        set_bogey(&mut app, uuid, closer, 0.0, 0.0);
        fly_for(&mut app, 2.0);

        assert_eq!(
            steering_state(&mut app),
            "hold",
            "a closing target must not push the battleship out of its firing \
             position (target at {fraction} of the station range)"
        );
        assert_eq!(
            get_thrust_input(&mut app),
            warhawk_steering_param(ARTILLERY_HOLD_SPEED_PARAM),
            "and the commanded throttle must stay the authored hold fraction"
        );
        assert!(
            get_thrust_input(&mut app) >= 0.0,
            "a battleship that answered a charge with reverse thrust would be \
             kiting, which is the manoeuvre this hull deliberately does not fly"
        );
    }

    // The hull has not moved off its station through any of that.
    assert!(
        (range_to_bogey(&mut app) - station).abs() < station,
        "sanity: the hull's own position must not have run away"
    );
    let pose = ship_pose(&mut app);
    assert!(
        pose.forward_speed.abs() < 0.1,
        "and it is still stationary, got {}",
        pose.forward_speed
    );
}

/// AC3's facing half, and the whole reason this leg is not
/// `hold_torpedo_bearing`: the bow goes onto the PREDICTED INTERCEPT, not
/// onto the target.
///
/// The two are only distinguishable against a target with real crossing
/// velocity, so the bogey is given one — and the expected lead is derived
/// from the SAME `predict_intercept_heading` the bolt is launched on, at the
/// authored bank speed, so this asserts agreement between the aim and the
/// ballistics rather than agreement with a number written here.
///
/// The control is the assertion that carries it: the settled bow bearing to
/// where the target IS must be non-zero and must sit on the side the target
/// is travelling towards. A leg that merely tracked would settle at zero.
#[test]
fn the_firing_position_pivots_onto_a_predicted_intercept_not_onto_the_target() {
    let hold = warhawk_steering_param(ARTILLERY_HOLD_RANGE_PARAM);
    let bank = warhawk_artillery_bank();
    // Crossing square across the line of sight, fast enough that the lead is
    // a real angle rather than float noise.
    let crossing_speed = 24.0_f32;
    let crossing_yaw = std::f32::consts::FRAC_PI_2; // heading +X

    let (mut app, uuid) = run_to_artillery(hold * 0.99);
    assert_eq!(steering_state(&mut app), "hold");
    set_bogey(&mut app, uuid, ORBIT_BOGEY, crossing_yaw, crossing_speed);
    // Let the hull settle onto the solution. The bogey's snapshot position is
    // held still deliberately: a target that both moved and was led would mix
    // "did the bow follow it" into a test about "did the bow lead it".
    fly_for(&mut app, 25.0);
    assert_eq!(
        steering_state(&mut app),
        "hold",
        "everything below is about the HOLD, so the hold must still be the \
         resolved state"
    );

    let pose = ship_pose(&mut app);
    // The heading the gun itself would fire on, from this pose.
    let expected = crate::weapons::blaster::predict_intercept_heading(
        pose.x,
        pose.z,
        ORBIT_BOGEY[0],
        ORBIT_BOGEY[2],
        simmath::sin(crossing_yaw) * crossing_speed,
        -simmath::cos(crossing_yaw) * crossing_speed,
        bank.projectile_speed,
        pose.yaw,
        0.0,
    );
    let lead_error = crate::ai::target_relative_motion(
        [pose.x, pose.y, pose.z],
        pose.yaw,
        0.0,
        [
            pose.x + simmath::sin(expected) * 100.0,
            0.0,
            pose.z - simmath::cos(expected) * 100.0,
        ],
        Some(0.0),
        0.0,
    )
    .bearing_rad;
    assert!(
        lead_error.abs() < warhawk_steering_param(TRACKING_DEADBAND_PARAM) * 2.0,
        "the bow must settle on the heading the gun fires ({expected} rad); \
         residual bearing error {lead_error} rad"
    );

    // ...and that heading is NOT the bearing to the target. This is the
    // control: a leg that tracked the live position would leave this at zero.
    let live_error = bearing_to_bogey(&mut app);
    assert!(
        live_error.abs() > warhawk_steering_param(TRACKING_DEADBAND_PARAM) * 3.0,
        "a predictive solution must be OFF the target's live bearing against a \
         crossing target — got {live_error} rad, which is what a plain \
         tracking leg would produce"
    );
    assert!(
        live_error < 0.0,
        "and the lead must fall on the side the target is travelling TOWARDS: \
         the bogey runs +X, so the aim point is to STARBOARD of it and the \
         target's own live bearing therefore sits to port of the bow. A \
         positive reading would be a lead aimed behind a crossing target; got \
         {live_error}"
    );
}

/// "Decline rather than invent", over the artillery arm's whole requirement.
///
/// All THREE params, one at a time. The throttle is the one an authored value
/// cannot distinguish from an omission — this hull authors `0.0` — and the two
/// ranges are here because a gate over only part of an arm's requirement is
/// the exact mistake #788's and #790's reviews each caught once.
///
/// The shipped hull holds its position at this exact point (asserted above),
/// so nothing here passes for want of getting that far.
#[test]
fn a_hull_omitting_an_artillery_param_declines_the_whole_arm() {
    let hold = warhawk_steering_param(ARTILLERY_HOLD_RANGE_PARAM);
    for omitted in ARTILLERY_PARAMS {
        let (mut app, _uuid) = run_to_artillery_omitting(hold * 0.99, &[omitted]);

        // Omitting the THROTTLE changes nothing about the machine — the verb
        // parses and resolves either way, and the state is reached exactly as
        // the shipped hull reaches it. That is what makes this the sharp case:
        // the leg is selected, the host simply refuses to fly it. (The two
        // range params are different: their names appear in the machine's own
        // guards, so removing one strands the machine in `acquire`. It is
        // rejected outright at content load — see
        // `harrow_warhawk_cannot_drop_a_guard_referenced_artillery_range` —
        // and this loop covers what the host does if one ever reached it.)
        if *omitted == ARTILLERY_HOLD_SPEED_PARAM {
            assert_eq!(
                steering_state(&mut app),
                "hold",
                "omitting `{omitted}` must not change which state is entered"
            );
        }
        let pass = pass_surface(&mut app);
        assert!(
            !pass.artillery_hold,
            "omitting `{omitted}` must decline the artillery arm outright"
        );
        assert_eq!(
            pass.artillery_hold_speed, 0.0,
            "the whole arm declines together, not part of it"
        );

        // And it stays declined.
        fly_for(&mut app, 3.0);
        assert!(
            !pass_surface(&mut app).artillery_hold,
            "omitting `{omitted}` must keep declining the arm"
        );
    }
}

/// AC6: hazard avoidance may RELOCATE the firing position, and may not turn
/// the battleship into something that orbits or kites.
///
/// The relocation is measured on the LATERAL axis, and it has to be: a hull
/// that has come to rest projects no forward path, so `avoidance_steering` —
/// which is the layer that bends a *travelling* leg — is zero by construction
/// for a ship holding station (`avoidance_steering_is_zero_when_stationary`
/// pins that directly). `ai_helm_lateral_thrust` is the layer that actually
/// nudges a stopped hull sideways off its held point, it runs as its own fine
/// system, and it never touches Engines or Steering — which is exactly what
/// makes the detour "limited". The pure planner's own additive fold is pinned
/// where it is testable, on the pure function
/// (`artillery_position_folds_avoidance_onto_the_intercept_facing`).
///
/// The other half is the absence: the machine must never leave `hold` for any
/// of it. A detour that became a state would be a manoeuvre with an exit to
/// get stuck in, and re-entering the hold afterwards would be a second
/// commitment nobody authored.
#[test]
fn a_hazard_relocates_the_firing_position_without_ending_the_hold() {
    let hold = warhawk_steering_param(ARTILLERY_HOLD_RANGE_PARAM);
    let (mut app, uuid) = run_to_artillery(hold * 0.99);
    fly_for(&mut app, 12.0);
    assert_eq!(steering_state(&mut app), "hold");
    assert_eq!(
        lateral_intent(&mut app),
        0.0,
        "precondition: an unobstructed gun line commands no dodge, or the \
         reading below is not the hazard's doing"
    );
    let station = range_to_bogey(&mut app);

    // Drop a large, dangerous obstacle right alongside the hull, off to one
    // side so the repulsion is a genuine lateral push rather than a head-on
    // one. The bogey is republished with it: the snapshot is the whole world.
    let pose = ship_pose(&mut app);
    let hazard = uuid::Uuid::new_v4();
    app.insert_resource(crate::ai::server::WorldSnapshot {
        entities: vec![
            crate::ai::AiWorldEntity {
                uuid,
                name: Some(BOGEY.into()),
                position: ORBIT_BOGEY,
                yaw: Some(0.0),
                forward_speed: 0.0,
                radius: 3.0,
                size_rating: 3.0,
                movable: true,
                dangerous: true,
                ..Default::default()
            },
            crate::ai::AiWorldEntity {
                uuid: hazard,
                name: Some("rock".into()),
                position: [pose.x + 5.0, 0.0, pose.z - 5.0],
                yaw: None,
                forward_speed: 0.0,
                radius: 9.0,
                size_rating: 9.0,
                movable: false,
                dangerous: true,
                ..Default::default()
            },
        ],
    });
    tick_twice(&mut app);

    assert_eq!(
        steering_state(&mut app),
        "hold",
        "a hazard must NOT be a leg: the doctrine authors no hazard-guarded \
         transition, so the detour stays a stateless bend"
    );
    assert!(
        pass_surface(&mut app).artillery_hold,
        "and the published leg is still the artillery hold"
    );
    assert!(
        lateral_intent(&mut app) < 0.0,
        "the hull must be pushed sideways AWAY from the obstacle (it sits to \
         starboard, so the dodge is to port); got {}",
        lateral_intent(&mut app)
    );
    assert_eq!(
        get_thrust_input(&mut app),
        warhawk_steering_param(ARTILLERY_HOLD_SPEED_PARAM),
        "and the dodge must not become a translation the ENGINES fly: the \
         hold's throttle is untouched by hazards"
    );

    // Clearing the hazard evaporates the detour: no state was entered, so
    // there is none to leave, and the gun line is where it was.
    set_bogey(&mut app, uuid, ORBIT_BOGEY, 0.0, 0.0);
    fly_for(&mut app, 3.0);
    assert_eq!(steering_state(&mut app), "hold");
    assert!(pass_surface(&mut app).artillery_hold);
    assert_eq!(
        lateral_intent(&mut app),
        0.0,
        "the dodge must evaporate with the hazard rather than persisting as \
         state"
    );
    assert!(
        (range_to_bogey(&mut app) - station).abs() < 5.0,
        "and the firing position is RELOCATED, not abandoned: {} vs {station}",
        range_to_bogey(&mut app)
    );
}

// ── The broadside readings and the two levers they drive (issue #929) ────────
//
// Unit-level, deliberately: the sweeps that justify the TUNING live in
// `assets/entities/alliance_cruiser.toml`'s rationale and in `headless_runner`,
// and a seeded fleet action is a poor instrument for "does the minimum-over-
// banks fold pick the minimum". These pin the MECHANISM, so a future retune that
// breaks the fold fails here with a sentence instead of failing there with a
// win-rate.

/// A shield system with `n` evenly-spaced arcs, every arc at `hp`.
fn shields_with(n: usize, hp: i32) -> crate::ship::shields::ShipShields {
    let mut sys =
        crate::weapons::shield::ShieldSystem::new(&crate::weapons::shield::ShieldConfig {
            num_facings: n,
            max_hp: 100,
            regen_per_sec: 0.0,
            offline_duration: 10.0,
        });
    for facing in &mut sys.facings {
        facing.hp = hp;
    }
    crate::ship::shields::ShipShields(sys, 1.0)
}

/// One phaser bank: centreline `facing_deg`, `fire_arc_deg` wide, `dps` damage.
fn bank(
    id: &str,
    facing_deg: f32,
    fire_arc_deg: f32,
    dps: f32,
) -> crate::entities::config::PhaserBankConfig {
    crate::entities::config::PhaserBankConfig {
        id: id.into(),
        facing_deg,
        fire_arc_deg,
        auto_arc_deg: fire_arc_deg,
        beam_damage_per_sec: dps,
        ..Default::default()
    }
}

fn phasers(
    banks: Vec<crate::entities::config::PhaserBankConfig>,
) -> crate::entities::config::PhaserCombatConfig {
    crate::entities::config::PhaserCombatConfig { banks }
}

/// Seed the broadside facts for a target on `bearing_deg`, and hand back the bag.
fn broadside_facts_at(
    bearing_deg: f32,
    shields: Option<&crate::ship::shields::ShipShields>,
    phasers: Option<&crate::entities::config::PhaserCombatConfig>,
) -> crate::world::flags::AiFacts {
    let mut facts = crate::world::flags::AiFacts::new();
    facts.set(BEARING_TO_TARGET_FACT, bearing_deg.to_radians() as f64);
    seed_own_broadside_facts(&mut facts, shields, phasers);
    facts
}

/// THE FOLD IS A MINIMUM OVER BEARING BANKS, and the test is the case that made
/// it one.
///
/// Two 270-degree arcs on the centreline (the shipped cruiser's geometry) overlap
/// so heavily that their union has no gap: the BEST-bearing bank's margin never
/// falls below 45 degrees at any bearing at all, so a threshold under 45 could
/// never fire and one above it would fire always. Yet beams end constantly,
/// because `tick_beams_prepare` asks about the ONE bank that is burning. The
/// reading a helm can act on is how soon the FIRST bearing bank loses it.
///
/// At 50 degrees off the bow the fore bank (facing 0, ±135) has 85 degrees in
/// hand and the aft bank (facing 180, ±135) has 5. The fact must say 5.
#[test]
fn the_bank_arc_margin_is_the_soonest_bearing_bank_not_the_best_one() {
    let cfg = phasers(vec![
        bank("fore", 0.0, 270.0, 10.0),
        bank("aft", 180.0, 270.0, 10.0),
    ]);
    let facts = broadside_facts_at(50.0, None, Some(&cfg));
    let margin = facts
        .get(OWN_BANK_ARC_MARGIN_DEG_FACT)
        .expect("a bearing and an armed bank must produce a margin");

    assert!(
        (margin - 5.0).abs() < 1e-3,
        "the fold must report the SOONEST bank to drop the target (aft, 5 deg in \
         hand), not the best-placed one (fore, 85). Reading the maximum is what \
         the first draft did, and on this geometry it is a constant: read {margin}"
    );
}

/// The fallback when the minimum is over an empty set.
///
/// A single narrow bank with the target behind it: nothing bears, so there is no
/// "soonest to drop" to report. The honest remaining reading is the nearest arc's
/// NEGATIVE margin — how far outside it the target sits — and the sign is what
/// keeps an authored `<= margin` guard from reading a blind spot as "on the edge,
/// slow down".
#[test]
fn an_arc_margin_with_nothing_bearing_reports_how_far_outside_the_nearest_arc_is() {
    let cfg = phasers(vec![bank("fore", 0.0, 90.0, 10.0)]);
    let facts = broadside_facts_at(120.0, None, Some(&cfg));
    let margin = facts
        .get(OWN_BANK_ARC_MARGIN_DEG_FACT)
        .expect("a bearing and an armed bank must produce a margin");

    assert!(
        (margin - -75.0).abs() < 1e-3,
        "with a 90-degree bank on the bow and the target 120 degrees off it, the \
         target is 75 degrees outside the only arc there is: expected -75, read \
         {margin}"
    );
}

/// The TERMINATION gate is `fire_arc_deg`, and this pins the field the fold
/// reads rather than trusting the comment.
///
/// `validate_phaser_banks` requires `auto_arc_deg <= fire_arc_deg`, so the two
/// can only differ in one direction: auto is the tighter. A hull that differs
/// (`alliance_battleship`, fire 270 / auto 180) must be measured against the
/// edge a BURNING beam dies at — `tick_beams_prepare` ends the cycle on
/// `!in_arc(.., fire_arc_deg)` — not the edge a new one would refuse to start at.
#[test]
fn the_bank_arc_margin_measures_the_fire_arc_and_not_the_auto_arc() {
    let mut wide_fire = bank("fore", 0.0, 270.0, 10.0);
    wide_fire.auto_arc_deg = 180.0;
    let facts = broadside_facts_at(100.0, None, Some(&phasers(vec![wide_fire])));
    let margin = facts
        .get(OWN_BANK_ARC_MARGIN_DEG_FACT)
        .expect("a bearing and an armed bank must produce a margin");

    assert!(
        (margin - 35.0).abs() < 1e-3,
        "fire 270 gives 135 degrees of half-arc and the target is 100 off the bow, \
         so 35 remain before the beam is cut. Reading `auto_arc_deg` instead would \
         say -10 and have the helm give up a ring it still has a burning beam on: \
         read {margin}"
    );
}

/// A bank authored to hurt nothing is not a reason to give away the ring.
#[test]
fn a_zero_damage_bank_does_not_pull_the_arc_margin_down() {
    let armed = phasers(vec![bank("fore", 0.0, 270.0, 10.0)]);
    let with_dud = phasers(vec![
        bank("fore", 0.0, 270.0, 10.0),
        bank("dud", 180.0, 270.0, 0.0),
    ]);

    assert_eq!(
        broadside_facts_at(50.0, None, Some(&armed)).get(OWN_BANK_ARC_MARGIN_DEG_FACT),
        broadside_facts_at(50.0, None, Some(&with_dud)).get(OWN_BANK_ARC_MARGIN_DEG_FACT),
        "an unarmed bank must not move the margin — its edge costs the hull nothing"
    );
}

/// The whole ladder is seeded, and the indexed reading agrees with the
/// facing-relative one.
///
/// The agreement is the point: the latch trips on `own_facing_shield_hp` and
/// restores on `own_shield_hp_arc_<index>`, so if the index the seeder reports
/// did not name the arc the HP came from, the latch would protect one arc and
/// wait on another.
#[test]
fn every_arc_gets_its_own_reading_and_the_index_names_the_one_facing_the_target() {
    let mut shields = shields_with(4, 100);
    // Beat down whichever arc looks aft, so the two readings can disagree.
    let aft = shields.0.facing_index_for_bearing(std::f32::consts::PI);
    shields.0.facings[aft].hp = 7;

    let facts = broadside_facts_at(180.0, Some(&shields), None);
    let index = facts
        .get(OWN_FACING_SHIELD_INDEX_FACT)
        .expect("a bearing and a shield system must produce an index");
    let facing_hp = facts
        .get(OWN_FACING_SHIELD_HP_FACT)
        .expect("...and an HP reading");

    assert_eq!(index as usize, aft, "the index must name the arc astern");
    assert_eq!(facing_hp, 7.0, "and that arc is the beaten one");
    assert_eq!(
        facts.get(&format!("{OWN_SHIELD_HP_ARC_PREFIX}{aft}")),
        Some(7.0),
        "the indexed family must agree with the facing-relative reading, or the \
         latch protects one arc and waits on another"
    );
    for i in 0..shields.0.facings.len() {
        assert!(
            facts
                .get(&format!("{OWN_SHIELD_HP_ARC_PREFIX}{i}"))
                .is_some(),
            "every arc needs a reading — a latch that named arc {i} and found no \
             fact would never clear"
        );
    }
}

/// The absent-fact contract, both halves.
///
/// No target is not "my shields are fine" and no shields is not "my shields are
/// down"; both are "no reading", and an authored guard on an absent fact reads
/// false rather than picking a side.
#[test]
fn the_broadside_readings_are_absent_rather_than_zero_when_there_is_nothing_to_read() {
    let shields = shields_with(4, 100);
    let cfg = phasers(vec![bank("fore", 0.0, 270.0, 10.0)]);

    // No bearing at all: no target.
    let mut facts = crate::world::flags::AiFacts::new();
    seed_own_broadside_facts(&mut facts, Some(&shields), Some(&cfg));
    for name in [
        OWN_BANK_ARC_MARGIN_DEG_FACT,
        OWN_FACING_SHIELD_HP_FACT,
        OWN_FACING_SHIELD_INDEX_FACT,
    ] {
        assert_eq!(
            facts.get(name),
            None,
            "`{name}` must be absent with no target — which of my arcs faces \
             nothing is not a question with a numeric answer"
        );
    }
    assert_eq!(facts.get(&format!("{OWN_SHIELD_HP_ARC_PREFIX}0")), None);

    // A bearing, but a hull with no shields and no guns.
    let facts = broadside_facts_at(50.0, None, None);
    for name in [
        OWN_BANK_ARC_MARGIN_DEG_FACT,
        OWN_FACING_SHIELD_HP_FACT,
        OWN_FACING_SHIELD_INDEX_FACT,
    ] {
        assert_eq!(
            facts.get(name),
            None,
            "`{name}` must be absent for a hull that has no such component — a \
             doctrine reacting to its own absence is worse than one that declines"
        );
    }
}

/// F4's direction of agreement: run the REAL seeder and ask the registry about
/// every name it produced.
///
/// The registry's own drift test pins the catalogue against the per-host
/// descriptors, which keeps those two consistent with each other and says
/// nothing about the code. #929 shipped four seeded facts that no descriptor
/// declared, and both halves of that test stayed green; the only symptom was a
/// load error telling an author that `fact(own_facing_shield_hp)` named
/// something no helm seeder produces, at the moment they wrote the one guard
/// that would have used it.
#[test]
fn every_broadside_fact_the_seeder_writes_is_one_this_host_declares() {
    let shields = shields_with(4, 100);
    let cfg = phasers(vec![
        bank("fore", 0.0, 270.0, 10.0),
        bank("aft", 180.0, 270.0, 10.0),
    ]);
    let facts = broadside_facts_at(50.0, Some(&shields), Some(&cfg));

    let host = &crate::entities::ai_flag_hosts::HELM_STEERING;
    let mut checked = 0;
    for (name, _) in facts.iter() {
        if name == BEARING_TO_TARGET_FACT {
            continue; // seeded by the fixture, not by the seeder under test
        }
        assert!(
            host.seeds_fact(crate::entities::ai_flag_hosts::FactScope::Ship, name),
            "`seed_own_broadside_facts` writes `{name}`, which the helm host's \
             registry does not declare. An authored `fact({name})` guard would be \
             REJECTED at load with a message saying no helm seeder produces it — \
             which is false, and points the author away from the fix"
        );
        checked += 1;
    }
    assert!(
        checked >= 7,
        "this fixture should exercise the margin, the facing HP, the facing index \
         and one reading per arc: only {checked} names were checked, so the \
         agreement above is weaker than it looks"
    );
}

/// A latch under test, with the shipped hull's thresholds.
fn flip_params(flip: f64, restore: f64, dwell: f64) -> crate::world::flags::AiParams {
    let mut params = crate::world::flags::AiParams::default();
    params.set(WEAK_SHIELD_FLIP_HP_PARAM, flip);
    params.set(WEAK_SHIELD_RESTORE_HP_PARAM, restore);
    params.set(WEAK_SHIELD_FLIP_DWELL_SECS_PARAM, dwell);
    params
}

/// Facts as the latch sees them: the arc facing the target, its index, and the
/// per-arc ladder.
fn latch_facts(facing_index: usize, arc_hp: &[f64]) -> crate::world::flags::AiFacts {
    let mut facts = crate::world::flags::AiFacts::new();
    facts.set(OWN_FACING_SHIELD_HP_FACT, arc_hp[facing_index]);
    facts.set(OWN_FACING_SHIELD_INDEX_FACT, facing_index as f64);
    for (i, hp) in arc_hp.iter().enumerate() {
        facts.set(&format!("{OWN_SHIELD_HP_ARC_PREFIX}{i}"), *hp);
    }
    facts
}

/// THE LIMIT CYCLE, as a regression test.
///
/// This is the exact sequence the first design failed on. Arc 2 is beaten to 10
/// and faces the target, so the latch trips. The ring then mirrors — which is an
/// instruction to bring a DIFFERENT arc round — and within a few degrees of turn
/// arc 0, untouched at 100, is the one facing the enemy. The old latch read
/// "whichever arc faces the target", saw 100, cleared, un-mirrored, and started
/// over; measured across 28 seeds the result was byte-identical to no flip on
/// nine of them.
///
/// The latch must hold, because arc 2 — the arc it named — is still at 10.
#[test]
fn the_flip_latch_holds_when_the_swing_it_ordered_brings_a_healthy_arc_around() {
    let params = flip_params(25.0, 60.0, 4.0);
    let mut memory = crate::world::flags::AiPolicyMemory::default();

    // Arc 2 beaten and facing the enemy: trip.
    fold_broadside_flip_latch(
        &params,
        &mut memory,
        &latch_facts(2, &[100.0, 100.0, 10.0, 100.0]),
        0.0,
    );
    assert_eq!(
        memory.get(BROADSIDE_FLIP_MEMORY),
        Some(1.0),
        "an arc at 10 against a floor of 25 must trip the flip"
    );
    assert_eq!(
        memory.get(BROADSIDE_FLIP_ARC_MEMORY),
        Some(2.0),
        "and the latch must record WHICH arc, or it cannot tell recovery from rotation"
    );

    // The swing the flip ordered brings the healthy arc 0 into the bearing, well
    // past the dwell so only identity can be what holds the latch.
    fold_broadside_flip_latch(
        &params,
        &mut memory,
        &latch_facts(0, &[100.0, 100.0, 10.0, 100.0]),
        30.0,
    );
    assert_eq!(
        memory.get(BROADSIDE_FLIP_MEMORY),
        Some(1.0),
        "the arc now facing the target is a healthy one BECAUSE the flip worked. \
         Clearing on that reading is a feedback loop: the hull un-mirrors, the \
         beaten arc comes back round, and it flips again — for ever"
    );
}

/// The dwell, isolated: the named arc recovers immediately and the latch must
/// still hold until the authored time has been served.
///
/// The case it guards is not hypothetical — an arc crosses the restore threshold
/// on regeneration or a focus change while the swing is still in flight, and
/// unflipping there costs the whole manoeuvre and buys nothing.
#[test]
fn the_flip_latch_serves_its_authored_dwell_before_it_will_look_at_recovery() {
    let params = flip_params(25.0, 60.0, 4.0);
    let mut memory = crate::world::flags::AiPolicyMemory::default();
    fold_broadside_flip_latch(
        &params,
        &mut memory,
        &latch_facts(2, &[100.0, 100.0, 10.0, 100.0]),
        100.0,
    );
    assert_eq!(memory.get(BROADSIDE_FLIP_MEMORY), Some(1.0));

    // Fully recovered, one tick later.
    let recovered = latch_facts(2, &[100.0, 100.0, 100.0, 100.0]);
    fold_broadside_flip_latch(&params, &mut memory, &recovered, 103.9);
    assert_eq!(
        memory.get(BROADSIDE_FLIP_MEMORY),
        Some(1.0),
        "3.9 s into a 4 s dwell the latch must hold even on a fully recovered arc"
    );

    fold_broadside_flip_latch(&params, &mut memory, &recovered, 104.0);
    assert_eq!(
        memory.get(BROADSIDE_FLIP_MEMORY),
        Some(0.0),
        "and at the authored dwell exactly, with the named arc above the restore \
         reading, it clears"
    );
}

/// The hysteresis band, read off the arc the latch named.
#[test]
fn the_flip_latch_clears_on_the_restore_reading_and_not_the_flip_one() {
    let params = flip_params(25.0, 60.0, 4.0);
    let mut memory = crate::world::flags::AiPolicyMemory::default();
    fold_broadside_flip_latch(
        &params,
        &mut memory,
        &latch_facts(2, &[100.0, 100.0, 10.0, 100.0]),
        0.0,
    );

    // Back above the FLIP floor but inside the deadband, dwell long served.
    fold_broadside_flip_latch(
        &params,
        &mut memory,
        &latch_facts(2, &[100.0, 100.0, 59.0, 100.0]),
        50.0,
    );
    assert_eq!(
        memory.get(BROADSIDE_FLIP_MEMORY),
        Some(1.0),
        "59 is above the flip floor of 25 and below the restore reading of 60: \
         that is the deadband, and a latch that cleared there would reverse the \
         ring every few ticks while the arc regenerates through it"
    );

    fold_broadside_flip_latch(
        &params,
        &mut memory,
        &latch_facts(2, &[100.0, 100.0, 60.0, 100.0]),
        51.0,
    );
    assert_eq!(memory.get(BROADSIDE_FLIP_MEMORY), Some(0.0));
}

/// An arc the ladder no longer reports cannot be read as recovered.
///
/// A hull that lost facings mid-run leaves the latch naming an index with no
/// fact behind it. "Cannot confirm recovery" is not "recovered", so it holds.
#[test]
fn the_flip_latch_holds_when_the_arc_it_named_has_no_reading_left() {
    let params = flip_params(25.0, 60.0, 4.0);
    let mut memory = crate::world::flags::AiPolicyMemory::default();
    fold_broadside_flip_latch(
        &params,
        &mut memory,
        &latch_facts(2, &[100.0, 100.0, 10.0, 100.0]),
        0.0,
    );

    // Only two arcs left, and neither is arc 2.
    fold_broadside_flip_latch(&params, &mut memory, &latch_facts(0, &[100.0, 100.0]), 60.0);
    assert_eq!(
        memory.get(BROADSIDE_FLIP_MEMORY),
        Some(1.0),
        "the named arc has no reading, so recovery is unconfirmed and the latch holds"
    );
}

/// A hull that authors none of the set never has the slots written at all, so
/// the ring it flies is bit-for-bit the ring it flew before this issue.
#[test]
fn an_unauthored_hull_never_acquires_a_flip_slot() {
    let mut memory = crate::world::flags::AiPolicyMemory::default();
    let facts = latch_facts(2, &[0.0, 0.0, 0.0, 0.0]);

    fold_broadside_flip_latch(
        &crate::world::flags::AiParams::default(),
        &mut memory,
        &facts,
        0.0,
    );
    assert_eq!(memory.get(BROADSIDE_FLIP_MEMORY), None);

    // And a HALF-authored one is refused too, rather than half-applied. (The
    // load validator makes this unreachable from TOML; the guard is here because
    // the runtime must not depend on the validator having run.)
    let mut half = crate::world::flags::AiParams::default();
    half.set(WEAK_SHIELD_FLIP_HP_PARAM, 25.0);
    fold_broadside_flip_latch(&half, &mut memory, &facts, 0.0);
    assert_eq!(
        memory.get(BROADSIDE_FLIP_MEMORY),
        None,
        "a floor with no restore reading and no dwell is not a latch"
    );
}
