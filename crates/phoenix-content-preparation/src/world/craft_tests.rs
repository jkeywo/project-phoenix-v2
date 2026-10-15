use super::*;
use crate::world::{config::parse_world, load::MemoryTemplateLoader};
use phoenix_sim_gameplay::ship::composition::SystemComponentConfig;
const WORLD: &str = r#"
[[ship_slot]]
id="crew"
label="Crew"
default_ship="parent.toml"
[[ship_slot.ships]]
template_path="parent.toml"
[[entity]]
name="parent"
template_path="parent.toml"
spawn_on="game_start"
[[entity]]
name="probe"
template_path="probe.toml"
spawn_on="game_start"
carried_by={parent="parent",bay="actual-bay"}
"#;
fn templates() -> MemoryTemplateLoader {
    let loader = crate::entities::loader::FixtureTemplates;
    let mut parent = loader
        .load_template("assets/entities/local_jump_cruiser.toml")
        .unwrap();
    let id = phoenix_model::identity::SystemId("actual-bay".into());
    let ship = parent.ship_config.as_mut().unwrap();
    let mut system = ship
        .system(&phoenix_model::identity::SystemId("local-jump".into()))
        .unwrap()
        .clone();
    system.id = id.clone();
    system.kind = phoenix_sim_gameplay::ship::system_registry::CRAFT_BAY_KIND.into();
    system.channels.clear();
    system.components = vec![
        SystemComponentConfig::PowerInput { id: "input".into() },
        SystemComponentConfig::CraftBay {
            id: "bay".into(),
            power_input: "input".into(),
            config: phoenix_model::craft::BayConfig {
                compartment: "bay-room".into(),
                capacity_space: 1,
                launch_work_secs: 2.0,
                recovery_work_secs: 2.0,
                recovery_range: 8.0,
                maximum_parent_speed: 0.1,
                release_offset: [0.0, 0.0, -3.0],
            },
        },
    ];
    ship.systems.push(system);
    let interior = parent.interior.as_mut().unwrap();
    let mut room = interior
        .compartments
        .iter()
        .find(|row| row.id == "engineering")
        .unwrap()
        .clone();
    room.id = "bay-room".into();
    room.x += 30.0;
    interior.compartments.push(room);
    interior.placements.insert(id.clone(), "bay-room".into());
    let energy = parent.energy.as_mut().unwrap();
    let mut consumer = energy
        .consumers
        .iter()
        .find(|row| row.system.0 == "local-jump")
        .unwrap()
        .clone();
    consumer.system = id;
    energy.consumers.push(consumer);
    let mut probe = loader
        .load_template("assets/entities/battery_probe.toml")
        .unwrap();
    probe.probe = Some(phoenix_model::craft::ProbeProfile {
        space: 1,
        passenger_berths: Vec::new(),
    });
    MemoryTemplateLoader::new([("parent.toml", parent), ("probe.toml", probe)])
}
#[test]
fn ordinary_ship_tags_do_not_turn_stored_craft_into_an_offered_crew_slot() {
    let loader = templates();
    let config = parse_world(WORLD).unwrap();
    let check = |candidate: &crate::world::config::WorldConfig| {
        validate_initial(
            &WorldSource::new("probe-world.toml", WORLD, candidate),
            &loader,
            true,
        )
    };
    check(&config).unwrap();
    assert_eq!(
        loader.load_template("probe.toml").unwrap().tags,
        vec!["ship"]
    );
    for fault in 0..5 {
        let mut invalid = config.clone();
        match fault {
            0 => invalid.ship_slots.clear(),
            1 => invalid.entities.swap(0, 1),
            2 => invalid.entities[0].when = Some("flag(later)".into()),
            3 => {
                let mut second = invalid.entities[1].clone();
                second.name = Some("other-probe".into());
                invalid.entities.push(second);
            }
            _ => {
                invalid.ship_slots[0].unclaimed = crate::world::config::UnclaimedSlotPolicy::Absent
            }
        }
        assert!(check(&invalid).is_err(), "case {fault}");
    }
    assert!(validate_initial(
        &WorldSource::new("layer.toml", WORLD, &config),
        &loader,
        false
    )
    .is_err());
}
#[test]
fn every_offered_parent_hull_must_own_the_real_bay_not_borrow_selected_equipment() {
    let mut config = parse_world(WORLD).unwrap();
    let mut offered = config.ship_slots[0].ships[0].clone();
    offered.template_path = "probe.toml".into();
    config.ship_slots[0].ships.push(offered);
    assert!(validate_initial(
        &WorldSource::new("probe-world.toml", WORLD, &config),
        &templates(),
        true
    )
    .unwrap_err()
    .contains("every offered parent hull"));
}
