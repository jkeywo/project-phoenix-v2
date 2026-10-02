use super::*;
use crate::ship::damage::DamageTier;

fn status(id: &str, current: f32) -> SystemHullStatus {
    SystemHullStatus {
        system_id: SystemId(id.into()),
        display_name: id.into(),
        current,
        max_hp: 100.0,
        tier: DamageTier::Operational,
        debuff_magnitude: 0.0,
    }
}

/// A system at the `Destroyed` tier — 0 HP, full capacity still declared.
fn destroyed(id: &str) -> SystemHullStatus {
    SystemHullStatus {
        tier: DamageTier::Destroyed,
        ..status(id, 0.0)
    }
}

/// helm-radar -> helm, sensors -> science, repair -> engineering,
/// "core" -> ownerless.
fn vis(on_site: Vec<&str>) -> HullVisibility {
    vis_with(
        vec![
            status("core", 40.0),
            status("helm-radar", 60.0),
            status("sensors", 100.0),
            status("repair", 100.0),
        ],
        on_site,
    )
}

/// Same ownership map as [`vis`], with the hull entries supplied — used to
/// author tiers (a `Destroyed` system) the plain fixture never produces.
fn vis_with(entries: Vec<SystemHullStatus>, on_site: Vec<&str>) -> HullVisibility {
    let owner_of = [
        (SystemId("core".into()), None),
        (
            SystemId("helm-radar".into()),
            Some(StationId("helm".into())),
        ),
        (
            SystemId("sensors".into()),
            Some(StationId("science".into())),
        ),
        (
            SystemId("repair".into()),
            Some(StationId("engineering".into())),
        ),
    ]
    .into_iter()
    .collect();
    HullVisibility::new(
        entries,
        owner_of,
        Some(StationId("engineering".into())),
        on_site.into_iter().map(|s| SystemId(s.into())).collect(),
    )
}

fn ids(rows: &[SystemHullStatus]) -> Vec<&str> {
    rows.iter().map(|r| r.system_id.0.as_str()).collect()
}

#[test]
fn engineering_sees_core_and_its_own_systems_but_no_other_detail() {
    let v = vis(vec![]);
    let eng = StationId("engineering".into());
    assert_eq!(ids(&v.entries_for(Some(&eng))), vec!["core", "repair"]);
}

#[test]
fn station_owner_sees_only_its_own_systems() {
    let v = vis(vec![]);
    let helm = StationId("helm".into());
    assert_eq!(ids(&v.entries_for(Some(&helm))), vec!["helm-radar"]);
    let science = StationId("science".into());
    assert_eq!(ids(&v.entries_for(Some(&science))), vec!["sensors"]);
}

#[test]
fn station_owner_never_sees_core() {
    let v = vis(vec![]);
    let helm = StationId("helm".into());
    assert!(!v.can_see(Some(&helm), &SystemId("core".into())));
}

#[test]
fn on_site_team_reveals_non_core_detail_to_engineering_only() {
    let v = vis(vec!["helm-radar"]);
    let eng = StationId("engineering".into());
    assert_eq!(
        ids(&v.entries_for(Some(&eng))),
        vec!["core", "helm-radar", "repair"]
    );
    // The reveal is Engineering-scoped: Science still sees only its own.
    let science = StationId("science".into());
    assert_eq!(ids(&v.entries_for(Some(&science))), vec!["sensors"]);
}

#[test]
fn unassigned_viewer_sees_no_detail_but_still_gets_the_aggregate() {
    let v = vis(vec![]);
    let p = v.projection_for(None);
    assert!(p.entries.is_empty());
    assert!(p.aggregate_fraction.is_some());
}

#[test]
fn aggregate_covers_every_system_including_ones_the_viewer_cannot_see() {
    let v = vis(vec![]);
    // 40 + 60 + 100 + 100 out of 400.
    let expected = 300.0 / 400.0;
    for viewer in [
        None,
        Some(StationId("helm".into())),
        Some(StationId("engineering".into())),
    ] {
        let p = v.projection_for(viewer.as_ref());
        assert!((p.aggregate_fraction.unwrap() - expected).abs() < 1e-6);
    }
}

// ── Issue #1100: per-station health, published station-level ──────────────
//
// The Hero Bar shows every station's health from the host's own sum, not
// from the recipient-scoped rows a client happens to hold. `station_fractions`
// is that sum: one scalar per owning station, an explicit `None` for a
// station that owns no damageable capacity.

/// The `station_fractions` entry for `station`, or `None` if absent.
/// Outer `Option` is presence; inner is the published health.
fn health_of(v: &HullVisibility, station: &str) -> Option<Option<f32>> {
    v.station_fractions()
        .into_iter()
        .find(|(s, _)| s.0 == station)
        .map(|(_, f)| f)
}

#[test]
fn station_fractions_report_a_damaged_station_from_its_own_capacity() {
    // helm owns only helm-radar, at 60/100.
    let v = vis(vec![]);
    let f = health_of(&v, "helm")
        .expect("helm present")
        .expect("helm has damageable capacity");
    assert!((f - 0.6).abs() < 1e-6, "expected ~0.6, got {f}");
}

#[test]
fn station_fractions_report_a_fully_healthy_station_as_one() {
    // science owns only sensors, at 100/100.
    let v = vis(vec![]);
    let f = health_of(&v, "science")
        .expect("science present")
        .expect("science has damageable capacity");
    assert!((f - 1.0).abs() < 1e-6, "expected ~1.0, got {f}");
}

