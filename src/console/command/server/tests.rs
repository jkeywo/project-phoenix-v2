//! Unit tests for the Command console server wiring (issue #1107).
//!
//! These drive the plugin's systems directly over a hand-built world (post
//! admission), so they pin the CONTENT rules — what a stance order does once it
//! is admitted, how the alert switch and the AI operator behave, and what the
//! console blackboard reports. The admission AUTHORISATION of the order and the
//! wire round-trip are pinned in `command_admission`/`core::codec`.

use super::*;
use crate::core::messages::{
    AdmittedCommand, AdmittedCommands, StationId, SystemControlPayload, SystemId,
};
use crate::ship::control_source::ControlSource;
use crate::ship_plugin::{ShipConfigComponent, ShipSystemControlSources};

const KINDS: &[&str] = &["red_alert", "command", "phaser_bank"];

/// A captain, an AI-controllable proving Station ("tactical", owning a weapon
/// so `weapons_station()` resolves to it) with a full stance catalogue, and an
/// auxiliary Command station directing it.
fn command_config() -> ShipConfig {
    ShipConfig::from_toml(
        r#"
[[station]]
id = "captain"
name = "Captain"
description = "Command the bridge."
rank = "Cpt."

[[station.rating]]
name = "Std"
automated_systems = []

[[station]]
id = "tactical"
name = "Tactical"
description = "Weapons."
rank = "Ltn."

[[station.rating]]
name = "Std"
automated_systems = []

[[station.stance]]
id = "weapons-free"
kind = "standard"
high_alert = true
persist_behind_human = true
ai_engaged = true

[[station.stance]]
id = "hold"
kind = "standard"
high_alert = false

[[station.stance]]
id = "normal"
kind = "normal_alert_neutral"

[[station.stance]]
id = "high"
kind = "high_alert_neutral"
high_alert = true

[[station]]
id = "command"
name = "Command"
description = "Direct an AI station."
rank = "Cpt."
console = "gui/command-console.html"
auxiliary = true
human_seeking = true
host_order = ["captain"]
visiting_rating = "Std"
command_target = "tactical"

[[station.rating]]
name = "Std"
automated_systems = []

[[system]]
id = "red-alert"
kind = "red_alert"
station = "captain"

[[system]]
id = "phaser-fore"
kind = "phaser_bank"
station = "tactical"

[[system]]
id = "command"
kind = "command"
station = "command"
"#,
        KINDS,
    )
    .expect("command config parses")
}

fn tactical() -> StationId {
    StationId("tactical".into())
}

/// Spawn one Ship with the command config. `tactical_ai`/`command_ai` set the
/// initial control sources; everything else defaults.
fn spawn_ship(app: &mut App, tactical_ai: bool, command_ai: bool) -> Entity {
    let mut sources = ShipSystemControlSources::default();
    sources.0.set(
        SystemId("phaser-fore".into()),
        if tactical_ai {
            ControlSource::Ai
        } else {
            ControlSource::Human
        },
    );
    sources.0.set(
        crate::ship::system_registry::command_system_id(),
        if command_ai {
            ControlSource::Ai
        } else {
            ControlSource::Human
        },
    );
    app.world_mut()
        .spawn((
            crate::server_app::Ship,
            ShipConfigComponent(command_config()),
            sources,
            AdmittedCommands::default(),
            ShipStationStances::default(),
            LastDirectedControl::default(),
            crate::ship::state::ShipRedAlert::default(),
            crate::server_app::ShipSystemBlackboards::default(),
        ))
        .id()
}

/// Flip the directed weapons Station between AI and human control.
fn set_tactical_ai(app: &mut App, ship: Entity, ai: bool) {
    let mut ship_mut = app.world_mut().entity_mut(ship);
    let mut sources = ship_mut.get_mut::<ShipSystemControlSources>().unwrap();
    sources.0.set(
        SystemId("phaser-fore".into()),
        if ai {
            ControlSource::Ai
        } else {
            ControlSource::Human
        },
    );
}

/// Set the ship's red-alert flag.
fn set_red_alert(app: &mut App, ship: Entity, active: bool) {
    app.world_mut()
        .entity_mut(ship)
        .get_mut::<crate::ship::state::ShipRedAlert>()
        .unwrap()
        .0 = active;
}

/// Store a stance selection directly on the ship (as an admitted order would).
fn insert_stance(app: &mut App, ship: Entity, station: &str, stance: &str) {
    app.world_mut()
        .entity_mut(ship)
        .get_mut::<ShipStationStances>()
        .unwrap()
        .0
        .insert(StationId(station.into()), stance.into());
}

/// Build an objective-contributed stance config (issue #1110).
fn objective_stance(
    id: &str,
    kind: StanceKind,
    high_alert: bool,
    persist: bool,
) -> StationStanceConfig {
    StationStanceConfig {
        id: id.into(),
        label: String::new(),
        kind,
        high_alert,
        persist_behind_human: persist,
        ai_engaged: false,
    }
}

