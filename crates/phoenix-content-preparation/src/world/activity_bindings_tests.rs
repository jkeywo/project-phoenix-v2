use super::*;
use crate::world::config::parse_world;

const WORLD: &str = r#"
[[entity]]
name="crew"
template_path="assets/entities/alliance_cruiser.toml"
spawn_on="game_start"
[[situation_definition]]
id="work"
initial="useful"
[[situation_definition.state]]
id="useful"
when={kind="achieved"}
[[situation]]
id="restore"
definition="work"
request={kind="repair",actor="crew",owner="repair",team=0,request={type="Work",data={kind="compartment",compartment="engineering"}},target={kind="compartment",compartment="engineering"},goal="structure"}
[[situation]]
id="supply"
definition="work"
request={kind="delivery",actor="crew",owner="logistics",source="hold-ammunition",destination="magazine",commodity="torpedoes",amount=2}
"#;

#[test]
fn accepted_world_validation_refuses_actual_missing_goal_team_and_route_before_activation() {
    let loader = crate::entities::loader::FixtureTemplates;
    let config = parse_world(WORLD).unwrap();
    let inspect = |config: &crate::world::config::WorldConfig| {
        findings(
            &WorldSource::new("assets/worlds/activities.toml", WORLD, config),
            &loader,
        )
    };
    assert!(inspect(&config).is_empty());
    for change in 0..4 {
        let mut candidate = config.clone();
        match change {
            0 => {
                if let ActivityRequest::Repair { team, .. } = &mut candidate.situations[0].request {
                    *team = 2;
                }
            }
            1 => {
                if let ActivityRequest::Repair { goal, .. } = &mut candidate.situations[0].request {
                    *goal = phoenix_model::activity::RepairGoal::Full;
                }
            }
            2 => {
                if let ActivityRequest::Delivery { destination, .. } =
                    &mut candidate.situations[1].request
                {
                    *destination = "unowned".into();
                }
            }
            _ => {
                candidate.entities[0].overrides =
                    Some(toml::from_str("[repair]\nrepair_team_count=0\n").unwrap())
            }
        }
        let refused = inspect(&candidate);
        assert!(!refused.is_empty(), "case{change}");
        assert!(refused.iter().all(
            |finding| finding.category == "unsupported-activity-binding" && finding.is_error()
        ));
        let all = crate::world::validate::validate_composition_with_fragments(
            &WorldSource::new("assets/worlds/activities.toml", WORLD, &candidate),
            &[],
            &loader,
            &crate::entities::include_resolve::FixtureFragments,
        );
        assert!(all
            .iter()
            .any(|finding| finding.category == "unsupported-activity-binding"));
    }
}

#[test]
fn launch_slot_and_named_player_bindings_validate_every_offered_actual_hull_with_world_overrides() {
    let loader = crate::entities::loader::FixtureTemplates;
    let mut config = parse_world(WORLD).unwrap();
    let slot_world = parse_world(
        r#"
[[ship_slot]]
id="lead"
default_ship="assets/entities/alliance_cruiser.toml"
[[ship_slot.ships]]
template_path="assets/entities/alliance_cruiser.toml"
"#,
    )
    .unwrap();
    config.ship_slots = slot_world.ship_slots;
    for binding in &mut config.situations {
        match &mut binding.request {
            ActivityRequest::Repair { actor, .. }
            | ActivityRequest::Delivery { actor, .. }
            | ActivityRequest::Communication { actor, .. } => *actor = "slot:lead".into(),
        }
    }
    if let ActivityRequest::Repair { team, .. } = &mut config.situations[0].request {
        *team = 1;
    }
    let inspect = |config: &crate::world::config::WorldConfig| {
        findings(&WorldSource::new("world.toml", WORLD, config), &loader)
    };
    assert!(inspect(&config).is_empty());
    config.entities[0].overrides = Some(toml::from_str("[repair]\nrepair_team_count=0\n").unwrap());
    assert!(
        !inspect(&config).is_empty(),
        "selected launch hull uses the actual row override"
    );
    config.entities[0].overrides = None;
    // Both are actual accepted ship hulls; the Courier has only one team.
    let mut offer = config.ship_slots[0].ships[0].clone();
    offer.template_path = "assets/entities/alliance_courier.toml".into();
    config.ship_slots[0].ships.push(offer);
    assert!(
        !inspect(&config).is_empty(),
        "an incompatible actual hull choice cannot silently accept the activity"
    );
    if let ActivityRequest::Repair { actor, .. } = &mut config.situations[0].request {
        *actor = "crew".into();
    }
    assert!(
        !inspect(&config).is_empty(),
        "a named GameStart actor has the same launch substitution boundary"
    );
}