#[test]
fn station_fractions_bucket_ownerless_systems_under_core() {
    // core (ownerless) is at 40/100.
    let v = vis(vec![]);
    let f = health_of(&v, CORE_BUCKET_ID)
        .expect("core present")
        .expect("core has damageable capacity");
    assert!((f - 0.4).abs() < 1e-6, "expected ~0.4, got {f}");
}

#[test]
fn a_station_with_no_damageable_capacity_is_the_neutral_none_state() {
    // `comms` owns one system the ship declares at zero max — no damage
    // model at all — while `helm` owns a normal damageable system.
    let entries = vec![
        status("helm-radar", 60.0),
        SystemHullStatus {
            max_hp: 0.0,
            ..status("comms-array", 0.0)
        },
    ];
    let owner_of = [
        (
            SystemId("helm-radar".into()),
            Some(StationId("helm".into())),
        ),
        (
            SystemId("comms-array".into()),
            Some(StationId("comms".into())),
        ),
    ]
    .into_iter()
    .collect();
    let v = HullVisibility::new(
        entries,
        owner_of,
        Some(StationId("engineering".into())),
        vec![],
    );
    // Present, but explicitly neutral — the no-damage-model state.
    assert_eq!(health_of(&v, "comms"), Some(None));
    // The healthy path still reports its own capacity beside it.
    let f = health_of(&v, "helm").flatten().expect("helm has capacity");
    assert!((f - 0.6).abs() < 1e-6, "expected ~0.6, got {f}");
}

// ── Issue #1014: destroyed capability is a whole-ship scalar ──────────────
//
// The worst case is a system that is destroyed, owned by a station nobody is
// on, with no repair team on site — it appears in *no* recipient's projected
// rows, so both whole-ship scalars have to be computed from the full entry
// list or the loss is invisible to everyone.

/// `sensors` (science-owned, off-site) destroyed; nothing else damaged.
fn vis_with_destroyed_offsite() -> HullVisibility {
    vis_with(
        vec![
            status("core", 100.0),
            status("helm-radar", 100.0),
            destroyed("sensors"),
            status("repair", 100.0),
        ],
        vec![],
    )
}

#[test]
fn a_destroyed_offsite_system_still_drags_the_aggregate_down_for_every_viewer() {
    let v = vis_with_destroyed_offsite();
    // 100 + 100 + 0 + 100 out of 400 — the destroyed system stays in the
    // denominator, so the ship cannot read as fully healthy.
    let expected = 300.0 / 400.0;
    for viewer in [
        None,
        Some(StationId("helm".into())),
        Some(StationId("science".into())),
        Some(StationId("engineering".into())),
    ] {
        let p = v.projection_for(viewer.as_ref());
        let got = p.aggregate_fraction.expect("aggregate");
        assert!((got - expected).abs() < 1e-6, "{viewer:?} aggregate {got}");
    }
}

#[test]
fn every_viewer_gets_the_same_destroyed_fraction_including_systems_it_cannot_see() {
    let v = vis_with_destroyed_offsite();
    // One 100-max system destroyed out of 400 total capacity.
    let expected = 0.25;
    for viewer in [
        None,
        Some(StationId("helm".into())),
        Some(StationId("science".into())),
        Some(StationId("engineering".into())),
    ] {
        let p = v.projection_for(viewer.as_ref());
        let got = p.destroyed_fraction.expect("destroyed fraction");
        assert!((got - expected).abs() < 1e-6, "{viewer:?} destroyed {got}");
        // ...and it is genuinely unavailable from the rows they were sent:
        // only science can see the destroyed row at all.
        let visible_destroyed = p.entries.iter().any(|e| e.tier == DamageTier::Destroyed);
        assert_eq!(
            visible_destroyed,
            viewer.as_ref() == Some(&StationId("science".into())),
            "{viewer:?} row visibility must not change with the new scalar"
        );
    }
}

#[test]
fn destroyed_fraction_is_zero_when_nothing_is_destroyed() {
    // Damaged is not destroyed: the fixture's core is at 40/100.
    let v = vis(vec![]);
    assert_eq!(v.destroyed_fraction(), Some(0.0));
}

#[test]
fn destroyed_fraction_is_none_when_the_ship_declares_no_damageable_systems() {
    let v = vis_with(vec![], vec![]);
    assert_eq!(v.destroyed_fraction(), None);
    assert_eq!(v.aggregate_fraction(), None);
}

#[test]
fn the_repair_blackboard_carries_the_destroyed_scalar_to_engineering() {
    let v = vis_with_destroyed_offsite();
    let bb = bb_with_queue(vec![]);
    let eng = StationId("engineering".into());
    let p = v.project_repair_blackboard(Some(&eng), &bb);
    // Engineering cannot see the sensors row, but is told a quarter of the
    // ship's capacity is gone.
    assert!(!ids(&p.system_hull).contains(&"sensors"));
    assert_eq!(p.destroyed_hull_fraction, Some(0.25));
}

#[test]
fn blackboard_projection_filters_hull_but_keeps_dispatch_targets() {
    let v = vis(vec![]);
    let bb = RepairBlackboard {
        teams: vec![],
        travel_duration_secs: 5.0,
        system_hull: vec![
            status("core", 40.0),
            status("helm-radar", 60.0),
            status("sensors", 100.0),
            status("repair", 100.0),
        ],
        damageable_systems: vec![
            SystemId("core".into()),
            SystemId("helm-radar".into()),
            SystemId("sensors".into()),
            SystemId("repair".into()),
        ],
        priority_targets: vec![
            SystemId("core".into()),
            SystemId("helm-radar".into()),
            SystemId("sensors".into()),
            SystemId("repair".into()),
        ],
        queue_depth: vec![],
        aggregate_hull_fraction: None,
        destroyed_hull_fraction: None,
        ..Default::default()
    };
    let eng = StationId("engineering".into());
    let projected = v.project_repair_blackboard(Some(&eng), &bb);
    assert_eq!(ids(&projected.system_hull), vec!["core", "repair"]);
    assert_eq!(
        projected
            .priority_targets
            .iter()
            .map(|system_id| system_id.0.as_str())
            .collect::<Vec<_>>(),
        vec!["core", "repair"]
    );
    // Dispatch targets are ids only — no hull detail — so they stay whole.
    assert_eq!(projected.damageable_systems.len(), 4);
    assert!(projected.aggregate_hull_fraction.is_some());
    assert_eq!(projected.travel_duration_secs, 5.0);
}