/// Insert the `ActiveObjectiveStances` projection lending `stance` to `station`
/// (as `project_active_objective_stances` would while its objective is active).
fn contribute_objective_stance(app: &mut App, station: &str, stance: StationStanceConfig) {
    app.world_mut()
        .insert_resource(ActiveObjectiveStances(vec![(
            StationId(station.into()),
            stance,
        )]));
}

/// Clear every objective contribution — what completion, failure or
/// invalidation of the objective produces on the next projection refresh.
fn clear_objective_stances(app: &mut App) {
    app.world_mut()
        .insert_resource(ActiveObjectiveStances::default());
}

/// The full published Command blackboard for `ship`.
fn command_bb(app: &App, ship: Entity) -> crate::core::messages::CommandBlackboard {
    let bbs = app
        .world()
        .entity(ship)
        .get::<crate::server_app::ShipSystemBlackboards>()
        .unwrap();
    let SystemBlackboard::Command(bb) = bbs
        .0
        .get(&SystemId(
            crate::ship::system_registry::COMMAND_SYSTEM_ID.to_string(),
        ))
        .expect("a command blackboard is published")
    else {
        panic!("expected a Command blackboard");
    };
    bb.clone()
}

/// Run the target-transition trigger once (issue #1108).
fn run_target_reconcile(app: &mut App) {
    app.world_mut()
        .run_system_cached(reconcile_directed_target_control)
        .unwrap();
}

/// Read the published `selected_stance` from the Command blackboard.
fn published_selected_stance(app: &mut App, ship: Entity) -> String {
    app.world_mut()
        .run_system_cached(publish_command_blackboard)
        .unwrap();
    let bbs = app
        .world()
        .entity(ship)
        .get::<crate::server_app::ShipSystemBlackboards>()
        .unwrap();
    let SystemBlackboard::Command(bb) = bbs
        .0
        .get(&SystemId(
            crate::ship::system_registry::COMMAND_SYSTEM_ID.to_string(),
        ))
        .expect("a command blackboard is published")
    else {
        panic!("expected a Command blackboard");
    };
    bb.selected_stance.clone()
}

fn set_admitted(app: &mut App, ship: Entity, station: &str, stance: &str) {
    let mut admitted = app.world_mut().entity_mut(ship);
    let mut cmds = admitted.get_mut::<AdmittedCommands>().unwrap();
    cmds.0.push(AdmittedCommand {
        target: crate::ship::system_registry::command_system_id(),
        payload: SystemControlPayload::SetStationStance {
            station: StationId(station.into()),
            stance: stance.into(),
        },
        response_token: None,
    });
}

fn stances(app: &App, ship: Entity) -> ShipStationStances {
    app.world()
        .entity(ship)
        .get::<ShipStationStances>()
        .unwrap()
        .clone()
}

/// An app carrying an empty `Sessions` resource, which `operate_command_ai`'s
/// emit path requires as a system param (an `ai:` token is authorised on
/// `operate_ai` alone, so the manager is never consulted, but the resource must
/// exist).
fn ai_app() -> App {
    let mut app = App::new();
    app.insert_resource(crate::lobby::Sessions(
        crate::lobby::session::SessionManager::new(),
    ));
    app
}

/// One AI Command decision tick through the REAL path: the decider emits an
/// admitted order, then the shared applier lands it. Driven directly (not
/// through `app.update()`), matching this module's convention — the cadence gate
/// itself is pinned in `ai::cadence`, and driving the systems by hand keeps
/// these tests about selection CONTENT.
fn run_command_ai_tick(app: &mut App) {
    app.world_mut()
        .run_system_cached(operate_command_ai)
        .unwrap();
    app.world_mut()
        .run_system_cached(handle_set_station_stance)
        .unwrap();
}

#[test]
fn ai_command_selects_the_engaged_stance_into_stored_stances_deterministically() {
    // AC1/AC2/AC5: an uncrewed Command seat (command_ai) directing an AI Tactical
    // station selects the authored `ai_engaged` stance at Red Alert, lands it in
    // ShipStationStances through the shared applier, and does so repeatably across
    // repeated fixed ticks.
    let mut app = ai_app();
    let ship = spawn_ship(&mut app, true, true); // tactical AI, Command AI
    set_red_alert(&mut app, ship, true);

    for _ in 0..6 {
        run_command_ai_tick(&mut app);
        assert_eq!(
            stances(&app, ship).0.get(&tactical()).map(String::as_str),
            Some("weapons-free"),
            "AI Command repeatably selects the authored engaged stance at Red Alert",
        );
    }
    // AC2/AC3: the landed id is one the catalogue authors.
    assert!(crate::ship::command_stance::is_selectable(
        &command_config().station(&tactical()).unwrap().stances,
        "weapons-free",
    ));
}

