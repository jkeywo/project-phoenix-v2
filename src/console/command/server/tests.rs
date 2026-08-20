//! Unit tests for the Command console server wiring (issue #1107).
//!
//! These drive the plugin's systems directly over a hand-built world (post
//! admission), so they pin the CONTENT rules — what a stance order does once it
//! is admitted, how the alert switch and the AI operator behave, and what the
//! console blackboard reports. The admission AUTHORISATION of the order and the
//! wire round-trip are pinned in `command_admission`/`core::codec`.

use super::*;
use crate::messages::{
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
        crate::system_registry::command_system_id(),
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
            crate::ship_state::ShipRedAlert::default(),
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
        .get_mut::<crate::ship_state::ShipRedAlert>()
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
            crate::system_registry::COMMAND_SYSTEM_ID.to_string(),
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
        target: crate::system_registry::command_system_id(),
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
            crate::system_registry::command_system_id(),
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
        .get_mut::<crate::ship_state::ShipRedAlert>()
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
        .get_mut::<crate::ship_state::ShipRedAlert>()
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
        weapons_station_stance_high_alert(Some(&selections), &config, &sources, false),
        None
    );

    // A human's "hold" stance forces stood-down even at red alert.
    selections.0.insert(tactical(), "hold".into());
    assert_eq!(
        weapons_station_stance_high_alert(Some(&selections), &config, &sources, true),
        Some(false),
        "a hold stance overrides the ship's own red alert for the fire gate"
    );

    // The same selection is ignored once a human holds the weapons Station.
    let mut human_sources = ControlSourceResolver::default();
    human_sources.set(SystemId("phaser-fore".into()), ControlSource::Human);
    assert_eq!(
        weapons_station_stance_high_alert(Some(&selections), &config, &human_sources, true),
        None,
        "Command does not direct a human-held weapons Station"
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
            crate::system_registry::COMMAND_SYSTEM_ID.to_string(),
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
            crate::system_registry::COMMAND_SYSTEM_ID.to_string(),
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
        .insert(crate::entity_spawner::EntityUuid(
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
