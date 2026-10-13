use super::*;

#[test]
fn cohosted_blaster_banks_do_not_share_an_unbound_lock() {
    let config = crate::ship_plugin::ShipConfigComponent(
        crate::ship::config::ShipConfig::from_toml(
            r#"
[[station]]
id = "tactical"
name = "Tactical"
description = "Weapons"
rank = "Ltn."
[[console]]
id = "left-screen"
home_station = "tactical"
screen = "fixture.html"
[[console.feed]]
source = "viewscreen"
kind = "Viewscreen"
[[console]]
id = "right-screen"
home_station = "tactical"
screen = "fixture.html"
[[system]]
id = "blaster-left"
kind = "blaster_bank"
console = "left-screen"
[[system]]
id = "blaster-right"
kind = "blaster_bank"
console = "right-screen"
"#,
            &["blaster_bank"],
        )
        .unwrap(),
    );
    let left = crate::ship::system_registry::blaster_bank_system_id("left").unwrap();
    let right = crate::ship::system_registry::blaster_bank_system_id("right").unwrap();
    let mut boards = crate::server_app::ShipSystemBlackboards::default();
    boards.0.insert(
        crate::ship::system_registry::viewscreen_system_id(),
        SystemBlackboard::Viewscreen(crate::core::messages::ViewscreenBlackboard {
            combat_lock: Some("contact".into()),
            ..Default::default()
        }),
    );
    assert_eq!(
        blaster_ai_combat_lock(Some(&config), &left, Some(&boards)).as_deref(),
        Some("contact")
    );
    assert_eq!(
        blaster_ai_combat_lock(Some(&config), &right, Some(&boards)),
        None
    );
    assert_eq!(blaster_ai_combat_lock(None, &left, Some(&boards)), None);
    // The authoritative machinery applier still uses the existing lock.
    assert_eq!(
        blaster_combat_lock(Some(&boards)).as_deref(),
        Some("contact")
    );
}
