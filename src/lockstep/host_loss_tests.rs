use super::*;
use bevy::{app::Update, prelude::App};

fn gm_loss_app(
    gms: &[(HostSlot, &str)],
    lost: HostSlot,
) -> (App, crate::gm_puppet::StationPuppetTarget) {
    let ship_slot = HostSlot(1);
    let participants = std::iter::once(ship_slot)
        .chain(gms.iter().map(|(slot, _)| *slot))
        .collect();
    let roster = super::super::FleetRoster::with_participants_and_gms(
        vec![super::super::FleetShip::new(ship_slot)],
        participants,
        gms.iter()
            .map(|(host, operator_id)| super::super::FleetGm {
                host: *host,
                operator_id: (*operator_id).into(),
            })
            .collect(),
        ship_slot,
        ship_slot,
    )
    .unwrap();
    let config = crate::ship::config::ShipConfig::from_toml(
        r#"
[[station]]
id = "helm"
name = "Helm"
description = ""
rank = ""
console = "helm.html"

[[station.rating]]
name = "Manual"
automated_systems = []

[[system]]
id = "helm-thrust"
kind = "helm_thrust"
station = "helm"
"#,
        &["helm_thrust"],
    )
    .unwrap();
    let station = crate::core::messages::StationId("helm".into());
    let target = crate::gm_puppet::StationPuppetTarget::new(
        crate::command_admission::ShipKey("player-1".into()),
        station.clone(),
    );
    let mut puppets = crate::gm_puppet::StationPuppets::default();
    let mut activity = crate::gm_puppet::StationPuppetActivity::default();
    for (sequence, (slot, operator)) in gms.iter().enumerate() {
        puppets.set_operator(target.clone(), (*operator).into(), true);
        activity.push(crate::gm_puppet::StationPuppetActivityEntry {
            tick: 6,
            order: crate::gm_action::GmActionOrder::new(*slot, sequence as u64 + 1),
            operator_id: (*operator).into(),
            ship: target.ship.clone(),
            station: station.clone(),
            target: crate::core::messages::SystemId("helm-thrust".into()),
            action: "SetThrust".into(),
        });
    }
    let mut pending = PendingHostLoss::default();
    pending.observe(lost, 7);
    let mut ratings = crate::ship_plugin::ActiveStationRatings::default();
    ratings.0.insert(station, rating::BACKFILL_RATING.into());
    let mut sources = crate::ship_plugin::ShipSystemControlSources::default();
    sources.0.set(
        crate::core::messages::SystemId("helm-thrust".into()),
        crate::ship::control_source::ControlSource::Human,
    );

    let mut app = App::new();
    app.insert_resource(crate::sim_tick::SimTick(7))
        .insert_resource(pending)
        .insert_resource(roster)
        .insert_resource(puppets)
        .insert_resource(activity)
        .init_resource::<crate::gm_puppet::PreviousStationPuppetTargets>()
        .add_systems(Update, apply_host_loss_backfill);
    app.world_mut().spawn((
        crate::server_app::Ship,
        crate::entities::spawner::EntityUuid("player-1".into()),
        crate::ship_plugin::ShipConfigComponent(config),
        sources,
        ratings,
        super::super::FleetSlotOf(ship_slot),
    ));
    (app, target)
}

/// The agreed tick is the first tick past the lost host's watermark, and it
/// is the same on every host because it is a function of that watermark
/// alone — never of who noticed the loss first.
#[test]
fn the_agreed_tick_is_the_first_uncovered_tick() {
    assert_eq!(agreed_loss_tick(0), 1);
    assert_eq!(agreed_loss_tick(417), 418);
    assert_eq!(
        agreed_loss_tick(u64::MAX),
        u64::MAX,
        "saturating: a watermark at the end of the line names itself rather \
             than wrapping to re-apply the flip on tick 1"
    );
}

/// Two reports of one loss, and one report twice, converge on a single
/// transition at a single tick — the max of everything anyone derived.
#[test]
fn reports_converge_on_one_transition_at_one_tick() {
    let mut pending = PendingHostLoss::default();
    // First report from one survivor.
    assert!(
        pending.observe(HostSlot(3), 100),
        "a first report changes the picture"
    );
    // A duplicate at the same tick changes nothing and asks for no re-broadcast.
    assert!(!pending.observe(HostSlot(3), 100));
    // A second survivor's report derived a higher tick (it had heard one more
    // of the lost host's frames): the agreement RAISES to it, and that is
    // worth re-broadcasting so the first survivor catches up.
    assert!(pending.observe(HostSlot(3), 102));
    // A lower, later-arriving report cannot walk it back.
    assert!(!pending.observe(HostSlot(3), 99));
    assert_eq!(pending.agreed_tick(HostSlot(3)), Some(102));

    // Nothing applies before its tick…
    assert!(pending.drain_due(101).is_empty());
    // …and exactly one record applies at it, whatever the report history.
    let due = pending.drain_due(102);
    assert_eq!(
        due,
        vec![HostLossRecord {
            slot: HostSlot(3),
            tick: 102
        }]
    );
    assert_eq!(pending.records(), due.as_slice());

    // Once applied, a late duplicate is inert — the flip cannot be re-agreed.
    assert!(!pending.observe(HostSlot(3), 200));
    assert!(pending.is_applied(HostSlot(3)));
    assert!(pending.drain_due(300).is_empty());
    assert_eq!(pending.records().len(), 1, "applying once records once");
}

