use super::*;
use crate::world::config::parse_world;
const VALID: &str = r#"
[[entity]]
template_path="parent.toml"
name="parent"
spawn_on="immediate"
[[entity]]
template_path="probe.toml"
name="probe"
spawn_on="game_start"
carried_by={parent="parent",bay="actual-bay"}
"#;
#[test]
fn initial_custody_is_closed_named_and_parent_available_before_probe_operation() {
    let config = parse_world(VALID).unwrap();
    validate_initial_rows(&config).unwrap();
    let encoded = toml::to_string(&config.entities[1]).unwrap();
    let row: crate::world::config::WorldEntity = toml::from_str(&encoded).unwrap();
    assert_eq!(row.carried_by, config.entities[1].carried_by);
    for fault in 0..6 {
        let mut invalid = config.clone();
        match fault {
            0 => invalid.entities[1].carried_by.as_mut().unwrap().parent = "missing".into(),
            1 => invalid.entities[1].name = None,
            2 => invalid.entities[1].when = Some("flag(later)".into()),
            3 => invalid.entities[0].when = Some("flag(later)".into()),
            4 => {
                invalid.entities[0].spawn_on = WorldEntitySpawnOn::GameStart;
                invalid.entities[1].spawn_on = WorldEntitySpawnOn::Immediate;
            }
            _ => {
                invalid.entities[0].spawn_on = WorldEntitySpawnOn::GameStart;
                invalid.entities.swap(0, 1);
            }
        }
        assert!(validate_initial_rows(&invalid).is_err(), "case {fault}");
    }
    assert!(parse_world(&VALID.replace(r#"parent="parent""#, r#"parent="probe""#)).is_err());
    assert!(
        parse_world(&VALID.replace(r#"bay="actual-bay""#, r#"bay="actual-bay",progress=0.5"#))
            .is_err()
    );
}
