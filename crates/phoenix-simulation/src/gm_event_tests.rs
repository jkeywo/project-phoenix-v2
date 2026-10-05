use super::*;
use crate::world::config::{scripted_trigger, TriggerCondition};

fn state(id: &str, layer: Option<&str>, repeat: bool, fired: bool) -> TriggerState {
    let mut trigger = scripted_trigger(TriggerCondition::Manual);
    trigger.id = Some(id.to_string());
    trigger.repeat = repeat;
    trigger.gm_controls = Some(GmEventControls::fire_only(
        id.to_string(),
        format!("world.gm.event.{id}"),
    ));
    TriggerState {
        trigger,
        fired,
        origin_layer: layer.map(str::to_string),
        seen_destroyed: Default::default(),
        last_fired_elapsed: None,
    }
}

#[test]
fn the_same_authored_id_in_two_layers_is_two_qualified_events() {
    let states = vec![
        state("breach", None, false, false),
        state("breach", Some("assets/worlds/layer.toml"), false, false),
    ];
    let events = controllable_events(
        &states,
        &Default::default(),
        &Default::default(),
        &Default::default(),
    );
    assert_eq!(
        events.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
        vec!["base-world::breach", "assets/worlds/layer.toml::breach"],
    );
    assert_eq!(find_event(&states, "base-world::breach"), Some(0));
    assert_eq!(
        find_event(&states, "assets/worlds/layer.toml::breach"),
        Some(1)
    );
    assert_eq!(find_event(&states, "base-world::missing"), None);
}

#[test]
fn a_trigger_without_controls_is_invisible_and_unaddressable() {
    let mut plain = state("breach", None, false, false);
    plain.trigger.gm_controls = None;
    let states = vec![plain];
    assert!(controllable_events(
        &states,
        &Default::default(),
        &Default::default(),
        &Default::default(),
    )
    .is_empty());
    assert_eq!(find_event(&states, "base-world::breach"), None);
    assert_eq!(fireable_index(&states, "base-world::breach"), None);
}

#[test]
fn a_control_set_without_fire_is_not_fireable_but_is_still_listed() {
    let mut listed = state("breach", None, false, false);
    listed.trigger.gm_controls.as_mut().expect("controls").fire = false;
    let states = vec![listed];
    assert_eq!(
        controllable_events(
            &states,
            &Default::default(),
            &Default::default(),
            &Default::default(),
        )
        .len(),
        1
    );
    assert_eq!(find_event(&states, "base-world::breach"), Some(0));
    assert_eq!(fireable_index(&states, "base-world::breach"), None);
}

#[test]
fn spent_reports_the_one_shot_latch_and_never_a_repeatable_one() {
    let states = vec![
        state("once", None, false, true),
        state("again", None, true, true),
    ];
    let events = controllable_events(
        &states,
        &Default::default(),
        &Default::default(),
        &Default::default(),
    );
    assert!(events[0].spent && !events[0].repeatable);
    assert!(!events[1].spent && events[1].repeatable);
}

/// Issue #1302: nothing in this module reads the condition, and this is the
/// assertion that says so rather than leaving it to inspection. An ORDINARY
/// condition-bearing trigger carrying the same control set is listed,
/// resolved and fireable on exactly the same terms as a manual one — and a
/// declaration WITHOUT Fire is still listed and still not fireable, whatever
/// its condition.
#[test]
fn an_automatic_event_is_listed_and_fireable_on_the_same_terms_as_a_manual_one() {
    let automatic = |id: &str, fire: bool| {
        let mut st = state(id, None, false, false);
        st.trigger.condition = TriggerCondition::OnDestroyed {
            entity_name: "courier".to_string(),
        };
        st.trigger.gm_controls.as_mut().expect("controls").fire = fire;
        st
    };
    let states = vec![automatic("evac", true), automatic("silent", false)];

    let events = controllable_events(
        &states,
        &Default::default(),
        &Default::default(),
        &Default::default(),
    );
    assert_eq!(
        events.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
        vec!["base-world::evac", "base-world::silent"],
    );
    assert_eq!(
        state_event_id(&states[0]).as_deref(),
        Some("base-world::evac"),
    );
    assert_eq!(fireable_index(&states, "base-world::evac"), Some(0));
    assert_eq!(
        fireable_index(&states, "base-world::silent"),
        None,
        "a listed automatic event without Fire is not fireable"
    );

    // And an automatic trigger that declares nothing is invisible, which is
    // every trigger every shipped world already authors.
    let mut plain = automatic("evac", true);
    plain.trigger.gm_controls = None;
    assert!(controllable_events(
        &[plain],
        &Default::default(),
        &Default::default(),
        &Default::default(),
    )
    .is_empty());
}

#[test]
fn an_armed_fire_is_projected_until_the_pipeline_runs_it() {
    let states = vec![state("breach", None, false, false)];
    let pending = std::collections::BTreeSet::from(["base-world::breach".to_string()]);
    assert!(
        controllable_events(&states, &pending, &Default::default(), &Default::default(),)[0].armed
    );
    assert!(
        !controllable_events(
            &states,
            &Default::default(),
            &Default::default(),
            &Default::default(),
        )[0]
        .armed
    );
}

