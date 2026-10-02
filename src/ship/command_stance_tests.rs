use super::*;

fn catalogue() -> Vec<StationStanceConfig> {
    vec![
        StationStanceConfig {
            id: "weapons-free".into(),
            label: String::new(),
            kind: StanceKind::Standard,
            high_alert: true,
            persist_behind_human: true,
            ai_engaged: true,
        },
        StationStanceConfig {
            id: "hold".into(),
            label: String::new(),
            kind: StanceKind::Standard,
            high_alert: false,
            persist_behind_human: false,
            ai_engaged: false,
        },
        StationStanceConfig {
            id: "normal".into(),
            label: String::new(),
            kind: StanceKind::NormalAlertNeutral,
            high_alert: false,
            persist_behind_human: false,
            ai_engaged: false,
        },
        StationStanceConfig {
            id: "high".into(),
            label: String::new(),
            kind: StanceKind::HighAlertNeutral,
            high_alert: true,
            persist_behind_human: false,
            ai_engaged: false,
        },
    ]
}

#[test]
fn neutral_tracks_the_alert_level() {
    let c = catalogue();
    assert_eq!(neutral_stance_for_alert(&c, false), Some("normal"));
    assert_eq!(neutral_stance_for_alert(&c, true), Some("high"));
}

#[test]
fn only_authored_stances_are_selectable() {
    let c = catalogue();
    assert!(is_selectable(&c, "weapons-free"));
    assert!(is_selectable(&c, "normal"));
    assert!(!is_selectable(&c, "invented-order"));
}

#[test]
fn no_selection_tracks_the_ships_own_alert() {
    let c = catalogue();
    // This is the byte-identical default: posture == red_alert.
    assert!(!effective_high_alert(&c, None, false));
    assert!(effective_high_alert(&c, None, true));
}

#[test]
fn a_standard_stance_seeds_its_authored_posture() {
    let c = catalogue();
    // "hold" forces stood-down even at red alert; "weapons-free" forces
    // high alert even when the ship is not.
    assert!(!effective_high_alert(&c, Some("hold"), true));
    assert!(effective_high_alert(&c, Some("weapons-free"), false));
}

#[test]
fn alert_change_switches_neutral_to_neutral() {
    let c = catalogue();
    assert_eq!(
        selection_after_alert_change(&c, Some("normal"), true).as_deref(),
        Some("high"),
    );
    assert_eq!(
        selection_after_alert_change(&c, Some("high"), false).as_deref(),
        Some("normal"),
    );
}

#[test]
fn alert_change_never_overwrites_a_standard_stance() {
    let c = catalogue();
    assert_eq!(
        selection_after_alert_change(&c, Some("weapons-free"), true).as_deref(),
        Some("weapons-free"),
    );
    assert_eq!(
        selection_after_alert_change(&c, Some("hold"), false).as_deref(),
        Some("hold"),
    );
}

#[test]
fn absent_selection_adopts_the_current_neutral() {
    let c = catalogue();
    assert_eq!(
        selection_after_alert_change(&c, None, true).as_deref(),
        Some("high"),
    );
}

/// One objective-contributed stance for the merge tests (issue #1110).
fn objective_stance() -> StationStanceConfig {
    StationStanceConfig {
        id: "objective-escort".into(),
        label: String::new(),
        kind: StanceKind::Standard,
        high_alert: true,
        persist_behind_human: true,
        ai_engaged: false,
    }
}

#[test]
fn effective_catalogue_without_contributions_is_byte_identical() {
    // AC1: contributed absent → the merge is a plain clone of the permanent
    // catalogue, so an undirected/objective-free hull is unchanged.
    let c = catalogue();
    assert_eq!(effective_catalogue(&c, &[]), c);
}

#[test]
fn effective_catalogue_appends_an_active_contribution() {
    // AC2: an active objective's stance joins the exposed vocabulary…
    let c = catalogue();
    let contributed = vec![objective_stance()];
    let effective = effective_catalogue(&c, &contributed);
    assert_eq!(effective.len(), c.len() + 1);
    assert!(is_selectable(&effective, "objective-escort"));
    // …WITHOUT mutating the permanent catalogue (AC1): it still lacks the id.
    assert!(!is_selectable(&c, "objective-escort"));
}