// ── queue_depth: the other carrier of exact detail ────────────────────────
//
// `queue_depth` is scoped to *damaged* systems and carries each one's exact
// tier and HP deficit, so leaving it whole made the `system_hull` filter
// cosmetic precisely where it mattered. These pin it to the same rule.

fn queue(entries: &[(&str, f32)]) -> Vec<QueueEntryPreview> {
    entries
        .iter()
        .map(|(station, deficit)| QueueEntryPreview {
            station_id: (*station).into(),
            station_label: (*station).into(),
            tier: DamageTier::Damaged,
            deficit: *deficit,
        })
        .collect()
}

fn bb_with_queue(queue: Vec<QueueEntryPreview>) -> RepairBlackboard {
    RepairBlackboard {
        teams: vec![],
        travel_duration_secs: 5.0,
        system_hull: vec![status("core", 40.0), status("helm-radar", 60.0)],
        damageable_systems: vec![SystemId("core".into()), SystemId("helm-radar".into())],
        queue_depth: queue,
        // Non-None on input so the withholding tests that use this fixture
        // (see `a_non_engineering_viewer_is_sent_the_empty_blackboard_not_a_filtered_one`)
        // genuinely exercise the overwrite-to-`None` behaviour instead of
        // passing vacuously because the input was already `None`.
        aggregate_hull_fraction: Some(0.25),
        destroyed_hull_fraction: Some(0.25),
        ..Default::default()
    }
}

fn queued_stations(bb: &RepairBlackboard) -> Vec<&str> {
    bb.queue_depth
        .iter()
        .map(|e| e.station_id.as_str())
        .collect()
}

#[test]
fn a_station_owner_gets_no_queue_entry_for_a_station_it_does_not_own() {
    let v = vis(vec![]);
    let bb = bb_with_queue(queue(&[("core", 60.0), ("helm", 40.0), ("science", 10.0)]));
    let helm = StationId("helm".into());
    let projected = v.project_repair_blackboard(Some(&helm), &bb);
    assert_eq!(
        queued_stations(&projected),
        vec!["helm"],
        "Helm must not learn the exact tier or HP deficit of core or science"
    );
}

#[test]
fn an_unassigned_viewer_gets_no_queue_entries_at_all() {
    let v = vis(vec![]);
    let bb = bb_with_queue(queue(&[("core", 60.0), ("helm", 40.0)]));
    assert!(v
        .project_repair_blackboard(None, &bb)
        .queue_depth
        .is_empty());
}

#[test]
fn engineering_gets_no_non_core_queue_entry_before_a_team_arrives() {
    let v = vis(vec![]);
    let bb = bb_with_queue(queue(&[("core", 60.0), ("helm", 40.0), ("science", 10.0)]));
    let eng = StationId("engineering".into());
    let projected = v.project_repair_blackboard(Some(&eng), &bb);
    assert_eq!(
        queued_stations(&projected),
        vec!["core"],
        "Engineering may queue-preview Core (and its own) only until a team is on site"
    );
}

#[test]
fn engineering_gets_the_non_core_queue_entry_once_a_team_is_on_site() {
    // helm-radar is a helm-owned system with a team on site.
    let v = vis(vec!["helm-radar"]);
    let bb = bb_with_queue(queue(&[("core", 60.0), ("helm", 40.0), ("science", 10.0)]));
    let eng = StationId("engineering".into());
    let projected = v.project_repair_blackboard(Some(&eng), &bb);
    assert_eq!(queued_stations(&projected), vec!["core", "helm"]);
    assert!(
        !queued_stations(&projected).contains(&"science"),
        "the reveal is per-station, not a blanket unlock"
    );
}

#[test]
fn the_projection_names_every_field_so_none_can_ride_through_unprojected() {
    // A guard against reintroducing `..bb.clone()`. If a new detail-bearing
    // field is added and left unprojected, it shows up here as a field this
    // assertion does not account for and the test needs revisiting.
    let v = vis(vec![]);
    let mut bb = bb_with_queue(queue(&[("helm", 40.0)]));
    bb.external_dispatch_candidate_name = Some("candidate.name".into());
    bb.external_dispatch_candidate_refusal = Some("repair.dispatch.refused.out_of_range".into());
    let eng = StationId("engineering".into());
    let p = v.project_repair_blackboard(Some(&eng), &bb);
    // Projected:
    assert_eq!(ids(&p.system_hull), vec!["core", "repair"]);
    assert!(p.queue_depth.is_empty());
    assert!(p.aggregate_hull_fraction.is_some());
    assert!(p.destroyed_hull_fraction.is_some());
    // Deliberately whole:
    assert_eq!(p.damageable_systems, bb.damageable_systems);
    assert_eq!(p.teams, bb.teams);
    assert_eq!(p.travel_duration_secs, bb.travel_duration_secs);
    // The external field-repair view, whole for the Engineering holder this
    // projection is built for: whole-ship (and whole-TARGET) scalars and
    // ids that name no system of anyone's. Issue #1386 added the last two —
    // which of this seat's own teams went, and how the target it is working
    // is doing — and a seat that chose where to send its teams learns
    // nothing new from being told which one it picked.
    assert_eq!(p.external_dispatch_range, bb.external_dispatch_range);
    assert_eq!(p.external_dispatch_target, bb.external_dispatch_target);
    assert_eq!(
        p.external_dispatch_target_name,
        bb.external_dispatch_target_name
    );
    assert_eq!(
        p.external_dispatch_candidate_name,
        bb.external_dispatch_candidate_name
    );
    assert_eq!(
        p.external_dispatch_candidate_refusal,
        bb.external_dispatch_candidate_refusal
    );
    assert_eq!(p.external_dispatch_refusal, bb.external_dispatch_refusal);
    assert_eq!(p.external_dispatch_team_idx, bb.external_dispatch_team_idx);
    assert_eq!(
        p.external_dispatch_target_condition,
        bb.external_dispatch_target_condition
    );
}