#[test]
fn ai_command_tracks_the_neutral_and_stores_nothing_off_alert() {
    // Off Red Alert the AI tracks the alert-neutral, which equals the stored
    // default, so nothing is emitted and the map stays empty — byte-identical to
    // a never-commanded hull.
    let mut app = ai_app();
    let ship = spawn_ship(&mut app, true, true); // tactical AI, Command AI
    set_red_alert(&mut app, ship, false);

    run_command_ai_tick(&mut app);
    assert!(
        stances(&app, ship).0.is_empty(),
        "an AI Command at normal alert tracks the neutral without storing it",
    );
    assert_eq!(published_selected_stance(&mut app, ship), "normal");
}

#[test]
fn ai_command_does_not_act_while_the_target_is_human_held() {
    // AC3 boundary: with a human at Tactical the AI Command makes no selection —
    // the same target-AI gate the applier enforces.
    let mut app = ai_app();
    let ship = spawn_ship(&mut app, false, true); // tactical HUMAN, Command AI
    set_red_alert(&mut app, ship, true);

    run_command_ai_tick(&mut app);
    assert!(
        stances(&app, ship).0.is_empty(),
        "AI Command does not direct a human-held target station",
    );
}

#[test]
fn ai_command_does_not_act_while_the_command_seat_is_human_held() {
    // AC1 boundary: the decider only runs for an uncrewed (AI) Command seat. With
    // a human hosting Command it makes no selection of its own.
    let mut app = ai_app();
    let ship = spawn_ship(&mut app, true, false); // tactical AI, Command HUMAN
    set_red_alert(&mut app, ship, true);

    run_command_ai_tick(&mut app);
    assert!(
        stances(&app, ship).0.is_empty(),
        "a human-hosted Command seat is not driven by the AI decider",
    );
}

#[test]
fn a_human_taking_command_sees_the_ai_intent_and_overrides_it() {
    // AC4: an AI-selected intent is visible to a later human host and can be
    // changed through the ordinary SetStationStance path.
    let mut app = ai_app();
    let ship = spawn_ship(&mut app, true, true); // tactical AI, Command AI
    set_red_alert(&mut app, ship, true);
    run_command_ai_tick(&mut app);
    assert_eq!(
        published_selected_stance(&mut app, ship),
        "weapons-free",
        "the AI-selected intent is the published stance in force",
    );

    // A human takes the Command seat and re-picks through the normal UI path.
    app.world_mut()
        .entity_mut(ship)
        .get_mut::<ShipSystemControlSources>()
        .unwrap()
        .0
        .set(
            crate::ship::system_registry::command_system_id(),
            ControlSource::Human,
        );
    set_admitted(&mut app, ship, "tactical", "hold");
    app.world_mut()
        .run_system_cached(handle_set_station_stance)
        .unwrap();
    assert_eq!(
        stances(&app, ship).0.get(&tactical()).map(String::as_str),
        Some("hold"),
        "a later human host overrides the AI-selected stance",
    );
}

#[test]
fn an_authored_stance_for_the_directed_ai_station_is_applied() {
    let mut app = App::new();
    let ship = spawn_ship(&mut app, true, false);
    set_admitted(&mut app, ship, "tactical", "weapons-free");
    app.world_mut()
        .run_system_cached(handle_set_station_stance)
        .unwrap();
    assert_eq!(
        stances(&app, ship).0.get(&tactical()).map(String::as_str),
        Some("weapons-free"),
    );
}

#[test]
fn an_unauthored_stance_is_rejected() {
    let mut app = App::new();
    let ship = spawn_ship(&mut app, true, false);
    set_admitted(&mut app, ship, "tactical", "invented-order");
    app.world_mut()
        .run_system_cached(handle_set_station_stance)
        .unwrap();
    assert!(
        stances(&app, ship).0.is_empty(),
        "Command invents no orders"
    );
}

#[test]
fn a_stance_is_refused_while_the_station_is_human_controlled() {
    // Criterion 2: Command applies a stance only while the target is AI.
    let mut app = App::new();
    let ship = spawn_ship(&mut app, false, false);
    set_admitted(&mut app, ship, "tactical", "weapons-free");
    app.world_mut()
        .run_system_cached(handle_set_station_stance)
        .unwrap();
    assert!(
        stances(&app, ship).0.is_empty(),
        "a human-held Station is off the Command board"
    );
}

#[test]
fn a_stance_for_an_undirected_station_is_refused() {
    let mut app = App::new();
    let ship = spawn_ship(&mut app, true, false);
    // Command directs "tactical", not "captain".
    set_admitted(&mut app, ship, "captain", "weapons-free");
    app.world_mut()
        .run_system_cached(handle_set_station_stance)
        .unwrap();
    assert!(stances(&app, ship).0.is_empty());
}