#[test]
fn effective_catalogue_dedupes_by_id_permanent_wins() {
    // A contributed id colliding with a permanent one never shadows or
    // redefines the authored stance: the permanent copy is kept.
    let c = catalogue();
    let collide = StationStanceConfig {
        id: "weapons-free".into(),
        label: "SHADOW".into(),
        kind: StanceKind::Standard,
        high_alert: false,
        persist_behind_human: false,
        ai_engaged: false,
    };
    let effective = effective_catalogue(&c, &[collide]);
    assert_eq!(effective.len(), c.len(), "no duplicate id is appended");
    let kept = stance_by_id(&effective, "weapons-free").unwrap();
    assert!(kept.high_alert, "the permanent stance is the one kept");
    assert_eq!(
        kept.label, "",
        "the contributed stance did not overwrite it"
    );
}

#[test]
fn reconcile_selection_against_the_merged_slice_keeps_an_active_objective_stance() {
    // #1110: while the objective is active its stance is a member of the
    // effective catalogue, so a selection of it survives reconcile…
    let c = catalogue();
    let effective = effective_catalogue(&c, &[objective_stance()]);
    assert_eq!(
        reconcile_selection(&effective, "objective-escort").as_deref(),
        Some("objective-escort"),
    );
    // …but once the objective ends the contribution is gone, so the same
    // selection reconciled against the PERMANENT catalogue is dropped.
    assert_eq!(reconcile_selection(&c, "objective-escort"), None);
}

#[test]
fn reconcile_keeps_authored_ids_and_drops_vanished_ones() {
    // Criterion 4: the catalogue is the single membership authority. An id
    // still authored survives; one no longer present falls out so the
    // caller clears it back to the alert-neutral tracking default.
    let c = catalogue();
    assert_eq!(
        reconcile_selection(&c, "weapons-free").as_deref(),
        Some("weapons-free"),
    );
    assert_eq!(reconcile_selection(&c, "normal").as_deref(), Some("normal"));
    // A stance that has left the catalogue (e.g. an objective stance whose
    // objective ended, #1110) is dropped.
    assert_eq!(reconcile_selection(&c, "objective-escort"), None);
}

#[test]
fn human_handoff_resets_only_non_persistent_standard_stances() {
    let c = catalogue();
    // Non-persistent "hold" resets to the level's neutral…
    assert_eq!(
        selection_after_human_lost(&c, Some("hold"), true).as_deref(),
        Some("high"),
    );
    // …but a persist-behind-human standard order is kept…
    assert_eq!(
        selection_after_human_lost(&c, Some("weapons-free"), false).as_deref(),
        Some("weapons-free"),
    );
    // …and a neutral is always kept.
    assert_eq!(
        selection_after_human_lost(&c, Some("normal"), false).as_deref(),
        Some("normal"),
    );
}

// ── AI Command selection (issue #1109) ──────────────────────────────────

#[test]
fn ai_command_selects_only_authored_stances() {
    // AC2/AC3 catalogue parity: whatever the ship knowledge, the pick is
    // always an id the catalogue authors — never invented. Exhaustively over
    // both alert levels.
    let c = catalogue();
    for red_alert in [false, true] {
        let picked = select_stance(&c, CommandKnowledge { red_alert })
            .expect("a well-formed catalogue always resolves a stance");
        assert!(
            is_selectable(&c, &picked),
            "AI Command must only ever select an authored stance; picked {picked:?}"
        );
    }
}

#[test]
fn ai_command_adopts_the_engaged_stance_at_high_alert() {
    // At Red Alert the AI adopts the authored `ai_engaged` posture…
    let c = catalogue();
    assert_eq!(
        select_stance(&c, CommandKnowledge { red_alert: true }).as_deref(),
        Some("weapons-free"),
    );
    // …and stands down to the normal-alert neutral otherwise.
    assert_eq!(
        select_stance(&c, CommandKnowledge { red_alert: false }).as_deref(),
        Some("normal"),
    );
}

#[test]
fn ai_command_selection_is_repeatable_for_the_same_knowledge() {
    // AC5 repeatability: a pure function of catalogue + knowledge, so the
    // same inputs yield the same id every time.
    let c = catalogue();
    for red_alert in [false, true] {
        let facts = CommandKnowledge { red_alert };
        let first = select_stance(&c, facts);
        for _ in 0..8 {
            assert_eq!(select_stance(&c, facts), first);
        }
    }
}

#[test]
fn ai_command_without_an_engaged_stance_tracks_the_neutral() {
    // A catalogue that flags no `ai_engaged` posture falls back to the
    // alert-appropriate neutral at every level — the byte-identical tracking
    // default, never an invented escalation.
    let mut c = catalogue();
    for stance in &mut c {
        stance.ai_engaged = false;
    }
    assert_eq!(
        select_stance(&c, CommandKnowledge { red_alert: true }).as_deref(),
        Some("high"),
    );
    assert_eq!(
        select_stance(&c, CommandKnowledge { red_alert: false }).as_deref(),
        Some("normal"),
    );
}