/// …and a recipient who is not the Engineering holder is told nothing about
/// the claim at all: the withheld payload clears the team and the target's
/// condition with the rest of Engineering's working state (issue #1386).
#[test]
fn a_withheld_blackboard_clears_the_field_claim_too() {
    let mut bb = bb_with_queue(queue(&[("helm", 40.0)]));
    bb.external_dispatch_range = Some(800.0);
    bb.external_dispatch_target = Some("ally-1".into());
    bb.external_dispatch_team_idx = Some(1);
    bb.external_dispatch_target_condition = Some(0.5);
    bb.external_dispatch_candidate_name = Some("next.name".into());
    bb.external_dispatch_candidate_refusal = Some("repair.dispatch.refused.out_of_range".into());

    let withheld = super::withheld_repair_blackboard(&bb);
    assert_eq!(withheld.external_dispatch_range, None);
    assert_eq!(withheld.external_dispatch_target, None);
    assert_eq!(withheld.external_dispatch_team_idx, None);
    assert_eq!(withheld.external_dispatch_target_condition, None);
    assert_eq!(withheld.external_dispatch_candidate_name, None);
    assert_eq!(withheld.external_dispatch_candidate_refusal, None);
    assert_eq!(
        withheld.travel_duration_secs, bb.travel_duration_secs,
        "the ship constant is kept, so a console shown this again renders no nonsense bar"
    );
}

#[test]
fn only_the_engineering_holder_receives_the_repair_blackboard() {
    let v = vis(vec![]);
    assert!(v.may_receive_repair_blackboard(Some(&StationId("engineering".into()))));
    for other in ["helm", "science", "captain"] {
        assert!(
            !v.may_receive_repair_blackboard(Some(&StationId(other.into()))),
            "{other} must not be sent the repair blackboard"
        );
    }
    assert!(!v.may_receive_repair_blackboard(None));
}

#[test]
fn a_non_engineering_viewer_is_sent_the_empty_blackboard_not_a_filtered_one() {
    // Withholding has to be an overwrite, not silence: a player who has just
    // moved off Engineering still has the previous payload on their phone.
    let v = vis(vec![]);
    let bb = SystemBlackboard::Repair(bb_with_queue(queue(&[("core", 60.0)])));
    let helm = Some(StationId("helm".into()));
    let SystemBlackboard::Repair(out) = project_blackboard_for_token(Some(&v), helm.as_ref(), &bb)
    else {
        unreachable!()
    };
    assert!(out.system_hull.is_empty());
    assert!(out.queue_depth.is_empty());
    assert!(out.damageable_systems.is_empty());
    assert!(out.teams.is_empty());
    assert!(out.aggregate_hull_fraction.is_none());
    assert!(out.destroyed_hull_fraction.is_none());
}

#[test]
fn a_station_change_invalidates_the_cached_projection() {
    let mut cache = LastVisibleRepairBlackboard::default();
    let viewers = vec![("eng".to_string(), Some(StationId("engineering".into())))];
    assert!(
        cache.stations_changed(&viewers),
        "a token never seen before counts as changed"
    );
    cache.record_stations(&viewers);
    assert!(!cache.stations_changed(&viewers));

    let moved = vec![("eng".to_string(), Some(StationId("helm".into())))];
    assert!(
        cache.stations_changed(&moved),
        "moving station must invalidate the projection even though the \
             internal blackboard did not change"
    );
}

// ── AC#3 / AC#4: the on-site gate driven by the real state machine ────────
//
// These exercise `RepairTeams` transitions rather than hand-set `on_site`
// lists, so travel/arrival/recall are asserted against the code that
// actually moves teams around.

fn hull_with(entries: &[(&str, f32)]) -> SystemHull {
    SystemHull::from_config(
        &entries
            .iter()
            .map(|(id, hp)| (SystemId((*id).into()), *hp))
            .collect::<Vec<_>>(),
    )
}

fn vis_with_teams(teams: &RepairTeams) -> HullVisibility {
    let mut v = vis(vec![]);
    v.on_site = teams.on_site_systems().cloned().collect();
    v
}

fn eng() -> StationId {
    StationId("engineering".into())
}

#[test]
fn travelling_team_does_not_reveal_non_core_detail_to_engineering() {
    let mut teams = RepairTeams::new(1);
    let mut hull = hull_with(&[("helm-radar", 100.0)]);
    hull.set_hp(&SystemId("helm-radar".into()), 40.0);
    teams.dispatch(0, SystemId("helm-radar".into()), "Helm Radar".into());
    // Part-way through travel: en route is not on site.
    teams.tick(1.0, &mut hull, None);

    let v = vis_with_teams(&teams);
    assert!(!v.can_see(Some(&eng()), &SystemId("helm-radar".into())));
    assert_eq!(ids(&v.entries_for(Some(&eng()))), vec!["core", "repair"]);
}