#[test]
fn changing_alert_switches_a_stored_neutral_but_not_a_standard_stance() {
    // Criterion 5. Start in the normal-alert neutral; raise the alert; the
    // stored neutral follows to the high-alert neutral.
    let mut app = App::new();
    let ship = spawn_ship(&mut app, true, false);
    app.world_mut()
        .entity_mut(ship)
        .get_mut::<ShipStationStances>()
        .unwrap()
        .0
        .insert(tactical(), "normal".into());

    app.world_mut()
        .entity_mut(ship)
        .get_mut::<crate::ship::state::ShipRedAlert>()
        .unwrap()
        .0 = true;
    app.world_mut()
        .run_system_cached(apply_alert_change_to_stances)
        .unwrap();
    assert_eq!(
        stances(&app, ship).0.get(&tactical()).map(String::as_str),
        Some("high"),
        "a stored neutral follows the alert to the other neutral"
    );

    // A standard stance is never overwritten by an alert change.
    app.world_mut()
        .entity_mut(ship)
        .get_mut::<ShipStationStances>()
        .unwrap()
        .0
        .insert(tactical(), "hold".into());
    app.world_mut()
        .entity_mut(ship)
        .get_mut::<crate::ship::state::ShipRedAlert>()
        .unwrap()
        .0 = false;
    app.world_mut()
        .run_system_cached(apply_alert_change_to_stances)
        .unwrap();
    assert_eq!(
        stances(&app, ship).0.get(&tactical()).map(String::as_str),
        Some("hold"),
        "an explicit standard stance survives an alert change"
    );
}

#[test]
fn a_persistent_stance_resumes_when_the_target_returns_to_ai() {
    // Criterion 3: a persist-behind-human standard order is carried across a
    // human's control of the directed Station and resumes intact on the
    // Human→AI edge.
    let mut app = App::new();
    let ship = spawn_ship(&mut app, false, false); // tactical human-held
    insert_stance(&mut app, ship, "tactical", "weapons-free"); // persist = true

    // First observation while human-held records state, fires no edge.
    run_target_reconcile(&mut app);
    assert_eq!(
        stances(&app, ship).0.get(&tactical()).map(String::as_str),
        Some("weapons-free"),
        "a dormant order is untouched while the target is human-held"
    );

    // Human releases the target → Human→AI edge → persistent order resumes.
    set_tactical_ai(&mut app, ship, true);
    run_target_reconcile(&mut app);
    assert_eq!(
        stances(&app, ship).0.get(&tactical()).map(String::as_str),
        Some("weapons-free"),
        "a persistent stance resumes when the human hands the target back to AI"
    );
}

#[test]
fn a_transient_stance_falls_back_to_neutral_when_the_target_returns_to_ai() {
    // Criterion 3: a non-persistent standard order does NOT resume behind the
    // handoff — it clears to the alert-neutral tracking default.
    let mut app = App::new();
    let ship = spawn_ship(&mut app, false, false); // tactical human-held
    insert_stance(&mut app, ship, "tactical", "hold"); // persist = false

    run_target_reconcile(&mut app); // record human-held
    set_tactical_ai(&mut app, ship, true);
    run_target_reconcile(&mut app); // Human→AI edge

    assert!(
        !stances(&app, ship).0.contains_key(&tactical()),
        "a transient stance clears to neutral tracking on the handoff"
    );
}

#[test]
fn a_transient_handoff_falls_back_to_the_alert_appropriate_neutral() {
    // Criterion 3 + 5: the transient fallback is BOTH neutrals — the current
    // alert level's one, observed through the published stance in force.
    for (red_alert, expected) in [(false, "normal"), (true, "high")] {
        let mut app = App::new();
        let ship = spawn_ship(&mut app, false, false); // tactical human-held
        set_red_alert(&mut app, ship, red_alert);
        insert_stance(&mut app, ship, "tactical", "hold");

        run_target_reconcile(&mut app);
        set_tactical_ai(&mut app, ship, true);
        run_target_reconcile(&mut app);

        assert!(!stances(&app, ship).0.contains_key(&tactical()));
        assert_eq!(
            published_selected_stance(&mut app, ship),
            expected,
            "a transient handoff falls back to the current alert's neutral"
        );
    }
}

#[test]
fn a_human_holding_the_target_sees_the_stored_intent_as_advice() {
    // Criterion 2: while the directed Station is human-held its holder keeps
    // full authority (a stance order no-ops, pinned elsewhere) and the current
    // Command intent stays visible as advice — the stored order is still the
    // published stance in force, which `withCommandAdvice` surfaces on the
    // target console.
    let mut app = App::new();
    let ship = spawn_ship(&mut app, false, false); // tactical human-held
    insert_stance(&mut app, ship, "tactical", "weapons-free");
    // A human at the target cannot be re-directed…
    set_admitted(&mut app, ship, "tactical", "hold");
    app.world_mut()
        .run_system_cached(handle_set_station_stance)
        .unwrap();
    assert_eq!(
        stances(&app, ship).0.get(&tactical()).map(String::as_str),
        Some("weapons-free"),
        "a human-held target retains full authority — no order lands"
    );
    // …but the intent remains readable as non-binding advice.
    assert_eq!(
        published_selected_stance(&mut app, ship),
        "weapons-free",
        "the current Command intent stays visible while the target is human-held"
    );
}

