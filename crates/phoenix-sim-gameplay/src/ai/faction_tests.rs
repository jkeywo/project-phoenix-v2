use super::*;

fn alliance_uuid() -> Uuid {
    Uuid::parse_str("aaaaaaaa-0000-0000-0000-000000000001").unwrap()
}

fn pirate_uuid() -> Uuid {
    Uuid::parse_str("bbbbbbbb-0000-0000-0000-000000000002").unwrap()
}

fn make_registry_allied_hostile_to_pirate() -> FactionRegistry {
    let mut reg = FactionRegistry::new();
    reg.insert(FactionConfig {
        display_name: None,
        uuid: alliance_uuid(),
        name: "Alliance".to_string(),
        enemies: vec![pirate_uuid()],
        compliance: None,
    });
    reg.insert(FactionConfig {
        display_name: None,
        uuid: pirate_uuid(),
        name: "Pirate".to_string(),
        enemies: vec![],
        compliance: None,
    });
    reg
}

// Tracer bullet: both factionless → not enemies
#[test]
fn both_factionless_are_not_enemies() {
    let reg = FactionRegistry::new();
    assert!(!is_enemy(None, None, &reg));
}

// One factionless → not enemies
#[test]
fn one_factionless_is_not_enemy() {
    let reg = make_registry_allied_hostile_to_pirate();
    assert!(!is_enemy(Some(alliance_uuid()), None, &reg));
    assert!(!is_enemy(None, Some(pirate_uuid()), &reg));
}

// A lists B → A considers B an enemy (asymmetric)
#[test]
fn a_lists_b_as_enemy_is_true() {
    let reg = make_registry_allied_hostile_to_pirate();
    assert!(is_enemy(Some(alliance_uuid()), Some(pirate_uuid()), &reg));
}

// B does NOT list A → not an enemy (asymmetry)
#[test]
fn b_does_not_list_a_is_not_enemy() {
    let reg = make_registry_allied_hostile_to_pirate();
    // Pirate has no enemies listed
    assert!(!is_enemy(Some(pirate_uuid()), Some(alliance_uuid()), &reg));
}

// Neither lists the other → not enemies
#[test]
fn neither_lists_other_is_not_enemy() {
    let mut reg = FactionRegistry::new();
    let alpha = Uuid::parse_str("cccccccc-0000-0000-0000-000000000003").unwrap();
    let beta = Uuid::parse_str("dddddddd-0000-0000-0000-000000000004").unwrap();
    reg.insert(FactionConfig {
        display_name: None,
        uuid: alpha,
        name: "Alpha".to_string(),
        enemies: vec![],
        compliance: None,
    });
    reg.insert(FactionConfig {
        display_name: None,
        uuid: beta,
        name: "Beta".to_string(),
        enemies: vec![],
        compliance: None,
    });
    assert!(!is_enemy(Some(alpha), Some(beta), &reg));
    assert!(!is_enemy(Some(beta), Some(alpha), &reg));
}

// TOML round-trip for FactionConfig
#[test]
fn faction_config_toml_round_trip() {
    let toml_str = r#"
uuid = "aaaaaaaa-0000-0000-0000-000000000001"
name = "Alliance"
enemies = ["bbbbbbbb-0000-0000-0000-000000000002"]
"#;
    let config = parse_faction_config(toml_str).expect("parse must succeed");
    assert_eq!(config.uuid, alliance_uuid());
    assert_eq!(config.name, "Alliance");
    assert_eq!(config.enemies, vec![pirate_uuid()]);
}

// TOML round-trip: enemies defaults to empty when omitted
#[test]
fn faction_config_no_enemies_defaults_to_empty() {
    let toml_str = r#"
uuid = "bbbbbbbb-0000-0000-0000-000000000002"
name = "Pirate"
"#;
    let config = parse_faction_config(toml_str).expect("parse must succeed");
    assert!(config.enemies.is_empty());
}

// FactionRegistry insert and lookup
#[test]
fn registry_insert_and_get() {
    let reg = make_registry_allied_hostile_to_pirate();
    assert_eq!(reg.len(), 2);
    let alliance = reg.get(&alliance_uuid()).expect("alliance must be present");
    assert_eq!(alliance.name, "Alliance");
}

// Unknown faction UUID → not an enemy (registry miss)
#[test]
fn unknown_faction_uuid_is_not_enemy() {
    let reg = FactionRegistry::new();
    let unknown = Uuid::new_v4();
    let other = Uuid::new_v4();
    assert!(!is_enemy(Some(unknown), Some(other), &reg));
}

// Load actual TOML asset files
#[test]
fn alliance_toml_parses_correctly() {
    let toml_str = include_str!("../../../../assets/factions/alliance.toml");
    let config = parse_faction_config(toml_str).expect("alliance.toml must parse");
    assert_eq!(config.name, "Alliance");
    assert!(!config.uuid.is_nil());
    // Must list pirates as enemies
    assert!(!config.enemies.is_empty(), "Alliance must have enemies");
}

#[test]
fn pirate_toml_parses_correctly() {
    let toml_str = include_str!("../../../../assets/factions/pirate.toml");
    let config = parse_faction_config(toml_str).expect("pirate.toml must parse");
    assert_eq!(config.name, "Pirate");
    assert!(!config.uuid.is_nil());
}