#[test]
fn arrival_reveals_non_core_detail_to_engineering() {
    let mut teams = RepairTeams::new(1);
    let mut hull = hull_with(&[("helm-radar", 100.0)]);
    hull.set_hp(&SystemId("helm-radar".into()), 40.0);
    teams.dispatch(0, SystemId("helm-radar".into()), "Helm Radar".into());
    assert!(!vis_with_teams(&teams).can_see(Some(&eng()), &SystemId("helm-radar".into())));

    // Travel completes (default travel_duration is 5s) → Repairing.
    teams.tick(6.0, &mut hull, None);
    let v = vis_with_teams(&teams);
    assert!(
        v.can_see(Some(&eng()), &SystemId("helm-radar".into())),
        "a team on site must reveal exact detail for the system it is at"
    );
    assert_eq!(
        ids(&v.entries_for(Some(&eng()))),
        vec!["core", "helm-radar", "repair"]
    );
}

#[test]
fn recall_before_arrival_never_reveals_detail() {
    let mut teams = RepairTeams::new(1);
    let mut hull = hull_with(&[("helm-radar", 100.0)]);
    hull.set_hp(&SystemId("helm-radar".into()), 40.0);
    teams.dispatch(0, SystemId("helm-radar".into()), "Helm Radar".into());
    teams.tick(2.0, &mut hull, None);
    // Dispatching to the same system while Travelling is a recall.
    teams.dispatch(0, SystemId("helm-radar".into()), "Helm Radar".into());

    // Walk the whole return leg: at no point may the detail appear.
    for _ in 0..10 {
        teams.tick(1.0, &mut hull, None);
        assert!(
            !vis_with_teams(&teams).can_see(Some(&eng()), &SystemId("helm-radar".into())),
            "a recalled team never arrives, so it must never reveal detail"
        );
    }
}

#[test]
fn detail_is_withdrawn_again_once_the_team_leaves() {
    let mut teams = RepairTeams::new(1);
    let mut hull = hull_with(&[("helm-radar", 100.0)]);
    hull.set_hp(&SystemId("helm-radar".into()), 40.0);
    teams.dispatch(0, SystemId("helm-radar".into()), "Helm Radar".into());
    teams.tick(6.0, &mut hull, None);
    assert!(vis_with_teams(&teams).can_see(Some(&eng()), &SystemId("helm-radar".into())));

    // Recall from Repairing → Returning: detail goes away immediately.
    teams.dispatch(0, SystemId("helm-radar".into()), "Helm Radar".into());
    assert!(
        !vis_with_teams(&teams).can_see(Some(&eng()), &SystemId("helm-radar".into())),
        "detail must be withdrawn the moment the team stops being on site"
    );
}

#[test]
fn on_site_reveal_does_not_extend_to_other_systems_of_that_station() {
    // helm-radar is on site; the visibility grant is per-system, so a
    // second helm-owned system stays hidden from Engineering.
    let mut v = vis(vec!["helm-radar"]);
    v.entries.push(status("helm-thrust", 10.0));
    v.owner_of.insert(
        SystemId("helm-thrust".into()),
        Some(StationId("helm".into())),
    );
    assert!(!v.can_see(Some(&eng()), &SystemId("helm-thrust".into())));
}

// ── World-level: the actual wire path, live and on reconnect ─────────────

/// A minimal but *real* ship config: helm owns `helm-radar`, engineering
/// owns `repair`, and `core` is declared as a hull entry with no owning
/// `[[system]]` — exactly the ownerless-bucket shape the shipped hulls use.
fn world_ship_config() -> ShipConfig {
    ShipConfig::from_toml(
        r#"
[[station]]
id = "helm"
name = "Helm"
description = "Flying."
rank = "Ltn."

[[station]]
id = "engineering"
name = "Engineering"
description = "Fixing."
rank = "Ltn."

[[system]]
id = "helm-radar"
kind = "helm_radar"
station = "helm"

[[system]]
id = "repair"
kind = "repair"
station = "engineering"
"#,
        &["helm_radar", "repair"],
    )
    .expect("test ship config must parse")
}

/// Spawn a LocalShip carrying the config/hull/teams the projection reads,
/// plus two connected sessions: `eng` at Engineering, `pilot` at Helm.
fn world_app(teams: RepairTeams) -> App {
    world_app_with_hp(teams, &[("helm-radar", 30.0), ("core", 60.0)])
}

/// [`world_app`] with the post-damage HP of each system spelled out, so a
/// test can author a *destroyed* system (0 HP) the default fixture lacks.
fn world_app_with_hp(teams: RepairTeams, hp: &[(&str, f32)]) -> App {
    use crate::entities::spawner::EntitySystemHull;
    use crate::server_app::LocalShip;
    use crate::ship_plugin::ShipConfigComponent;

    let mut app = App::new();
    register_hull_replication_lifecycle(&mut app);
    app.init_resource::<crate::server_app::SimOutbox>();

    let mut sessions = crate::lobby::session::SessionManager::new();
    sessions.register("eng".into(), "Bob".into()).unwrap();
    sessions.register("pilot".into(), "Ada".into()).unwrap();
    sessions.set_station("eng", Some(StationId("engineering".into())));
    sessions.set_station("pilot", Some(StationId("helm".into())));
    app.insert_resource(Sessions(sessions));

    let mut hull = hull_with(&[("core", 100.0), ("helm-radar", 100.0), ("repair", 100.0)]);
    for (id, current) in hp {
        hull.set_hp(&SystemId((*id).into()), *current);
    }

    app.world_mut().spawn((
        crate::server_app::Ship,
        LocalShip,
        ShipConfigComponent(world_ship_config()),
        EntitySystemHull(hull),
        super::super::server::ShipRepairTeams(teams),
    ));
    app
}

