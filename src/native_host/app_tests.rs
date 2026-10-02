use super::*;
use crate::core::messages::StationId;

fn stations(ids: &[&str]) -> Vec<StationId> {
    ids.iter().map(|id| StationId(id.to_string())).collect()
}

#[test]
fn a_pane_named_for_a_station_on_this_hull_is_refused_at_the_prompt() {
    // Issue #1331 opened a collision that could not exist before it: the
    // lobby's screen rows open a station's console under the STATION ID as
    // its pane name, and `PaneBus::open_pane_for_name` resolves by exactly
    // that name. So a hand-authored `--pane helm` on a hull with a `helm`
    // station is two participants under one key — and the screen row's off
    // button, resolving "helm", would close the person's console instead of
    // the station's.
    let shadowed = pane_labels_shadowing_stations(
        &["Ada".to_string(), "helm".to_string()],
        &stations(&["helm", "weapons"]),
    );
    assert_eq!(shadowed, vec!["helm".to_string()], "only the collision");

    // And it is REFUSED, in a sentence that says what to do instead — a
    // warning would leave the operator's own console to be closed by
    // somebody pressing a button about a station.
    let refusal = NativeHostError::PaneShadowsStation(shadowed).to_string();
    assert!(refusal.contains("\"helm\""), "{refusal}");
    assert!(refusal.contains("one namespace"), "{refusal}");
    assert!(refusal.contains("--pane helm-crew"), "{refusal}");
}

#[test]
fn a_pane_that_names_nobody_on_the_roster_is_left_alone() {
    // The ordinary `--pane <NAME>` this must not disturb: a crew member's
    // own name, on a hull whose stations are named for jobs. It is also
    // exact and case-sensitive, because `open_pane_for_name` is — a guard
    // that judged by a looser rule than the lookup it protects would refuse
    // a name the lookup never confuses.
    assert!(pane_labels_shadowing_stations(
        &["Ada".to_string(), "Grace".to_string()],
        &stations(&["helm", "weapons"]),
    )
    .is_empty());
    assert!(
        pane_labels_shadowing_stations(&["Helm".to_string()], &stations(&["helm"]),).is_empty()
    );
    assert!(pane_labels_shadowing_stations(&[], &stations(&["helm"])).is_empty());
}