/// Issue #1303: the Pause LEVER and the paused STATE are two facts, and
/// the projection reports both. A control set that does not declare Pause
/// is not pausable however the run has gone, which is the absent-control
/// refusal expressed at its source.
#[test]
fn only_a_declared_pause_lever_is_pausable_and_paused_is_a_separate_fact() {
    let mut declared = state("breach", None, false, false);
    declared
        .trigger
        .gm_controls
        .as_mut()
        .expect("controls")
        .pause = true;
    // Fire and Pause are independent levers, so an event may declare Pause
    // alone: `pausable_index` must not reach for `declares_fire`.
    let mut pause_only = state("quiet", None, false, false);
    {
        let controls = pause_only.trigger.gm_controls.as_mut().expect("controls");
        controls.fire = false;
        controls.pause = true;
    }
    let states = vec![declared, pause_only, state("sweep", None, true, false)];

    assert_eq!(pausable_index(&states, "base-world::breach"), Some(0));
    assert_eq!(pausable_index(&states, "base-world::quiet"), Some(1));
    assert_eq!(
        pausable_index(&states, "base-world::sweep"),
        None,
        "an event that declares no Pause lever exposes no toggle"
    );
    assert_eq!(pausable_index(&states, "base-world::missing"), None);
    assert_eq!(
        fireable_index(&states, "base-world::quiet"),
        None,
        "and declaring Pause does not quietly add Fire"
    );

    let paused = std::collections::BTreeSet::from(["base-world::breach".to_string()]);
    let events = controllable_events(&states, &Default::default(), &paused, &Default::default());
    assert_eq!(
        events
            .iter()
            .map(|e| (e.pause, e.paused))
            .collect::<Vec<_>>(),
        vec![(true, true), (true, false), (false, false)],
        "the lever is authored; the engaged state belongs to the run"
    );
}

/// A spent one-shot is still pausable (issue #1303). Pause decides whether
/// the condition is EVALUATED, and a GM offered a toggle that silently
/// stopped answering once the event had fired would have no way to tell
/// that apart from a broken control.
#[test]
fn a_spent_one_shot_event_is_still_pausable() {
    let mut spent = state("breach", None, false, true);
    spent.trigger.gm_controls.as_mut().expect("controls").pause = true;
    let states = vec![spent];
    assert!(
        controllable_events(
            &states,
            &Default::default(),
            &Default::default(),
            &Default::default(),
        )[0]
        .spent
    );
    assert_eq!(pausable_index(&states, "base-world::breach"), Some(0));
}

/// Issue #1304: the Skip lever is declared, resolved and projected
/// independently of Fire. A control set may carry either, both or neither,
/// and the panel must be able to tell which — an event that declares only
/// Skip is listed and skippable but NOT fireable, and the reverse.
#[test]
fn the_skip_lever_is_declared_and_resolved_independently_of_fire() {
    let lever = |id: &str, fire: bool, skip: bool| {
        let mut st = state(id, None, false, false);
        st.trigger.condition = TriggerCondition::OnDestroyed {
            entity_name: "courier".to_string(),
        };
        let controls = st.trigger.gm_controls.as_mut().expect("controls");
        controls.fire = fire;
        controls.skip = skip;
        st
    };
    let states = vec![
        lever("both", true, true),
        lever("fire_only", true, false),
        lever("skip_only", false, true),
    ];

    let events = controllable_events(
        &states,
        &Default::default(),
        &Default::default(),
        &Default::default(),
    );
    assert_eq!(
        events
            .iter()
            .map(|event| (event.fire, event.skip))
            .collect::<Vec<_>>(),
        vec![(true, true), (true, false), (false, true)],
        "the panel is told exactly which levers each event declares"
    );

    assert_eq!(skippable_index(&states, "base-world::both"), Some(0));
    assert_eq!(
        skippable_index(&states, "base-world::fire_only"),
        None,
        "a listed event without Skip is not skippable"
    );
    assert_eq!(skippable_index(&states, "base-world::skip_only"), Some(2));
    assert_eq!(
        fireable_index(&states, "base-world::skip_only"),
        None,
        "and declaring Skip does not imply Fire"
    );
    assert_eq!(skippable_index(&states, "base-world::missing"), None);

    // A trigger that declares nothing is unaddressable by either lever.
    let mut plain = lever("both", true, true);
    plain.trigger.gm_controls = None;
    assert_eq!(skippable_index(&[plain], "base-world::both"), None);
}

/// The two arms are projected from two sets, so an event can be armed on
/// one lever, the other, both or neither — which is what makes "Fire does
/// not consume an armed Skip" visible on the panel rather than merely true
/// in the simulation.
#[test]
fn an_armed_skip_is_projected_beside_an_armed_fire_and_never_instead_of_it() {
    let states = vec![state("breach", None, false, false)];
    let armed = std::collections::BTreeSet::from(["base-world::breach".to_string()]);
    let none = std::collections::BTreeSet::new();

    let neither = &controllable_events(&states, &none, &none, &none)[0];
    assert!(!neither.armed && !neither.skip_armed);

    let skip_only = &controllable_events(&states, &none, &none, &armed)[0];
    assert!(!skip_only.armed && skip_only.skip_armed);

    let fire_only = &controllable_events(&states, &armed, &none, &none)[0];
    assert!(fire_only.armed && !fire_only.skip_armed);

    let both = &controllable_events(&states, &armed, &none, &armed)[0];
    assert!(both.armed && both.skip_armed);
}

/// The one live-id set both cleanups read. An event with no control set has
/// no qualified id at all, so it can never appear here — which is what
/// keeps a Skip arm from being retained against a trigger no GM can name.
#[test]
fn live_event_ids_names_every_addressable_event_and_nothing_else() {
    let mut plain = state("silent", None, false, false);
    plain.trigger.gm_controls = None;
    let states = vec![
        state("breach", None, false, false),
        state("breach", Some("assets/worlds/layer.toml"), false, false),
        plain,
    ];
    assert_eq!(
        live_event_ids(&states).into_iter().collect::<Vec<_>>(),
        vec![
            "assets/worlds/layer.toml::breach".to_string(),
            "base-world::breach".to_string(),
        ],
    );
}