/// One recipient's `SystemHullUpdate`: the rows it may see, plus both
/// whole-ship scalars it is entitled to regardless of those rows.
struct SentHull {
    token: String,
    rows: Vec<String>,
    aggregate: Option<f32>,
    destroyed: Option<f32>,
}

fn sent_hull(app: &mut App) -> Vec<SentHull> {
    push_hull_updates(app.world_mut());
    app.world()
        .resource::<crate::server_app::SimOutbox>()
        .iter()
        .filter_map(|(target, msg)| match (target, msg) {
            (
                Target::Token(token),
                ServerMessage::SystemHullUpdate {
                    entries,
                    aggregate_fraction,
                    destroyed_fraction,
                },
            ) => Some(SentHull {
                token: token.clone(),
                rows: entries.iter().map(|e| e.system_id.0.clone()).collect(),
                aggregate: *aggregate_fraction,
                destroyed: *destroyed_fraction,
            }),
            _ => None,
        })
        .collect()
}

fn for_token<'a>(sent: &'a [SentHull], token: &str) -> &'a SentHull {
    sent.iter()
        .find(|s| s.token == token)
        .unwrap_or_else(|| panic!("expected a SystemHullUpdate for {token}"))
}

fn rows_for<'a>(sent: &'a [SentHull], token: &str) -> &'a Vec<String> {
    &for_token(sent, token).rows
}

fn reconnected_hull(app: &mut App, token: &str) -> SentHull {
    crate::core::broadcast::reconnect_registered_replication(app.world_mut(), token)
        .into_iter()
        .find_map(|message| match message {
            ServerMessage::SystemHullUpdate {
                entries,
                aggregate_fraction,
                destroyed_fraction,
            } => Some(SentHull {
                token: token.to_string(),
                rows: entries.into_iter().map(|entry| entry.system_id.0).collect(),
                aggregate: aggregate_fraction,
                destroyed: destroyed_fraction,
            }),
            _ => None,
        })
        .unwrap_or_else(|| panic!("registered Hull reconnect projection missing for {token}"))
}

fn assert_registered_reconnect_matches_live(app: &mut App, token: &str) {
    let live = sent_hull(app);
    let live = for_token(&live, token);
    let cached_before = app.world().resource::<LastBroadcastHull>().0.clone();

    let reconnect = reconnected_hull(app, token);

    assert_eq!(
        reconnect.rows, live.rows,
        "reconnect must reproduce the live row visibility for {token}"
    );
    assert_eq!(reconnect.aggregate, live.aggregate);
    assert_eq!(reconnect.destroyed, live.destroyed);
    assert_eq!(
        app.world().resource::<LastBroadcastHull>().0,
        cached_before,
        "a targeted reconnect must not mutate any recipient's live Hull cache"
    );
}

#[test]
fn repair_plugin_registers_hull_lifecycle_and_state_census_locally() {
    let mut app = App::new();
    app.add_plugins(crate::console::repair::server::RepairPlugin);
    crate::server_app::register_blackboard_replication_lifecycle(&mut app);

    assert_eq!(
        app.world()
            .resource::<crate::core::broadcast::ReplicationLifecycleRegistry>()
            .keys()
            .collect::<Vec<_>>(),
        vec!["blackboards", "hull"],
        "owner registration must retain stable lexical lifecycle ordering"
    );
    assert_eq!(
        app.world()
            .resource::<crate::authoritative::StateCensus>()
            .get(std::any::type_name::<LastBroadcastHull>()),
        Some((
            crate::authoritative::StateClass::Cache,
            "digest-exclusion-classes"
        )),
        "the owner must keep the Hull cache classified in StateCensus"
    );
}

#[test]
fn registered_run_reset_clears_the_hull_delta_cache() {
    let mut app = world_app(RepairTeams::new(1));
    let sent = sent_hull(&mut app);
    assert_eq!(sent.len(), 2);
    assert_eq!(app.world().resource::<LastBroadcastHull>().0.len(), 2);

    crate::core::broadcast::reset_registered_replication(app.world_mut());

    assert!(app.world().resource::<LastBroadcastHull>().0.is_empty());
}

#[test]
fn live_broadcast_gives_engineering_core_only_with_no_team_on_site() {
    let mut app = world_app(RepairTeams::new(1));
    let sent = sent_hull(&mut app);
    assert_eq!(
        rows_for(&sent, "eng"),
        &vec!["core".to_string(), "repair".to_string()]
    );
    assert!(
        !rows_for(&sent, "eng").contains(&"helm-radar".to_string()),
        "Engineering must not receive non-Core detail with no team on site"
    );
}

#[test]
fn live_broadcast_gives_a_station_owner_only_its_own_systems() {
    let mut app = world_app(RepairTeams::new(1));
    let sent = sent_hull(&mut app);
    assert_eq!(rows_for(&sent, "pilot"), &vec!["helm-radar".to_string()]);
}

#[test]
fn every_recipient_receives_the_same_ship_wide_aggregate() {
    let mut app = world_app(RepairTeams::new(1));
    let sent = sent_hull(&mut app);
    // 60 + 30 + 100 out of 300 — spans systems no single recipient sees.
    let expected = 190.0 / 300.0;
    for s in &sent {
        let token = &s.token;
        let got = s
            .aggregate
            .unwrap_or_else(|| panic!("{token} got no aggregate"));
        assert!((got - expected).abs() < 1e-6, "{token} aggregate {got}");
    }
}

