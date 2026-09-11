//! Typed world-authored GM widgets, from the world file to the payload the
//! browser reads (issue #1439, PRD #1419 story 13).
//!
//! The chain this pins is the one an author actually travels: a real TOML file
//! on disk, through the ordinary world loader, through the ordinary
//! presentation encoder, into the exact JSON `gui/gm-widgets-panel.js` is
//! driven with by `tests/client/gm-widgets-panel.test.js` — the two languages
//! share `tests/fixtures/gm-widgets-presets.json` so neither side can drift
//! into testing a shape the other does not produce.
//!
//! Refusal cases live beside the rest of the world grammar in
//! `src/world/config_tests.rs`, where `parse_world`'s other semantic errors are.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use project_phoenix::core::codec::encode_gm_role_presets;
use project_phoenix::world::config::parse_world;

const WORLD: &str = "assets/worlds/probe_gm_widgets.toml";
const FIXTURE: &str = "tests/fixtures/gm-widgets-presets.json";

fn presets() -> Vec<project_phoenix::world::config::GmRolePresetEntry> {
    let toml = std::fs::read_to_string(WORLD).expect("the probe world ships");
    parse_world(&toml)
        .expect("the probe world must load")
        .gm_role_presets
}

#[test]
fn an_authored_world_reaches_the_browser_as_the_fixture_both_languages_share() {
    let encoded = encode_gm_role_presets(&presets());
    let actual: serde_json::Value = serde_json::from_str(&encoded).expect("encoder emits JSON");
    let expected: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(FIXTURE).expect("the fixture ships"))
            .expect("the fixture is JSON");
    assert_eq!(
        actual, expected,
        "the world's widget composition changed; update {FIXTURE} so the \
         browser tests keep driving what this build actually publishes"
    );
}

#[test]
fn every_widget_the_world_authors_carries_only_the_facets_its_type_owns() {
    // The structural half of "no arbitrary author UI code": each rendered card
    // is a closed record whose fields are decided by its `type`, so there is no
    // authored world in which a note carries a filter or an action widget
    // carries text — the browser never has to defend against one.
    for preset in presets() {
        for widget in &preset.widget {
            match widget.kind.as_str() {
                "attention" => {
                    assert!(widget.actions.is_empty() && widget.text.is_none());
                }
                "workload" => {
                    assert!(widget.band.is_none() && widget.category.is_none());
                    assert!(widget.actions.is_empty() && widget.text.is_none());
                }
                "actions" => {
                    assert!(!widget.actions.is_empty());
                    assert!(
                        widget.band.is_none() && widget.ship.is_none() && widget.text.is_none()
                    );
                }
                "note" => {
                    let text = widget.text.as_deref().expect("a note has text");
                    // The note contract, stated as a property of the loaded
                    // world rather than as an escaping rule at the far end.
                    assert!(text
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')));
                    assert!(widget.band.is_none() && widget.actions.is_empty());
                }
                other => panic!("unknown widget type '{other}' loaded"),
            }
        }
    }
}

#[test]
fn an_actions_widget_names_only_gm_action_buttons_the_build_draws() {
    use project_phoenix::world::config::GM_WIDGET_ACTION_IDS;
    let authored = presets()
        .into_iter()
        .flat_map(|preset| preset.widget)
        .filter(|widget| widget.kind == "actions")
        .flat_map(|widget| widget.actions)
        .collect::<Vec<_>>();
    assert!(
        !authored.is_empty(),
        "the probe world authors an action widget"
    );
    for action in authored {
        assert!(
            GM_WIDGET_ACTION_IDS.contains(&action.as_str()),
            "'{action}' is not a permitted GM action button"
        );
    }
}

#[test]
fn the_widget_composition_is_presentation_and_never_reaches_the_simulation() {
    // Role presets are personal presentation (issue #1319) and widgets inherit
    // that exactly: they live on `WorldConfig::gm_role_presets`, which crosses
    // to the page through `wasm_get_gm_role_presets` alone. If a widget ever
    // grew a GmAction, a snapshot field or a digest fold, this assertion is the
    // first place it would have to be argued away.
    let toml = std::fs::read_to_string(WORLD).expect("the probe world ships");
    let cfg = parse_world(&toml).expect("must load");
    assert_eq!(cfg.gm_role_presets.len(), 2);
    // No authored widget declares a spawn palette entry, a GM event or any
    // other authoritative table — the widget vocabulary has no field for one.
    assert!(cfg.gm_palette.is_empty());
    assert!(cfg.gm_comms_routes.is_empty());
}