#[test]
fn an_invalid_stored_stance_falls_back_and_is_visibly_removed() {
    // Criterion 4: a stored id no longer in the authored catalogue is dropped,
    // the Station falls back to the alert-neutral, and the removal is visible on
    // the published blackboard.
    let mut app = App::new();
    let ship = spawn_ship(&mut app, true, false);
    insert_stance(&mut app, ship, "tactical", "objective-escort"); // never authored
    app.world_mut()
        .run_system_cached(reconcile_station_stances)
        .unwrap();
    assert!(
        !stances(&app, ship).0.contains_key(&tactical()),
        "an unauthored stored stance is reconciled away"
    );
    assert_eq!(
        published_selected_stance(&mut app, ship),
        "normal",
        "the directed Station falls back to the alert-neutral, visibly"
    );
}

#[test]
fn the_stance_override_feeds_the_weapons_posture_only_when_directed_and_ai() {
    // Criterion 4: the selection feeds the weapons AI host's alert posture.
    let config = command_config();
    let mut sources = ControlSourceResolver::default();
    sources.set(SystemId("phaser-fore".into()), ControlSource::Ai);
    let mut selections = ShipStationStances::default();

    // No selection → None → tracks red alert (byte-identical default).
    assert_eq!(
        weapons_station_stance_high_alert(Some(&selections), None, &config, &sources, false),
        None
    );

    // A human's "hold" stance forces stood-down even at red alert.
    selections.0.insert(tactical(), "hold".into());
    assert_eq!(
        weapons_station_stance_high_alert(Some(&selections), None, &config, &sources, true),
        Some(false),
        "a hold stance overrides the ship's own red alert for the fire gate"
    );

    // The same selection is ignored once a human holds the weapons Station.
    let mut human_sources = ControlSourceResolver::default();
    human_sources.set(SystemId("phaser-fore".into()), ControlSource::Human);
    assert_eq!(
        weapons_station_stance_high_alert(Some(&selections), None, &config, &human_sources, true),
        None,
        "Command does not direct a human-held weapons Station"
    );

    // Issue #1110: a SELECTED objective-contributed stance seeds its authored
    // posture for the fire gate exactly as a permanent one does — resolved
    // through the effective catalogue, not the permanent slice.
    let active = ActiveObjectiveStances(vec![(
        tactical(),
        objective_stance("objective-escort", StanceKind::Standard, true, false),
    )]);
    let mut objective_selection = ShipStationStances::default();
    objective_selection
        .0
        .insert(tactical(), "objective-escort".into());
    assert_eq!(
        weapons_station_stance_high_alert(
            Some(&objective_selection),
            Some(&active),
            &config,
            &sources,
            false,
        ),
        Some(true),
        "a selected objective stance forces its authored high-alert posture",
    );
}

#[test]
fn the_blackboard_lists_the_directed_station_and_its_stances() {
    // Criterion 2: the console shows the directed Station, its AI state and the
    // selectable stances (the persistent non-colour cue is derived from these).
    let mut app = App::new();
    let ship = spawn_ship(&mut app, true, false);
    app.world_mut()
        .run_system_cached(publish_command_blackboard)
        .unwrap();
    let bbs = app
        .world()
        .entity(ship)
        .get::<crate::server_app::ShipSystemBlackboards>()
        .unwrap();
    let SystemBlackboard::Command(bb) = bbs
        .0
        .get(&SystemId(
            crate::ship::system_registry::COMMAND_SYSTEM_ID.to_string(),
        ))
        .expect("a command blackboard is published")
    else {
        panic!("expected a Command blackboard");
    };
    assert_eq!(bb.directed_station, tactical());
    assert_eq!(bb.directed_station_name, "Tactical");
    assert!(bb.directed_station_ai);
    assert_eq!(bb.stances.len(), 4);
    // With nothing selected the console shows the alert level's neutral.
    assert_eq!(bb.selected_stance, "normal");
}

#[test]
fn the_blackboard_marks_the_station_off_the_board_when_human_held() {
    let mut app = App::new();
    let ship = spawn_ship(&mut app, false, false); // tactical human-held
    app.world_mut()
        .run_system_cached(publish_command_blackboard)
        .unwrap();
    let bbs = app
        .world()
        .entity(ship)
        .get::<crate::server_app::ShipSystemBlackboards>()
        .unwrap();
    let SystemBlackboard::Command(bb) = bbs
        .0
        .get(&SystemId(
            crate::ship::system_registry::COMMAND_SYSTEM_ID.to_string(),
        ))
        .unwrap()
    else {
        panic!("expected a Command blackboard");
    };
    assert!(
        !bb.directed_station_ai,
        "a human at the directed Station takes it off the Command board"
    );
}

