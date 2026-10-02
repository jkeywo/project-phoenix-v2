use super::*;

fn reference(uuid: &str) -> GmUndoReference {
    GmUndoReference {
        operator_id: "gm-1".into(),
        correlation: GmActionId::new(uuid.to_string()).unwrap(),
        sequence: 1,
    }
}

fn capture(correlation: &str, entity: &str, restorable: bool) -> GmRemovalCapture {
    GmRemovalCapture {
        action: reference(correlation),
        entity: entity.into(),
        names: Vec::new(),
        tick: 10,
        state: restorable.then(|| {
            Box::new(EntityState {
                uuid: entity.into(),
                spawn: Some(crate::world::spawn_origin::SpawnOrigin {
                    template_path: "assets/entities/raider.toml".into(),
                    name: entity.into(),
                    ..Default::default()
                }),
                ..Default::default()
            })
        }),
        references: Vec::new(),
    }
}

#[test]
fn evicts_the_oldest_capture_once_the_run_bound_is_reached() {
    let mut store = GmDespawnCaptures::default();
    for index in 0..MAX_GM_DESPAWN_CAPTURES + 3 {
        store.record(capture(
            &format!("corr-{index}"),
            &format!("e-{index}"),
            true,
        ));
    }
    assert_eq!(store.entries().len(), MAX_GM_DESPAWN_CAPTURES);
    // The three oldest are gone; the newest are the ones a live GM is
    // actually about to reverse.
    assert!(store
        .capture_for("gm-1", &GmActionId::new("corr-0".to_string()).unwrap(), 1)
        .is_none());
    assert!(store
        .capture_for(
            "gm-1",
            &GmActionId::new(format!("corr-{}", MAX_GM_DESPAWN_CAPTURES + 2)).unwrap(),
            1
        )
        .is_some());
}

#[test]
fn an_unrestorable_capture_is_retained_but_never_reported_restorable() {
    let mut store = GmDespawnCaptures::default();
    store.record(capture("corr-a", "authored-1", false));
    assert!(store
        .capture_for("gm-1", &GmActionId::new("corr-a".to_string()).unwrap(), 1)
        .is_some());
    assert!(!store.is_restorable("gm-1", &GmActionId::new("corr-a".to_string()).unwrap(), 1));
}

#[test]
fn refuses_a_restore_whose_identity_is_live_again() {
    let capture = capture("corr-a", "raider-1", true);
    let live: std::collections::BTreeSet<String> = ["raider-1".to_string()].into();
    assert_eq!(
        restore_precheck(
            &capture,
            &live,
            &Default::default(),
            &Default::default(),
            &Default::default(),
            &Default::default(),
            |_| true
        ),
        Err(GmActionRefusalReason::RestoreIdentityOccupied)
    );
}

#[test]
fn refuses_a_restore_whose_authored_name_the_world_has_re_bound() {
    let mut capture = capture("corr-a", "raider-1", true);
    capture.names = vec!["raider".into()];
    let live: std::collections::BTreeSet<String> = ["raider-2".to_string()].into();
    let mut names = std::collections::HashMap::new();
    names.insert("raider".to_string(), "raider-2".to_string());
    assert_eq!(
        restore_precheck(
            &capture,
            &live,
            &names,
            &Default::default(),
            &Default::default(),
            &Default::default(),
            |_| true
        ),
        Err(GmActionRefusalReason::RestoreReferenceConflict)
    );
}

#[test]
fn allows_a_restore_whose_authored_name_still_names_its_own_removed_uuid() {
    let mut capture = capture("corr-a", "raider-1", true);
    capture.names = vec!["raider".into()];
    let mut names = std::collections::HashMap::new();
    names.insert("raider".to_string(), "raider-1".to_string());
    assert_eq!(
        restore_precheck(
            &capture,
            &Default::default(),
            &names,
            &Default::default(),
            &Default::default(),
            &Default::default(),
            |_| true
        ),
        Ok(())
    );
}

#[test]
fn refuses_a_restore_whose_contact_override_pair_has_a_newer_decision() {
    let mut capture = capture("corr-a", "raider-1", true);
    capture.references = vec![GmRemovalReference {
        observer: "player-1".into(),
        target: "raider-1".into(),
        mode: ContactMode::Reveal,
        classification: None,
        report: None,
    }];
    let mut overrides = crate::gm_contact::ContactOverrides::new();
    overrides
        .entry("player-1".into())
        .or_default()
        .insert("raider-1".into(), ContactMode::Conceal);
    assert_eq!(
        restore_precheck(
            &capture,
            &Default::default(),
            &Default::default(),
            &overrides,
            &Default::default(),
            &Default::default(),
            |_| true
        ),
        Err(GmActionRefusalReason::RestoreReferenceConflict)
    );
}

#[test]
fn allows_a_restore_when_only_unrelated_overrides_changed() {
    let mut capture = capture("corr-a", "raider-1", true);
    capture.references = vec![GmRemovalReference {
        observer: "player-1".into(),
        target: "raider-1".into(),
        mode: ContactMode::Reveal,
        classification: None,
        report: None,
    }];
    let mut overrides = crate::gm_contact::ContactOverrides::new();
    overrides
        .entry("player-1".into())
        .or_default()
        .insert("freighter-9".into(), ContactMode::Conceal);
    assert_eq!(
        restore_precheck(
            &capture,
            &Default::default(),
            &Default::default(),
            &overrides,
            &Default::default(),
            &Default::default(),
            |_| true
        ),
        Ok(())
    );
}

#[test]
fn refuses_a_restore_whose_template_no_longer_resolves() {
    let capture = capture("corr-a", "raider-1", true);
    assert_eq!(
        restore_precheck(
            &capture,
            &Default::default(),
            &Default::default(),
            &Default::default(),
            &Default::default(),
            &Default::default(),
            |_| false
        ),
        Err(GmActionRefusalReason::InverseUnsupported)
    );
}
