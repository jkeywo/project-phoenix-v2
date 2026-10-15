use super::*;
use crate::ship::system_registry::{tactical_station_key, TACTICAL_STATION_ID};
use phoenix_model::weapon_targets_view::{
    WeaponGroupAssignmentView, WeaponTargetAssignmentView, WeaponTargetReference as Ref,
};

fn setup() -> (App, Entity, String) {
    let mut app = fixture();
    let alias = observe(&mut app, "tactical-radar", 0, 1.0);
    let ship = local_ship(&mut app);
    {
        let mut config = app
            .world_mut()
            .get_mut::<crate::ship::components::ShipConfigComponent>(ship)
            .unwrap();
        let science = config
            .0
            .system(&SystemId("sensor-radar".into()))
            .unwrap()
            .console
            .clone()
            .unwrap();
        config
            .0
            .consoles
            .iter_mut()
            .find(|console| console.id == science)
            .unwrap()
            .feeds
            .push(phoenix_model::console::ConsoleFeed {
                source: tactical_station_key(),
                kind: phoenix_model::console::ConsoleObservationKind::Weapons,
            });
        // Both Consoles remain on their actual Stations. A foreign bank is not
        // disclosed just because the producing aggregate knows every bank.
        let foreign = config
            .0
            .system(&SystemId("repair".into()))
            .unwrap()
            .console
            .clone()
            .unwrap();
        config
            .0
            .systems
            .iter_mut()
            .find(|system| system.id.0 == "phaser-aft")
            .unwrap()
            .console = Some(foreign);
    }
    let assignment = WeaponTargetAssignmentView {
        group: Some(WeaponGroupAssignmentView {
            group: "main-beams".into(),
            target: Ref::Target {
                contact: alias.clone(),
            },
        }),
        individual: Some(Ref::Hold),
    };
    app.world_mut()
        .get_mut::<crate::server_app::ShipSystemBlackboards>(ship)
        .unwrap()
        .0
        .insert(
            tactical_station_key(),
            SystemBlackboard::Weapons(crate::core::messages::WeaponsBlackboard {
                assignments: [
                    (SystemId("phaser-fore".into()), assignment.clone()),
                    (SystemId("phaser-aft".into()), assignment),
                ]
                .into(),
                ..Default::default()
            }),
        );
    (app, ship, alias)
}
fn projected(app: &mut App) -> crate::core::messages::WeaponsBlackboard {
    let raw = board_packet(app, &[TACTICAL_STATION_ID], Target::Token("science".into()));
    let output = capture(app).project(&raw);
    let message = phone(&output, "science").unwrap();
    let _ = encoded(message);
    let SystemBlackboard::Weapons(board) = only_board(message) else {
        panic!("Weapons")
    };
    board.clone()
}
#[test]
fn assignment_members_require_producing_console_ownership_or_exact_supported_feed() {
    let (mut app, ship, alias) = setup();
    let board = projected(&mut app);
    assert_eq!(board.assignments.len(), 1);
    assert_eq!(
        board.assignments[&SystemId("phaser-fore".into())].individual,
        Some(Ref::Hold)
    );
    assert_eq!(
        board.assignments[&SystemId("phaser-fore".into())]
            .group
            .as_ref()
            .unwrap()
            .target,
        Ref::Target { contact: alias }
    );
    // Explicit fine-bank feed grants information only; it is not ownership.
    {
        let mut config = app
            .world_mut()
            .get_mut::<crate::ship::components::ShipConfigComponent>(ship)
            .unwrap();
        let tactical = config
            .0
            .system(&SystemId("tactical-radar".into()))
            .unwrap()
            .console
            .clone()
            .unwrap();
        config
            .0
            .consoles
            .iter_mut()
            .find(|console| console.id == tactical)
            .unwrap()
            .feeds
            .push(phoenix_model::console::ConsoleFeed {
                source: SystemId("phaser-aft".into()),
                kind: phoenix_model::console::ConsoleObservationKind::PhaserBank,
            });
    }
    app.world_mut()
        .get_mut::<crate::server_app::ShipSystemBlackboards>(ship)
        .unwrap()
        .0
        .insert(
            SystemId("phaser-aft".into()),
            SystemBlackboard::PhaserBank(Default::default()),
        );
    assert_eq!(projected(&mut app).assignments.len(), 2);
}
#[test]
fn withheld_stale_and_forged_references_remain_unavailable_and_never_become_hold() {
    let (mut app, ship, alias) = setup();
    observe(&mut app, "tactical-radar", 1, 0.0);
    let board = projected(&mut app);
    let row = &board.assignments[&SystemId("phaser-fore".into())];
    assert_eq!(row.individual, Some(Ref::Hold));
    assert_eq!(row.group.as_ref().unwrap().target, Ref::Unavailable);
    let fresh_alias = observe(&mut app, "tactical-radar", 2, 1.0);
    let mut boards = app
        .world_mut()
        .get_mut::<crate::server_app::ShipSystemBlackboards>(ship)
        .unwrap();
    let SystemBlackboard::Weapons(raw) = boards.0.get_mut(&tactical_station_key()).unwrap() else {
        unreachable!()
    };
    assert_eq!(
        raw.assignments[&SystemId("phaser-fore".into())]
            .group
            .as_ref()
            .unwrap()
            .target,
        Ref::Target { contact: alias }
    );
    raw.assignments
        .get_mut(&SystemId("phaser-fore".into()))
        .unwrap()
        .individual = Some(Ref::Target {
        contact: "hidden".into(),
    });
    raw.assignments
        .get_mut(&SystemId("phaser-fore".into()))
        .unwrap()
        .group
        .as_mut()
        .unwrap()
        .target = Ref::Target {
        contact: fresh_alias,
    };

    drop(boards);
    assert_eq!(
        projected(&mut app).assignments[&SystemId("phaser-fore".into())].individual,
        Some(Ref::Unavailable)
    );
    crate::gm::operations::contact::set(
        &mut app
            .world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .contact_overrides,
        "own",
        "hidden",
        crate::gm::operations::contact::ContactMode::Conceal,
    );
    assert_eq!(
        projected(&mut app).assignments[&SystemId("phaser-fore".into())]
            .group
            .as_ref()
            .unwrap()
            .target,
        Ref::Unavailable
    );
}