/// Issue #1099 AC2: a direct claim relocates who OPERATES a Station's surface,
/// but must never reset the authoritative Station state stored off to the side
/// of hosting. A stored Command stance lives on the ship entity keyed by
/// `StationId`; the lobby claim path only mutates Sessions and control sources.
/// Prove the stance — and the canonical authoritative-state digest that folds
/// it (`sim_digest::world_digest` → `fold_station_stances_namespace`) — is
/// byte-unchanged when the directed Station is claimed by a human (its fine
/// System flips AI → Human, the whole observable authoritative effect of a
/// mid-game direct claim).
#[test]
fn a_direct_claim_of_the_directed_station_preserves_the_stored_stance_and_digest() {
    let mut app = App::new();
    // tactical AI (Command may direct it), Command human-operated.
    let ship = spawn_ship(&mut app, true, false);
    // A minted world id so the stance namespace folds this ship (the fold keys
    // on `EntityUuid`; an unminted hull would fold nothing here).
    app.world_mut()
        .entity_mut(ship)
        .insert(crate::entities::spawner::EntityUuid(
            crate::world_id::WorldId::new(crate::world_id::IdNamespace::Entity, 1, 1).render(),
        ));

    // A human Command operator lands a standing order while tactical is AI.
    set_admitted(&mut app, ship, "tactical", "weapons-free");
    app.world_mut()
        .run_system_cached(handle_set_station_stance)
        .unwrap();
    assert_eq!(
        stances(&app, ship).0.get(&tactical()).map(String::as_str),
        Some("weapons-free"),
    );

    let digest_before = crate::sim_digest::world_digest(app.world());

    // Direct claim: a player takes the Tactical seat, so its fine System flips
    // AI → Human. Nothing here touches ShipStationStances.
    {
        let mut ship_mut = app.world_mut().entity_mut(ship);
        let mut sources = ship_mut.get_mut::<ShipSystemControlSources>().unwrap();
        sources
            .0
            .set(SystemId("phaser-fore".into()), ControlSource::Human);
    }
    // Re-run the Command input systems exactly as a post-claim tick would: with
    // no new order admitted and Command still human, none of them may rewrite
    // the stored stance.
    app.world_mut()
        .run_system_cached(handle_set_station_stance)
        .unwrap();
    run_target_reconcile(&mut app);

    assert_eq!(
        stances(&app, ship).0.get(&tactical()).map(String::as_str),
        Some("weapons-free"),
        "a direct claim must not reset the stored Command stance",
    );
    assert_eq!(
        crate::sim_digest::world_digest(app.world()),
        digest_before,
        "a direct claim must not move the authoritative-state digest",
    );
}

// ── Objective-contributed stances (issue #1110) ────────────────────────────────

#[test]
fn an_active_objective_stance_is_exposed_to_human_and_the_shared_applier() {
    // AC2/AC3: while the objective is active its stance joins the console's
    // vocabulary WITHOUT mutating the permanent catalogue, and the shared order
    // applier — the seam BOTH a human order and the AI operator's emitted order
    // flow through — now admits it. Without the contribution it is an invented
    // order the applier rejects.
    let mut app = App::new();
    let ship = spawn_ship(&mut app, true, false); // tactical AI
    contribute_objective_stance(
        &mut app,
        "tactical",
        objective_stance("objective-escort", StanceKind::Standard, true, true),
    );

    // Human console lists the four permanent stances plus the one contribution.
    app.world_mut()
        .run_system_cached(publish_command_blackboard)
        .unwrap();
    let bb = command_bb(&app, ship);
    assert_eq!(bb.stances.len(), 5, "permanent four plus the contribution");
    assert!(
        bb.stances.iter().any(|s| s.id == "objective-escort"),
        "the objective stance is exposed while active",
    );
    // AC1: the permanent catalogue is untouched — it still authors only four.
    assert_eq!(
        command_config().station(&tactical()).unwrap().stances.len(),
        4,
        "the permanent catalogue is never mutated by a contribution",
    );

    // The shared applier admits an order for the contributed stance.
    set_admitted(&mut app, ship, "tactical", "objective-escort");
    app.world_mut()
        .run_system_cached(handle_set_station_stance)
        .unwrap();
    assert_eq!(
        stances(&app, ship).0.get(&tactical()).map(String::as_str),
        Some("objective-escort"),
        "an objective stance is selectable through the shared applier while active",
    );
}

#[test]
fn without_the_contribution_the_objective_stance_is_an_invented_order() {
    // The boundary of the test above: with no active contribution the same order
    // is refused, so exposure is gated on the objective (AC2), not standing.
    let mut app = App::new();
    let ship = spawn_ship(&mut app, true, false);
    set_admitted(&mut app, ship, "tactical", "objective-escort");
    app.world_mut()
        .run_system_cached(handle_set_station_stance)
        .unwrap();
    assert!(
        stances(&app, ship).0.is_empty(),
        "an objective stance is not selectable without its active contribution",
    );
}

#[test]
fn the_ai_operator_reads_the_effective_catalogue_and_stays_in_vocabulary() {
    // AC2/AC3 AI parity: the uncrewed Command seat decides against the SAME
    // effective catalogue the human sees. With a contribution present its pick is
    // still an authored member of that catalogue — never invented — proving the
    // AI reads the projection like every other consumer.
    let mut app = ai_app();
    let ship = spawn_ship(&mut app, true, true); // tactical AI, Command AI
    set_red_alert(&mut app, ship, true);
    contribute_objective_stance(
        &mut app,
        "tactical",
        objective_stance("objective-escort", StanceKind::Standard, true, false),
    );
    run_command_ai_tick(&mut app);
    let stored = stances(&app, ship).0.get(&tactical()).cloned();
    if let Some(id) = stored {
        let config = command_config();
        let permanent = &config.station(&tactical()).unwrap().stances;
        let effective = crate::ship::command_stance::effective_catalogue(
            permanent,
            &[objective_stance(
                "objective-escort",
                StanceKind::Standard,
                true,
                false,
            )],
        );
        assert!(
            crate::ship::command_stance::is_selectable(&effective, &id),
            "the AI operator only ever stores an effective-catalogue member; stored {id:?}",
        );
    }
}