#[test]
fn every_recipient_receives_the_same_destroyed_capability_share() {
    // helm-radar destroyed: helm-owned, no team on site, so Engineering
    // never receives its row — yet must still be told a third of the ship's
    // capacity is gone (issue #1014).
    let mut app = world_app_with_hp(RepairTeams::new(1), &[("helm-radar", 0.0)]);
    let sent = sent_hull(&mut app);
    assert!(
        !rows_for(&sent, "eng").contains(&"helm-radar".to_string()),
        "the destroyed system must stay hidden from Engineering"
    );
    let expected = 100.0 / 300.0;
    for s in &sent {
        let token = &s.token;
        let got = s
            .destroyed
            .unwrap_or_else(|| panic!("{token} got no destroyed fraction"));
        assert!((got - expected).abs() < 1e-6, "{token} destroyed {got}");
        // The aggregate is denominator-complete over the same entries.
        let agg = s.aggregate.expect("aggregate");
        assert!(
            (agg - 200.0 / 300.0).abs() < 1e-6,
            "{token} aggregate {agg}"
        );
    }
}

#[test]
fn an_undamaged_ship_reports_a_zero_destroyed_share_not_none() {
    let mut app = world_app_with_hp(RepairTeams::new(1), &[]);
    for s in &sent_hull(&mut app) {
        assert_eq!(s.destroyed, Some(0.0), "{} destroyed share", s.token);
    }
}

#[test]
fn live_broadcast_reveals_non_core_detail_once_a_team_is_on_site() {
    let mut teams = RepairTeams::new(1);
    let mut hull = hull_with(&[("helm-radar", 100.0)]);
    hull.set_hp(&SystemId("helm-radar".into()), 30.0);
    teams.dispatch(0, SystemId("helm-radar".into()), "Radar".into());
    teams.tick(6.0, &mut hull, None); // arrive

    let mut app = world_app(teams);
    let sent = sent_hull(&mut app);
    assert!(rows_for(&sent, "eng").contains(&"helm-radar".to_string()));
    // Helm's own view is unchanged by someone else's team arriving.
    assert_eq!(rows_for(&sent, "pilot"), &vec!["helm-radar".to_string()]);
}

#[test]
fn registered_reconnect_honours_off_site_visibility_without_mutating_caches() {
    for token in ["eng", "pilot"] {
        let mut app = world_app(RepairTeams::new(1));
        assert_registered_reconnect_matches_live(&mut app, token);
    }
}

#[test]
fn registered_reconnect_honours_on_site_visibility_without_mutating_caches() {
    let mut teams = RepairTeams::new(1);
    let mut hull = hull_with(&[("helm-radar", 100.0)]);
    hull.set_hp(&SystemId("helm-radar".into()), 30.0);
    teams.dispatch(0, SystemId("helm-radar".into()), "Radar".into());
    teams.tick(6.0, &mut hull, None);

    let mut app = world_app(teams);
    assert_registered_reconnect_matches_live(&mut app, "eng");
    let reconnect = reconnected_hull(&mut app, "eng");
    assert!(
        reconnect.rows.contains(&"helm-radar".to_string()),
        "an on-site Engineering reconnect must receive the detail live publication reveals"
    );
}

#[test]
fn scheduled_reconnect_projects_real_repair_and_hull_for_duplicate_private_requests() {
    use crate::core::messages::DeliveryClass;
    use crate::server_app::{ShipSystemBlackboards, SimOutbox};
    let mut app = world_app(RepairTeams::new(1));
    crate::server_app::register_blackboard_replication_lifecycle(&mut app);
    let entity = app
        .world_mut()
        .query_filtered::<Entity, With<crate::server_app::LocalShip>>()
        .single(app.world())
        .unwrap();
    let repair = RepairBlackboard {
        system_hull: vec![status("core", 60.0), status("helm-radar", 30.0)],
        damageable_systems: vec![SystemId("core".into()), SystemId("helm-radar".into())],
        aggregate_hull_fraction: Some(0.3),
        ..Default::default()
    };
    app.world_mut()
        .entity_mut(entity)
        .insert(ShipSystemBlackboards(
            [(SystemId("repair".into()), SystemBlackboard::Repair(repair))]
                .into_iter()
                .collect(),
        ));
    // Populate the real shared Hull delta cache before targeted projection.
    let live = sent_hull(&mut app);
    assert_eq!(rows_for(&live, "pilot"), &vec!["helm-radar".to_owned()]);
    app.world_mut().resource_mut::<SimOutbox>().drain();
    let cache_before = app.world().resource::<LastBroadcastHull>().0.clone();
    let boards_before = app
        .world()
        .get::<ShipSystemBlackboards>(entity)
        .unwrap()
        .0
        .clone();
    app.finish();
    let mut observed = Vec::new();
    for arrived in [false, true] {
        if arrived {
            let world = app.world_mut();
            let mut query = world.query::<(
                &mut crate::entities::spawner::EntitySystemHull,
                &mut super::super::server::ShipRepairTeams,
            )>();
            let (mut hull, mut teams) = query.get_mut(world, entity).unwrap();
            teams
                .0
                .dispatch(0, SystemId("helm-radar".into()), "Radar".into());
            teams.0.tick(6.0, &mut hull.0, None);
            assert!(teams.0.on_site_systems().any(|id| id.0 == "helm-radar"));
        }
        crate::core::broadcast::reconnect::test_reconnect_welcomes(
            &mut app,
            &["eng", "pilot", "eng", "stranger"],
        );
        app.world_mut().run_schedule(FixedUpdate);
        let messages = app.world_mut().resource_mut::<SimOutbox>().drain();
        assert_eq!(messages.len(), 8, "two lexical owners for every occurrence");
        for (pair, token) in messages
            .chunks_exact(2)
            .zip(["eng", "pilot", "eng", "stranger"])
        {
            for row in pair {
                assert_eq!(row.target, Target::Token(token.into()));
                assert_eq!(row.delivery, DeliveryClass::Snapshot);
            }
            let ServerMessage::BlackboardUpdate { updates, .. } = &pair[0].message else {
                panic!("blackboards precede hull")
            };
            let SystemBlackboard::Repair(board) = &updates[0].1 else {
                panic!("real Repair board")
            };
            assert_eq!(
                board
                    .system_hull
                    .iter()
                    .any(|r| r.system_id.0 == "helm-radar"),
                token == "eng" && arrived
            );
            if token != "eng" {
                assert!(board.system_hull.is_empty());
                assert!(board.damageable_systems.is_empty());
                assert!(board.aggregate_hull_fraction.is_none());
            }
            let ServerMessage::SystemHullUpdate { entries, .. } = &pair[1].message else {
                panic!("Hull owner second")
            };
            assert_eq!(
                entries.iter().any(|r| r.system_id.0 == "helm-radar"),
                token == "pilot" || (token == "eng" && arrived)
            );
            if token == "stranger" {
                assert!(entries.is_empty());
            }
        }
        observed.push(
            serde_json::to_value(messages.iter().map(|r| &r.message).collect::<Vec<_>>()).unwrap(),
        );
        assert_eq!(
            app.world().resource::<LastBroadcastHull>().0,
            cache_before,
            "all recipients' live cache stays unchanged"
        );
        assert!(app
            .world()
            .resource::<LastVisibleRepairBlackboard>()
            .projections
            .is_empty());
        assert_eq!(
            app.world().get::<ShipSystemBlackboards>(entity).unwrap().0,
            boards_before,
            "projection cannot rewrite its source"
        );
    }
    assert_ne!(
        observed[0], observed[1],
        "real team arrival changes permitted detail"
    );
}