#[test]
fn alliance_and_pirate_are_mutually_hostile() {
    let alliance_toml = include_str!("../../../../assets/factions/alliance.toml");
    let pirate_toml = include_str!("../../../../assets/factions/pirate.toml");
    let alliance = parse_faction_config(alliance_toml).unwrap();
    let pirate = parse_faction_config(pirate_toml).unwrap();

    let mut reg = FactionRegistry::new();
    reg.insert(alliance.clone());
    reg.insert(pirate.clone());

    assert!(
        is_enemy(Some(alliance.uuid), Some(pirate.uuid), &reg),
        "Alliance must consider Pirates as enemies"
    );
    assert!(
        is_enemy(Some(pirate.uuid), Some(alliance.uuid), &reg),
        "Pirates must consider Alliance as enemies"
    );
}

#[test]
fn alliance_and_harrow_are_neutral_by_default() {
    // Harrow defaults to neutral so it can be reused as ambient
    // patrols in non-combat worlds (e.g. Starbase Alpha, Before the
    // Fire). Hostile scenarios (combat test) flip the relationship at
    // runtime via the `add_faction_enemy` trigger action.
    let alliance_toml = include_str!("../../../../assets/factions/alliance.toml");
    let harrow_toml = include_str!("../../../../assets/factions/harrow.toml");
    let alliance = parse_faction_config(alliance_toml).unwrap();
    let harrow = parse_faction_config(harrow_toml).unwrap();

    let mut reg = FactionRegistry::new();
    reg.insert(alliance.clone());
    reg.insert(harrow.clone());

    assert!(
        !is_enemy(Some(alliance.uuid), Some(harrow.uuid), &reg),
        "Alliance must default to neutral toward Harrow"
    );
    assert!(
        !is_enemy(Some(harrow.uuid), Some(alliance.uuid), &reg),
        "Harrow must default to neutral toward Alliance"
    );
}

// ── Mutators ──────────────────────────────────────────────────────────────

#[test]
fn uuid_by_name_finds_existing_faction() {
    let reg = make_registry_allied_hostile_to_pirate();
    assert_eq!(reg.uuid_by_name("Alliance"), Some(alliance_uuid()));
    assert_eq!(reg.uuid_by_name("Pirate"), Some(pirate_uuid()));
}

#[test]
fn uuid_by_name_returns_none_for_unknown() {
    let reg = make_registry_allied_hostile_to_pirate();
    assert!(reg.uuid_by_name("Klingon").is_none());
}

#[test]
fn uuid_by_name_is_case_sensitive() {
    let reg = make_registry_allied_hostile_to_pirate();
    assert!(reg.uuid_by_name("alliance").is_none());
    assert!(reg.uuid_by_name("ALLIANCE").is_none());
}

#[test]
fn add_enemy_creates_new_relationship() {
    let mut reg = FactionRegistry::new();
    let alpha = Uuid::parse_str("cccccccc-0000-0000-0000-000000000003").unwrap();
    let beta = Uuid::parse_str("dddddddd-0000-0000-0000-000000000004").unwrap();
    reg.insert(FactionConfig {
        display_name: None,
        uuid: alpha,
        name: "Alpha".to_string(),
        enemies: vec![],
        compliance: None,
    });
    reg.insert(FactionConfig {
        display_name: None,
        uuid: beta,
        name: "Beta".to_string(),
        enemies: vec![],
        compliance: None,
    });
    assert!(!is_enemy(Some(alpha), Some(beta), &reg));
    assert!(reg.add_enemy(alpha, beta), "first add returns true");
    assert!(is_enemy(Some(alpha), Some(beta), &reg));
    // Asymmetric — Beta still does not consider Alpha an enemy.
    assert!(!is_enemy(Some(beta), Some(alpha), &reg));
}

#[test]
fn add_enemy_is_idempotent() {
    let mut reg = make_registry_allied_hostile_to_pirate();
    // Alliance already lists Pirate as an enemy.
    assert!(!reg.add_enemy(alliance_uuid(), pirate_uuid()));
    // And the relationship hasn't been duplicated.
    let alliance = reg.get(&alliance_uuid()).unwrap();
    assert_eq!(
        alliance
            .enemies
            .iter()
            .filter(|u| **u == pirate_uuid())
            .count(),
        1
    );
}

#[test]
fn add_enemy_returns_false_for_unknown_faction() {
    let mut reg = make_registry_allied_hostile_to_pirate();
    let unknown = Uuid::new_v4();
    assert!(!reg.add_enemy(unknown, alliance_uuid()));
}

#[test]
fn remove_enemy_clears_relationship() {
    let mut reg = make_registry_allied_hostile_to_pirate();
    assert!(is_enemy(Some(alliance_uuid()), Some(pirate_uuid()), &reg));
    assert!(reg.remove_enemy(alliance_uuid(), pirate_uuid()));
    assert!(!is_enemy(Some(alliance_uuid()), Some(pirate_uuid()), &reg));
}

#[test]
fn remove_enemy_is_idempotent() {
    let mut reg = make_registry_allied_hostile_to_pirate();
    // Pirate has no enemies listed → removing Alliance is a no-op.
    assert!(!reg.remove_enemy(pirate_uuid(), alliance_uuid()));
}

#[test]
fn remove_enemy_returns_false_for_unknown_faction() {
    let mut reg = make_registry_allied_hostile_to_pirate();
    let unknown = Uuid::new_v4();
    assert!(!reg.remove_enemy(unknown, alliance_uuid()));
}