#[test]
fn ending_the_objective_drops_a_selected_objective_stance_to_the_neutral() {
    // AC3/AC4: a selected objective stance whose objective completes, fails or is
    // invalidated (all three converge on "no longer contributed") is reconciled
    // away, and the directed Station falls back to the current alert's neutral —
    // visibly, on the published readout — at BOTH alert levels.
    for (red_alert, expected) in [(false, "normal"), (true, "high")] {
        let mut app = App::new();
        let ship = spawn_ship(&mut app, true, false);
        set_red_alert(&mut app, ship, red_alert);
        contribute_objective_stance(
            &mut app,
            "tactical",
            objective_stance("objective-escort", StanceKind::Standard, true, true),
        );
        // The operator selects the objective stance while it is offered.
        set_admitted(&mut app, ship, "tactical", "objective-escort");
        app.world_mut()
            .run_system_cached(handle_set_station_stance)
            .unwrap();
        assert_eq!(
            stances(&app, ship).0.get(&tactical()).map(String::as_str),
            Some("objective-escort"),
        );

        // The objective ends → the contribution is withdrawn → reconcile drops
        // the now-unauthored selection.
        clear_objective_stances(&mut app);
        app.world_mut()
            .run_system_cached(reconcile_station_stances)
            .unwrap();
        assert!(
            !stances(&app, ship).0.contains_key(&tactical()),
            "a selected objective stance is removed when its objective ends",
        );
        assert_eq!(
            published_selected_stance(&mut app, ship),
            expected,
            "the Station visibly falls back to the current alert's neutral",
        );
    }
}

#[test]
fn a_persistent_objective_stance_held_behind_a_human_resolves_to_neutral_when_it_ends() {
    // AC5 persist-behind-human interaction (issue #1110 §6): a persist=true
    // objective stance is dormant while a human holds the target. If the
    // objective ends WHILE the human still holds it, the contribution is gone
    // from the effective catalogue, so on the Human→AI handoff the persist branch
    // finds no such stance and the target resolves to the alert-neutral instead
    // of resuming a stance that no longer exists.
    let mut app = App::new();
    let ship = spawn_ship(&mut app, false, false); // tactical human-held
    contribute_objective_stance(
        &mut app,
        "tactical",
        objective_stance("objective-escort", StanceKind::Standard, false, true),
    );
    insert_stance(&mut app, ship, "tactical", "objective-escort");

    // First observation while human-held records state, fires no edge.
    run_target_reconcile(&mut app);
    // The objective ends while the human still holds the target.
    clear_objective_stances(&mut app);
    // Human hands the target back to AI → Human→AI edge.
    set_tactical_ai(&mut app, ship, true);
    run_target_reconcile(&mut app);
    // The vanished objective stance does not resume; also swept by the removal
    // reconcile so the readout shows the neutral.
    app.world_mut()
        .run_system_cached(reconcile_station_stances)
        .unwrap();
    assert!(
        !stances(&app, ship).0.contains_key(&tactical()),
        "a persistent objective stance whose objective ended does not resume",
    );
    assert_eq!(
        published_selected_stance(&mut app, ship),
        "normal",
        "the target falls back to the alert-neutral when the objective is gone",
    );
}

#[test]
fn a_persistent_objective_stance_survives_a_handoff_while_its_objective_is_active() {
    // The counterpart: while the objective is STILL active its persist=true stance
    // is carried across the human's control exactly as a permanent one is, so it
    // resumes intact on the Human→AI edge (issue #1110 reuses #1108's persist path
    // through the effective catalogue).
    let mut app = App::new();
    let ship = spawn_ship(&mut app, false, false); // tactical human-held
    contribute_objective_stance(
        &mut app,
        "tactical",
        objective_stance("objective-escort", StanceKind::Standard, true, true),
    );
    insert_stance(&mut app, ship, "tactical", "objective-escort");

    run_target_reconcile(&mut app); // record human-held
    set_tactical_ai(&mut app, ship, true);
    run_target_reconcile(&mut app); // Human→AI edge with the objective still active
    assert_eq!(
        stances(&app, ship).0.get(&tactical()).map(String::as_str),
        Some("objective-escort"),
        "a persistent objective stance resumes while its objective is still active",
    );
}