#[test]
fn repair_blackboard_fans_out_per_token_while_others_stay_broadcast() {
    let v = vis(vec![]);
    let repair_bb = SystemBlackboard::Repair(RepairBlackboard {
        teams: vec![],
        travel_duration_secs: 5.0,
        system_hull: vec![
            status("core", 40.0),
            status("helm-radar", 60.0),
            status("repair", 100.0),
        ],
        damageable_systems: vec![
            SystemId("core".into()),
            SystemId("helm-radar".into()),
            SystemId("repair".into()),
        ],
        queue_depth: vec![],
        // Non-None on input so the "pilot" assertions below genuinely
        // exercise `withheld_repair_blackboard` overwriting to `None`
        // rather than passing vacuously on an already-`None` input.
        aggregate_hull_fraction: Some(0.25),
        destroyed_hull_fraction: Some(0.25),
        ..Default::default()
    });
    let viewers = vec![
        ("eng".to_string(), Some(StationId("engineering".into()))),
        ("pilot".to_string(), Some(StationId("helm".into()))),
    ];
    let mut cache = LastVisibleRepairBlackboard::default();
    let out = project_repair_blackboards(
        vec![(SystemId("repair".into()), repair_bb)],
        Some(&v),
        &viewers,
        &mut cache,
    );

    assert!(
        !out.iter().any(|(t, _)| matches!(t, Target::All)),
        "a repair blackboard carrying hull detail must never go to Target::All"
    );
    for (target, msg) in &out {
        let (Target::Token(token), ServerMessage::BlackboardUpdate { updates, .. }) = (target, msg)
        else {
            panic!("expected a token-targeted BlackboardUpdate")
        };
        let SystemBlackboard::Repair(bb) = &updates[0].1 else {
            unreachable!()
        };
        let rows = ids(&bb.system_hull);
        match token.as_str() {
            "eng" => {
                assert_eq!(rows, vec!["core", "repair"]);
                assert!(bb.aggregate_hull_fraction.is_some());
                assert!(bb.destroyed_hull_fraction.is_some());
            }
            // Helm is not Engineering: the repair blackboard is the
            // Engineering console's payload and nothing else renders it,
            // so Helm receives the empty one — not a filtered one.
            "pilot" => {
                assert!(rows.is_empty());
                assert!(bb.damageable_systems.is_empty());
                assert!(bb.teams.is_empty());
                assert!(bb.queue_depth.is_empty());
                assert!(bb.aggregate_hull_fraction.is_none());
                assert!(bb.destroyed_hull_fraction.is_none());
            }
            other => panic!("unexpected recipient {other}"),
        }
    }

    // Unchanged projections are not re-sent.
    assert!(cache.projections.contains_key("eng"));
}

#[test]
fn reconnect_resync_gives_an_unknown_token_no_detail() {
    let mut app = world_app(RepairTeams::new(1));
    let ServerMessage::SystemHullUpdate { entries, .. } =
        hull_update_for_token(app.world_mut(), "stranger").expect("resync payload")
    else {
        unreachable!()
    };
    assert!(entries.is_empty());
}

#[test]
fn ship_with_no_engineering_station_reveals_nothing_extra() {
    let v = HullVisibility::new(
        vec![status("core", 50.0)],
        [(SystemId("core".into()), None)].into_iter().collect(),
        None,
        vec![SystemId("core".into())],
    );
    assert!(v.entries_for(Some(&StationId("pilot".into()))).is_empty());
}