#[test]
fn a_private_candidate_retains_loss_until_commit_installs_roster_then_drains_once() {
    let mut app = App::new();
    app.init_resource::<PendingHostLoss>()
        .add_systems(Update, apply_host_loss_backfill);
    app.world_mut()
        .resource_mut::<PendingHostLoss>()
        .observe(HostSlot(3), 0);

    app.update();
    let pending = app.world().resource::<PendingHostLoss>();
    assert_eq!(pending.pending_len(), 1);
    assert!(!pending.is_applied(HostSlot(3)));

    // Digest-proven GmJoinCommit is the point at which the candidate gains
    // the authoritative roster and can project an ordinary host loss.
    app.world_mut()
        .insert_resource(super::super::FleetRoster::default());
    app.update();
    let pending = app.world().resource::<PendingHostLoss>();
    assert_eq!(pending.pending_len(), 0);
    assert!(pending.is_applied(HostSlot(3)));
    assert_eq!(pending.records().len(), 1);

    app.update();
    assert_eq!(
        app.world().resource::<PendingHostLoss>().records().len(),
        1,
        "the retained transition is projected exactly once"
    );
}

/// A due drain is ordered by `(tick, slot)`, so two survivors that lose the
/// same pair of hosts record the transitions in the same order.
#[test]
fn a_multi_loss_drain_is_ordered_by_tick_then_slot() {
    let mut pending = PendingHostLoss::default();
    pending.observe(HostSlot(4), 50);
    pending.observe(HostSlot(2), 50);
    pending.observe(HostSlot(3), 40);

    let due = pending.drain_due(100);
    assert_eq!(
        due,
        vec![
            HostLossRecord {
                slot: HostSlot(3),
                tick: 40
            },
            HostLossRecord {
                slot: HostSlot(2),
                tick: 50
            },
            HostLossRecord {
                slot: HostSlot(4),
                tick: 50
            },
        ],
        "earlier tick first, then lower slot — a total order every survivor \
             computes the same way"
    );
}

#[test]
fn single_gm_loss_releases_takeover_restores_backfill_and_clears_activity() {
    let (mut app, target) = gm_loss_app(&[(HostSlot(2), "gm-1")], HostSlot(2));
    app.update();

    assert!(!app
        .world()
        .resource::<crate::gm_puppet::StationPuppets>()
        .is_active(&target));
    assert!(app
        .world()
        .resource::<crate::gm_puppet::StationPuppetActivity>()
        .entries()
        .is_empty());
    let sources = app
        .world_mut()
        .query::<&crate::ship_plugin::ShipSystemControlSources>()
        .single(app.world())
        .unwrap();
    assert_eq!(
        sources
            .0
            .source_for(&crate::core::messages::SystemId("helm-thrust".into())),
        crate::ship::control_source::ControlSource::Ai,
    );
}

#[test]
fn one_equal_gm_loss_preserves_the_surviving_takeover() {
    let (mut app, target) =
        gm_loss_app(&[(HostSlot(2), "gm-1"), (HostSlot(3), "gm-2")], HostSlot(2));
    app.update();

    assert_eq!(
        app.world()
            .resource::<crate::gm_puppet::StationPuppets>()
            .operators(&target),
        &["gm-2"],
    );
    assert_eq!(
        app.world()
            .resource::<crate::gm_puppet::StationPuppetActivity>()
            .entries()
            .iter()
            .map(|entry| entry.operator_id.as_str())
            .collect::<Vec<_>>(),
        vec!["gm-2"],
    );
    let sources = app
        .world_mut()
        .query::<&crate::ship_plugin::ShipSystemControlSources>()
        .single(app.world())
        .unwrap();
    assert_eq!(
        sources
            .0
            .source_for(&crate::core::messages::SystemId("helm-thrust".into())),
        crate::ship::control_source::ControlSource::Human,
    );
}

#[test]
fn recovered_gm_binding_can_retake_the_backfill_station() {
    use bevy::ecs::system::RunSystemOnce;

    let (mut app, target) = gm_loss_app(&[(HostSlot(2), "gm-1")], HostSlot(2));
    app.update();
    assert_eq!(
        app.world()
            .resource::<super::super::FleetRoster>()
            .gm_operator(HostSlot(2)),
        Some("gm-1"),
        "the frozen binding survives transport loss for authenticated recovery",
    );
    let action = crate::gm_action::GmAction::SetStationPuppet {
        ship: target.ship.clone(),
        station: target.station.clone(),
        active: true,
    };
    assert_eq!(
        crate::gm_puppet::validate_station_action_in_world(app.world_mut(), &action, "gm-1",),
        Ok(()),
    );
    app.world_mut()
        .resource_mut::<crate::gm_puppet::StationPuppets>()
        .set_operator(target.clone(), "gm-1".into(), true);
    app.world_mut()
        .run_system_once(crate::gm_puppet::reconcile_station_puppet_control)
        .unwrap();
    assert!(app
        .world()
        .resource::<crate::gm_puppet::StationPuppets>()
        .is_operated_by(&target, "gm-1"));
    let sources = app
        .world_mut()
        .query::<&crate::ship_plugin::ShipSystemControlSources>()
        .single(app.world())
        .unwrap();
    assert_eq!(
        sources
            .0
            .source_for(&crate::core::messages::SystemId("helm-thrust".into())),
        crate::ship::control_source::ControlSource::Human,
    );
}