/// A full-schedule app that wires the real [`CommandPlugin`] into `FixedUpdate`,
/// so `app.update()` drives the actual projection → reconcile → publish pipeline
/// (issue #1110).
///
/// Every fixture above injects [`ActiveObjectiveStances`] by hand and so never
/// runs [`project_active_objective_stances`]. This one instead seeds the
/// authoritative [`ObjectiveManagerRes`](crate::world::server::ObjectiveManagerRes)
/// and lets the SCHEDULED projection build the resource from it, closing the
/// projection seam the hand-injected tests skip. Mirrors the production SimSet
/// chain (`server_app`) so cross-set ordering — Input's projection/reconcile/
/// applier before Publish's blackboard — holds under `app.update()`.
fn projection_schedule_app() -> App {
    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin)
        // `operate_command_ai` takes `Res<Sessions>` as a param even for a
        // human-held Command seat that never emits; the resource must exist.
        .insert_resource(crate::lobby::Sessions(
            crate::lobby::session::SessionManager::new(),
        ))
        // The authoritative objective state the scheduled projection reads.
        .init_resource::<crate::world::server::ObjectiveManagerRes>()
        .configure_sets(
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
        .add_plugins(CommandPlugin);
    // One fixed step per `app.update()` (issue #895), so each update is exactly
    // one simulation tick of the pipeline under test.
    crate::ship::test_support::drive_one_fixed_step_per_update(
        &mut app,
        crate::ship::test_support::TEST_TICK,
    );
    app
}

/// Author an `Active` objective that lends `stance` to `station` through the
/// same [`ObjectiveManagerRes`](crate::world::server::ObjectiveManagerRes) door
/// the world dispatch pass writes — so the SCHEDULED projection, not the test,
/// publishes the contribution (issue #1110).
fn author_objective_stance(app: &mut App, id: &str, station: &str, stance: StationStanceConfig) {
    app.world_mut()
        .resource_mut::<crate::world::server::ObjectiveManagerRes>()
        .0
        .add_full_with_params(
            id,
            "objective text",
            std::collections::BTreeMap::new(),
            false,
            Vec::new(),
            crate::core::messages::AiDirective::default(),
            crate::objectives::UtilityConfig::default(),
            crate::core::messages::ObjectiveSource::default(),
            Some((StationId(station.into()), stance)),
        );
}

#[test]
fn the_scheduled_projection_exposes_then_drops_an_objective_stance_end_to_end() {
    // The projection seam every other #1110 test bypasses: drive the REAL
    // `project_active_objective_stances` through the schedule and prove
    // authoring an objective (a) exposes and makes selectable its stance, and
    // (b) once the objective ends, projection-empties → reconcile-drops the
    // selection back to the alert-neutral — end to end, over `app.update()`.
    let mut app = projection_schedule_app();
    let ship = spawn_ship(&mut app, true, false); // tactical AI, Command human-held

    // Author an objective contributing a persistent standard stance to Tactical
    // — into the manager, NOT the projection resource.
    author_objective_stance(
        &mut app,
        "escort-run",
        "tactical",
        objective_stance("objective-escort", StanceKind::Standard, true, true),
    );

    // Drive the schedule. Two ticks cover the documented one-tick projection lag
    // (projection reads objective state in SimSet::Input; a production
    // transition lands later, in SimSet::Physics) and let the state settle.
    app.update();
    app.update();

    // The projection ran: the contributed stance is in the published console
    // options WITHOUT the permanent catalogue being mutated.
    let bb = command_bb(&app, ship);
    assert!(
        bb.stances.iter().any(|s| s.id == "objective-escort"),
        "the scheduled projection exposes the objective stance in the console options",
    );
    assert_eq!(
        command_config().station(&tactical()).unwrap().stances.len(),
        4,
        "the permanent catalogue is never mutated by the projection",
    );

    // …and it is selectable through the shared applier, which reads the same
    // projection this tick.
    set_admitted(&mut app, ship, "tactical", "objective-escort");
    app.update();
    // The real admission pipeline consumes AdmittedCommands each tick; clear it
    // so a stale re-apply cannot resurrect the selection after the objective
    // ends (the assertion below would still hold, but this keeps the fixture
    // honest to production).
    app.world_mut()
        .entity_mut(ship)
        .get_mut::<AdmittedCommands>()
        .unwrap()
        .0
        .clear();
    assert_eq!(
        stances(&app, ship).0.get(&tactical()).map(String::as_str),
        Some("objective-escort"),
        "the projected objective stance is selectable through the scheduled applier",
    );

    // Complete the objective: its contribution leaves `active_station_stances`,
    // so the next scheduled projection empties the resource and the removal
    // reconcile drops the now-unauthored selection.
    assert!(
        app.world_mut()
            .resource_mut::<crate::world::server::ObjectiveManagerRes>()
            .0
            .complete("escort-run"),
        "the authored objective is Active and completes",
    );
    app.update();
    app.update();

    assert!(
        !stances(&app, ship).0.contains_key(&tactical()),
        "projection-empties → reconcile-drops removes the selection end to end",
    );
    assert_eq!(
        command_bb(&app, ship).selected_stance,
        "normal",
        "the directed Station settles on the alert-neutral once the objective ends",
    );
}
