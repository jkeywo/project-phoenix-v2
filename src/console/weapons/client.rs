//! Client-side Weapons Panel plugin â€” migrated to `src/gui/` library widgets.
//!
//! Owns all Tactical console UI: fire phasers button (`GuiButton`), phaser mode
//! toggle (`GuiButton`), torpedo tube selector (`RadioGroup`), fire torpedo
//! button (`GuiButton`), torpedo count / tube status readouts, and a
//! `GenericRadar` (WorldFixed, Ships + Torpedoes filter).
//!
//! No per-button marker-component query systems remain. All callbacks are wired
//! via observers at spawn time.

use bevy::prelude::*;

use crate::client::console_shell::ConsoleShell;
use crate::client_app::{
    WeaponsPanel, OutboundClientMessage, RepairIconLabel,
    HideableElement, ComplexityPopupRoot, ComplexityPresetButton, ComplexityPopupConfirm,
    ComplexityDropdownRoot,
};
use crate::client_lobby::{ActiveConsole, LobbyState, LobbyView, LocalPlayerToken};
use crate::client_sim::{
    fire_phaser_message, set_target_message,
    toggle_phaser_mode_message, fire_torpedo_message,
    is_fire_button_enabled, is_tube_loaded, tube_reload_secs,
    phaser_mode_label, ClientSimState,
};
use crate::gui::{
    bridge_sim_to_radar, spawn_gui_button, ButtonPressed,
    ButtonSize, ConsoleRadar, GenericRadar, OrientationMode, RadarArc,
    RadarArcKind, RadarArcs, RadarBlipClicked, RadarBlipMap, RadarCenterPose, RadarClipMode,
    RadarEntityUuid, RadarFilter, RadarTargetHighlight, RadarTargetRing, RadioButtonConfig, RadioGroup,
    RadioSelected, StateVisuals, Disabled,
};
use crate::messages::{Console, GamePhase, PhaserBankClientConfig, TorpedoTube, TorpedoTubeClientConfig};
use crate::phone_border::framing::{DeviceOrientation, PhoneAssets};
use crate::ship_view::ShipView;

// â”€â”€ Pure visibility helper â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// Decide whether the weapons panel should be visible.
///
/// Rules:
/// 1. Game phase must be `InProgress`.
/// 2. The local player must hold `Console::Tactical`.
/// 3. If holding **one console only**, show automatically.
/// 4. If holding **multiple consoles**, show only when `ActiveConsole`
///    is explicitly set to `Tactical`.
pub fn weapons_panel_visible(
    lobby: &LobbyState,
    token: &str,
    active: &ActiveConsole,
) -> bool {
    if lobby.phase != GamePhase::InProgress {
        return false;
    }
    let view = LobbyView::new(lobby, token);
    let consoles = view.my_consoles();
    if !consoles.contains(&Console::Tactical) {
        return false;
    }
    let count = consoles.len();
    match &active.0 {
        Some(c) => *c == Console::Tactical,
        None => count == 1,
    }
}

// â”€â”€ Marker components â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// Marks the text label inside a Fire Phasers button (shows cooldown status).
/// Carries the bank id so multiple banks can share the refresh system.
#[derive(Component)]
struct FirePhaserLabel(String);

/// Marks the text label inside the Phaser Mode toggle button.
#[derive(Component)]
struct PhaserModeLabel;

/// Marks the text label inside the Fire Torpedo button.
#[derive(Component)]
struct FireTorpedoLabel;

/// Marks the torpedo count text label.
#[derive(Component)]
struct TorpedoCountLabel;

/// Marks the label that shows tube reload status. Stores the tube id it displays.
#[derive(Component)]
struct TubeStatusLabel(TorpedoTube);

/// Marks a Fire Phasers `GuiButton` entity. Carries the bank id (matches
/// `PhaserBankClientConfig.id`) so the press handler knows which bank to fire.
#[derive(Component)]
struct FirePhaserButton(String);

/// Marks the Fire Torpedo `GuiButton` entity.
#[derive(Component)]
struct FireTorpedoButton;

/// Marks the `RadioGroup` entity used for torpedo tube selection. Carries the
/// ordered list of tube ids so the `RadioSelected` observer can map a member
/// child entity back to a tube id.
#[derive(Component)]
struct TubeRadioGroup(Vec<String>);

// â”€â”€ Resources â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// Tracks which torpedo tube is currently selected on the Weapons console.
///
/// `None` means no tube is selected. Updated when the `RadioGroup` fires
/// `RadioSelected`.
#[derive(Resource, Default, Clone, PartialEq, Eq, Debug)]
pub struct SelectedTube(pub Option<TorpedoTube>);

// â”€â”€ State visuals helpers â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// Danger (red) button visuals â€” used for Fire Phasers.
fn fire_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.40, 0.10, 0.10), // idle
        Color::srgb(0.55, 0.12, 0.12), // hover
        Color::srgb(0.60, 0.10, 0.10), // active
        Color::srgb(0.70, 0.15, 0.15), // press
        Color::srgb(0.15, 0.05, 0.05), // disabled
    )
}

/// Neutral (blue-grey) button visuals â€” used for Phaser Mode toggle.
fn mode_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.15, 0.15, 0.35), // idle
        Color::srgb(0.20, 0.20, 0.45), // hover
        Color::srgb(0.25, 0.25, 0.55), // active
        Color::srgb(0.30, 0.30, 0.60), // press
        Color::srgb(0.08, 0.08, 0.20), // disabled
    )
}

/// Safe (green) button visuals â€” used for Fire Torpedo.
fn torpedo_fire_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.10, 0.30, 0.10), // idle
        Color::srgb(0.12, 0.40, 0.12), // hover
        Color::srgb(0.10, 0.50, 0.10), // active
        Color::srgb(0.15, 0.55, 0.15), // press
        Color::srgb(0.05, 0.15, 0.05), // disabled
    )
}

/// Radio tube button visuals.
fn tube_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.10, 0.20, 0.30), // idle
        Color::srgb(0.12, 0.28, 0.42), // hover
        Color::srgb(0.10, 0.50, 0.70), // active (selected)
        Color::srgb(0.15, 0.35, 0.55), // press
        Color::srgb(0.05, 0.10, 0.15), // disabled
    )
}

// â”€â”€ Plugin â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// Marker resource set once the weapons UI has been spawned.
#[derive(Resource)]
pub struct WeaponsPanelSpawned;

/// Cached layout-key for the currently-spawned weapons panel. When the
/// `LobbyState::ship_config` reports a different list of bank/tube ids, the
/// panel is torn down and respawned so the per-bank/per-tube UI matches.
#[derive(Resource, Clone, PartialEq, Eq, Debug)]
pub struct WeaponsPanelLayoutKey {
    pub banks: Vec<String>,
    pub tubes: Vec<String>,
}

impl WeaponsPanelLayoutKey {
    /// Build a layout key from a `ShipClientConfig`.
    pub fn from_ship_config(cfg: &crate::messages::ShipClientConfig) -> Self {
        Self {
            banks: cfg.phaser_banks.iter().map(|b| b.id.clone()).collect(),
            tubes: cfg.torpedo_tubes.iter().map(|t| t.id.clone()).collect(),
        }
    }
}

// â”€â”€ Plugin â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// Plugin that owns all Tactical console UI and systems.
pub struct WeaponsPanelPlugin;

impl Plugin for WeaponsPanelPlugin {
    fn build(&self, app: &mut App) {
        app
            .init_resource::<SelectedTube>()
            .add_systems(Update, (
                spawn_weapons_ui.run_if(not(resource_exists::<WeaponsPanelSpawned>)),
                toggle_weapons_panel_visibility,
                add_tube_button_labels,
                refresh_weapons_panel,
                sync_fire_phaser_disabled,
                refresh_torpedo_ui,
                bridge_client_sim_to_weapons_radar,
                sync_weapons_radar_target_highlight,
                respawn_weapons_on_orientation_change,
                respawn_weapons_on_layout_change,
            ));
    }
}

// â”€â”€ Spawn (ConsoleShell) â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

fn spawn_weapons_ui(
    mut commands: Commands,
    assets: Option<Res<PhoneAssets>>,
    lobby: Res<LobbyState>,
    old_panel: Query<Entity, With<WeaponsPanel>>,
    old_help: Query<(Entity, &crate::client::elements::HelpOverlay)>,
    orientation: Option<Res<DeviceOrientation>>,
) {
    let Some(assets) = assets else { return };

    // Gate: wait until ship_config has been populated by Welcome.
    if lobby.ship_config.phaser_banks.is_empty() {
        return;
    }

    let is_landscape = crate::phone_border::framing::is_landscape(orientation.as_deref());

    for entity in old_panel.iter() {
        commands.entity(entity).despawn();
    }
    for (entity, overlay) in old_help.iter() {
        if overlay.0 == crate::client::elements::HelpPanel::Tactical {
            commands.entity(entity).despawn();
        }
    }

    commands.insert_resource(WeaponsPanelSpawned);
    commands.insert_resource(WeaponsPanelLayoutKey::from_ship_config(&lobby.ship_config));

    let banks = lobby.ship_config.phaser_banks.clone();
    let tubes = lobby.ship_config.torpedo_tubes.clone();
    let beam_color = lobby.ship_config.phaser_beam_color;
    let torp_color = lobby.ship_config.torpedo_arc_color;

    let shell = ConsoleShell::spawn(
        &mut commands,
        assets.helm_panel_bg.clone(),
        is_landscape,
        crate::client::elements::HelpPanel::Tactical,
        {
            let banks_r = banks.clone();
            let tubes_r = tubes.clone();
            move |commands: &mut Commands, primary: Entity| {
                fill_tactical_radar(commands, primary, &banks_r, &tubes_r, beam_color, torp_color);
            }
        },
        move |commands: &mut Commands, secondary: Entity| {
            fill_tactical_controls(commands, secondary, &banks, &tubes);
        },
        &assets,
    );

    commands.entity(shell.root).insert((WeaponsPanel, Visibility::Hidden));
}

// â”€â”€ Fill helpers â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// Primary slot: tactical radar (GenericRadar, ShipRelative, Ships + Torpedoes).
fn fill_tactical_radar(
    commands: &mut Commands,
    container: Entity,
    banks: &[PhaserBankClientConfig],
    tubes: &[TorpedoTubeClientConfig],
    beam_color: [f32; 4],
    torp_color: [f32; 4],
) {
    let col = commands
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            row_gap: Val::Px(8.0),
            ..default()
        })
        .id();
    commands.entity(container).add_child(col);

    let title = commands
        .spawn((
            Text::new("Weapons Console"),
            TextFont { font_size: 24.0, ..default() },
            TextColor(Color::srgb(1.0, 0.5, 0.2)),
        ))
        .id();
    commands.entity(col).add_child(title);

    // Spawn with empty filter â€” sync_tactical_radar_filter will populate it
    // from lobby.ship_config.tactical_radar_shows once the Welcome arrives.
    let radar_filter = RadarFilter(std::collections::HashSet::new());
    let radar = GenericRadar::spawn(
        commands,
        crate::client_sim::WEAPONS_RADAR_RANGE,
        OrientationMode::ShipRelative,
        radar_filter,
        None,
        None,
        RadarClipMode::Circle,
        1.0,
        1.0,
    );
    let beam = Color::srgba(beam_color[0], beam_color[1], beam_color[2], 0.15);
    let torp = Color::srgba(torp_color[0], torp_color[1], torp_color[2], 0.15);
    let mut arcs: Vec<RadarArc> = Vec::new();
    for bank in banks {
        arcs.push(RadarArc {
            id: format!("phaser:{}", bank.id),
            kind: RadarArcKind::Phaser,
            facing_deg: bank.facing_deg,
            fire_arc_deg: bank.fire_arc_deg,
            color: beam,
        });
    }
    for tube in tubes {
        arcs.push(RadarArc {
            id: format!("torpedo:{}", tube.id),
            kind: RadarArcKind::Torpedo,
            facing_deg: tube.facing_deg,
            fire_arc_deg: tube.fire_arc_deg,
            color: torp,
        });
    }
    commands.entity(radar).insert((
        Node {
            width:  Val::Px(240.0),
            height: Val::Px(240.0),
            border: UiRect::all(Val::Px(1.0)),
            aspect_ratio: Some(1.0),
            position_type: PositionType::Relative,
            ..default()
        },
        ConsoleRadar::Tactical,
        RadarBlipMap::default(),
        RadarArcs(arcs),
        RadarTargetHighlight(None),
        RadarTargetRing::default(),
    ));
    commands.entity(radar).observe(on_tactical_radar_blip_clicked);
    commands.entity(col).add_child(radar);
}

/// Observer: when a blip on the tactical radar is clicked, look up the
/// source entity's `RadarEntityUuid` and dispatch `SetTarget { uuid }`.
///
/// Registered per-radar via `commands.entity(radar).observe(...)`. The
/// `RadarBlipClicked` event is triggered against the radar entity by
/// `detect_radar_blip_press` (gui/radar.rs), so this observer only fires
/// for taps on **this** radar — sensors/long-range radars get their own
/// observer for their own designation flows.
///
/// The picking math, hit-test, and HiDPI conversions all live inside the
/// shared `gui::radar` widget; this observer just consumes the resulting
/// event.
fn on_tactical_radar_blip_clicked(
    trigger: On<RadarBlipClicked>,
    sources: Query<&RadarEntityUuid>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    let source = trigger.event().source;
    let lookup = sources.get(source);
    crate::wasm_log!(
        "[radar-instr 4] on_tactical_radar_blip_clicked: radar={:?} source={:?} uuid_lookup={}",
        trigger.event().radar, source,
        match &lookup { Ok(u) => format!("Some({})", u.0), Err(_) => "None".into() }
    );
    if let Ok(uuid) = lookup {
        crate::wasm_log!("[radar-instr 5] writing SetTarget uuid={}", uuid.0);
        outbound.write(OutboundClientMessage(set_target_message(uuid.0.clone())));
    }
}

/// Secondary slot: torpedo section + phaser mode + fire phasers + repair label
/// + complexity controls. Dynamically spawns per-bank fire buttons from `banks`
/// and per-tube radio buttons + status labels from `tubes`.
fn fill_tactical_controls(
    commands: &mut Commands,
    container: Entity,
    banks: &[PhaserBankClientConfig],
    tubes: &[TorpedoTubeClientConfig],
) {
    let col = commands
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::FlexStart,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            row_gap: Val::Px(12.0),
            ..default()
        })
        .id();
    commands.entity(container).add_child(col);

    // â”€â”€ Torpedo section (only if there are tubes) â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    if !tubes.is_empty() {
        let torpedo_container = commands
            .spawn((
                HideableElement("torpedo_tube_selector".into()),
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(8.0),
                    ..default()
                },
            ))
            .id();
        commands.entity(col).add_child(torpedo_container);

        // Torpedo count label
        let count_label = commands
            .spawn((
                TorpedoCountLabel,
                Text::new("Torpedoes: 10"),
                TextFont { font_size: 16.0, ..default() },
                TextColor(Color::srgb(0.8, 0.8, 0.2)),
            ))
            .id();
        commands.entity(torpedo_container).add_child(count_label);

        // Torpedo tube RadioGroup (one button per tube)
        let tube_btn_configs: Vec<RadioButtonConfig> = (0..tubes.len())
            .map(|_| RadioButtonConfig {
                size: ButtonSize::Rect { width: 80.0, height: 36.0 },
            })
            .collect();

        let radio_group = RadioGroup::spawn(
            commands,
            tube_btn_configs,
            tube_visuals(),
            None,
        );
        let tube_ids: Vec<String> = tubes.iter().map(|t| t.id.clone()).collect();
        commands.entity(radio_group)
            .insert(TubeRadioGroup(tube_ids.clone()))
            .observe(on_tube_selected);
        commands.entity(torpedo_container).add_child(radio_group);

        commands.insert_resource(TubeButtonLabelsPending);

        // Tube status labels row
        let status_row = commands
            .spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(8.0),
                ..default()
            })
            .id();
        for tube in tubes {
            let label = commands
                .spawn((
                    TubeStatusLabel(tube.id.clone()),
                    Text::new("LOADED"),
                    TextFont { font_size: 12.0, ..default() },
                    TextColor(Color::srgb(0.3, 1.0, 0.3)),
                    Node {
                        min_width: Val::Px(70.0),
                        ..default()
                    },
                ))
                .id();
            commands.entity(status_row).add_child(label);
        }
        commands.entity(torpedo_container).add_child(status_row);

        // Fire Torpedo button
        let fire_torpedo_btn = spawn_gui_button(
            commands,
            ButtonSize::Rect { width: 200.0, height: 52.0 },
            torpedo_fire_visuals(),
        );
        let fire_torpedo_label = commands
            .spawn((
                FireTorpedoLabel,
                Text::new("SELECT TUBE"),
                TextFont { font_size: 22.0, ..default() },
                TextColor(Color::srgb(0.5, 0.5, 0.5)),
            ))
            .id();
        commands.entity(fire_torpedo_btn)
            .insert((FireTorpedoButton, Disabled))
            .add_child(fire_torpedo_label)
            .observe(on_fire_torpedo_pressed);
        commands.entity(torpedo_container).add_child(fire_torpedo_btn);
    }

    // â”€â”€ Phaser Mode toggle button (hideable as "phaser_mode_selector") â”€â”€
    let mode_btn = spawn_gui_button(
        commands,
        ButtonSize::Rect { width: 200.0, height: 44.0 },
        mode_visuals(),
    );
    let mode_label = commands
        .spawn((
            PhaserModeLabel,
            Text::new("Mode: AUTO"),
            TextFont { font_size: 18.0, ..default() },
            TextColor(Color::srgb(0.7, 0.7, 1.0)),
        ))
        .id();
    commands.entity(mode_btn)
        .insert(HideableElement("phaser_mode_selector".into()))
        .add_child(mode_label)
        .observe(on_phaser_mode_pressed);
    commands.entity(col).add_child(mode_btn);

    // â”€â”€ Per-bank Fire Phasers buttons â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    for bank in banks {
        let fire_phaser_btn = spawn_gui_button(
            commands,
            ButtonSize::Rect { width: 200.0, height: 52.0 },
            fire_visuals(),
        );
        let label_text = format!("FIRE {}", bank.id.to_uppercase());
        let fire_phaser_label = commands
            .spawn((
                FirePhaserLabel(bank.id.clone()),
                Text::new(label_text),
                TextFont { font_size: 22.0, ..default() },
                TextColor(Color::srgb(1.0, 0.5, 0.2)),
            ))
            .id();
        commands.entity(fire_phaser_btn)
            .insert(FirePhaserButton(bank.id.clone()))
            .add_child(fire_phaser_label)
            .observe(on_fire_phaser_pressed);
        commands.entity(col).add_child(fire_phaser_btn);
    }

    // â”€â”€ Repair icon label â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    let repair_label = commands
        .spawn((
            RepairIconLabel,
            Text::new(""),
            TextFont { font_size: 14.0, ..default() },
            TextColor(Color::srgb(0.8, 0.5, 0.2)),
        ))
        .id();
    commands.entity(col).add_child(repair_label);

    // â”€â”€ Complexity dropdown row â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    let dropdown = commands
        .spawn((
            ComplexityDropdownRoot,
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(8.0),
                padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
                ..default()
            },
            Visibility::Hidden,
            BackgroundColor(Color::srgb(0.08, 0.08, 0.12)),
        ))
        .id();
    commands.entity(dropdown).with_children(|row| {
        row.spawn((
            Text::new("Complexity:"),
            TextFont { font_size: 13.0, ..default() },
            TextColor(Color::srgb(0.6, 0.7, 0.8)),
        ));
        for (preset, label) in [("Low", "Low"), ("Std", "Normal")] {
            row.spawn((
                ComplexityPresetButton(preset.to_string()),
                Button,
                Node {
                    padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.15, 0.20, 0.35)),
            )).with_children(|btn| {
                btn.spawn((
                    Text::new(label),
                    TextFont { font_size: 13.0, ..default() },
                    TextColor(Color::srgb(0.7, 0.8, 1.0)),
                ));
            });
        }
    });
    commands.entity(col).add_child(dropdown);

    // â”€â”€ Complexity pop-up overlay â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    let popup = commands
        .spawn((
            ComplexityPopupRoot,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                right: Val::Px(0.0),
                bottom: Val::Px(0.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                row_gap: Val::Px(12.0),
                ..default()
            },
            Visibility::Hidden,
            BackgroundColor(Color::srgba(0.05, 0.05, 0.15, 0.95)),
        ))
        .id();
    commands.entity(popup).with_children(|p| {
        p.spawn((
            Text::new("Choose Complexity Preset"),
            TextFont { font_size: 20.0, ..default() },
            TextColor(Color::srgb(0.8, 0.8, 1.0)),
        ));
        p.spawn((
            Text::new("Select a complexity level for this console."),
            TextFont { font_size: 13.0, ..default() },
            TextColor(Color::srgb(0.6, 0.6, 0.8)),
        ));
        for (preset, label) in [("Low", "Low"), ("Std", "Normal")] {
            p.spawn((
                ComplexityPresetButton(preset.to_string()),
                Button,
                Node {
                    padding: UiRect::axes(Val::Px(32.0), Val::Px(12.0)),
                    min_width: Val::Px(180.0),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.15, 0.25, 0.40)),
            )).with_children(|btn| {
                btn.spawn((
                    Text::new(label),
                    TextFont { font_size: 18.0, ..default() },
                    TextColor(Color::srgb(0.7, 0.8, 1.0)),
                ));
            });
        }
        p.spawn((
            ComplexityPopupConfirm,
            Button,
            Node {
                padding: UiRect::axes(Val::Px(48.0), Val::Px(12.0)),
                ..default()
            },
            BackgroundColor(Color::srgb(0.10, 0.40, 0.20)),
        )).with_children(|btn| {
            btn.spawn((
                Text::new("Confirm"),
                TextFont { font_size: 18.0, ..default() },
                TextColor(Color::srgb(0.5, 1.0, 0.5)),
            ));
        });
    });
    commands.entity(col).add_child(popup);
}

// â”€â”€ Orientation respawn â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

fn respawn_weapons_on_orientation_change(
    orientation: Option<Res<DeviceOrientation>>,
    panel: Query<Entity, With<WeaponsPanel>>,
    mut commands: Commands,
) {
    let Some(orientation) = orientation else { return };
    if !orientation.is_changed() || orientation.is_added() {
        return;
    }
    for entity in panel.iter() {
        commands.entity(entity).despawn();
    }
    commands.remove_resource::<WeaponsPanelSpawned>();
    commands.remove_resource::<WeaponsPanelLayoutKey>();
}

/// Compares the current `WeaponsPanelLayoutKey` against the lobby's
/// `ship_config`. If the bank or tube id lists differ, despawn the panel and
/// clear the spawn marker so `spawn_weapons_ui` rebuilds it next frame.
fn respawn_weapons_on_layout_change(
    lobby: Res<LobbyState>,
    layout: Option<Res<WeaponsPanelLayoutKey>>,
    panel: Query<Entity, With<WeaponsPanel>>,
    mut commands: Commands,
) {
    let Some(layout) = layout else { return };
    let want = WeaponsPanelLayoutKey::from_ship_config(&lobby.ship_config);
    if *layout == want {
        return;
    }
    for entity in panel.iter() {
        commands.entity(entity).despawn();
    }
    commands.remove_resource::<WeaponsPanelSpawned>();
    commands.remove_resource::<WeaponsPanelLayoutKey>();
}

// â”€â”€ Tube button labels post-setup â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// Resource flag: tube button labels haven't been added yet.
#[derive(Resource)]
struct TubeButtonLabelsPending;

/// One-shot system: once the RadioGroup children exist (deferred spawn),
/// add text label children to each member button using the tube ids stored on
/// the `TubeRadioGroup` component.
fn add_tube_button_labels(
    mut commands: Commands,
    pending: Option<Res<TubeButtonLabelsPending>>,
    groups: Query<(&Children, &TubeRadioGroup)>,
) {
    if pending.is_none() {
        return;
    }
    for (children, group) in groups.iter() {
        let want = group.0.len();
        if want == 0 || children.len() < want {
            // Children not yet resolved â€” try again next frame.
            return;
        }
        for (idx, child) in children.iter().take(want).enumerate() {
            if let Some(label_text) = group.0.get(idx) {
                let label = tube_label_short(label_text);
                commands.entity(child).with_children(|btn| {
                    btn.spawn((
                        Text::new(label),
                        TextFont { font_size: 14.0, ..default() },
                        TextColor(Color::srgb(0.6, 0.8, 1.0)),
                    ));
                });
            }
        }
        commands.remove_resource::<TubeButtonLabelsPending>();
        return;
    }
}

/// Human-readable short label for a torpedo tube id. Hardcodes friendly
/// abbreviations for the canonical fore_port/fore_starboard/aft trio and
/// falls back to upper-casing the raw id for unknown tubes.
fn tube_label_short(tube_id: &str) -> String {
    match tube_id {
        "fore_port"      => "FWD PORT".to_string(),
        "fore_starboard" => "FWD STBD".to_string(),
        "aft"            => "AFT".to_string(),
        other            => other.to_uppercase().replace('_', " "),
    }
}

// â”€â”€ Visibility system â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

fn toggle_weapons_panel_visibility(
    lobby: Res<LobbyState>,
    token: Res<LocalPlayerToken>,
    active: Res<ActiveConsole>,
    mut panel: Query<&mut Visibility, With<WeaponsPanel>>,
) {
    let visible = weapons_panel_visible(&lobby, &token.0, &active);
    for mut vis in panel.iter_mut() {
        *vis = if visible { Visibility::Visible } else { Visibility::Hidden };
    }
}

// â”€â”€ RadioGroup â†’ SelectedTube observer â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// Observer on the `TubeRadioGroup` entity: maps the selected member entity
/// to a `TorpedoTube` id (looking up the index in `children` and indexing into
/// the stored tube-id list on `TubeRadioGroup`) and updates `SelectedTube`.
fn on_tube_selected(
    trigger: On<RadioSelected>,
    groups: Query<(&Children, &TubeRadioGroup)>,
    mut selected: ResMut<SelectedTube>,
) {
    let group = trigger.entity;
    let member = trigger.event().member;

    if let Ok((children, tube_group)) = groups.get(group) {
        for (idx, child) in children.iter().enumerate() {
            if child == member {
                if let Some(tube) = tube_group.0.get(idx) {
                    selected.0 = Some(tube.clone());
                    return;
                }
            }
        }
    }
}

// â”€â”€ Button observers â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

fn on_fire_phaser_pressed(
    trigger: On<ButtonPressed>,
    sim: Res<ClientSimState>,
    buttons: Query<&FirePhaserButton>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    let Ok(button) = buttons.get(trigger.event().0) else { return };
    let bank_id = button.0.as_str();
    if !is_fire_button_enabled(&sim, bank_id) {
        return;
    }
    outbound.write(OutboundClientMessage(fire_phaser_message(bank_id)));
}

fn on_phaser_mode_pressed(
    _trigger: On<ButtonPressed>,
    sim: Res<ClientSimState>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    outbound.write(OutboundClientMessage(toggle_phaser_mode_message(sim.phaser_mode)));
}

fn on_fire_torpedo_pressed(
    _trigger: On<ButtonPressed>,
    selected: Res<SelectedTube>,
    sim: Res<ClientSimState>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    let Some(tube) = selected.0.clone() else { return };
    if !is_tube_loaded(&sim, &tube) || sim.torpedo_count == 0 {
        return;
    }
    outbound.write(OutboundClientMessage(fire_torpedo_message(tube, sim.current_target_uuid.clone())));
}

// â”€â”€ Phaser refresh system â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// Updates each Fire Phasers button label (one per bank) and the Phaser Mode
/// label from `ClientSimState`.
fn refresh_weapons_panel(
    sim: Res<ClientSimState>,
    mut fire_label: Query<(&mut Text, &mut TextColor, &FirePhaserLabel)>,
    mut mode_label: Query<&mut Text, (With<PhaserModeLabel>, Without<FirePhaserLabel>)>,
) {
    if !sim.is_changed() {
        return;
    }

    // Per-bank Fire Phasers button labels.
    for (mut text, mut color, label) in fire_label.iter_mut() {
        let bank_id = label.0.as_str();
        let bank = sim.bank_states.iter().find(|b| b.id == bank_id);
        let on_cooldown = bank.map(|b| b.on_cooldown).unwrap_or(false);
        let fire_enabled = is_fire_button_enabled(&sim, bank_id);
        let label_text = bank_label(bank_id);

        if on_cooldown {
            **text = format!("{} COOLING", label_text);
            *color = TextColor(Color::srgb(0.5, 0.2, 0.2));
        } else {
            **text = format!("FIRE {}", label_text);
            *color = if fire_enabled {
                TextColor(Color::srgb(1.0, 0.5, 0.2))
            } else {
                TextColor(Color::srgb(0.5, 0.3, 0.2))
            };
        }
    }

    // Phaser mode button label.
    for mut text in mode_label.iter_mut() {
        **text = format!("Mode: {}", phaser_mode_label(sim.phaser_mode));
    }
}

// Note: tactical radar filter is now sourced via the unified
// `sync_radar_widgets_from_lobby` system in `src/client/app.rs`, routed
// by `ConsoleRadar::Tactical`.

/// Inserts/removes `Disabled` on each Fire Phasers button as `sim` changes.
fn sync_fire_phaser_disabled(
    mut commands: Commands,
    sim: Res<ClientSimState>,
    fire_btn: Query<(Entity, &FirePhaserButton, Has<Disabled>)>,
) {
    if !sim.is_changed() {
        return;
    }
    for (entity, button, currently_disabled) in fire_btn.iter() {
        let fire_enabled = is_fire_button_enabled(&sim, &button.0);
        if !fire_enabled && !currently_disabled {
            commands.entity(entity).insert(Disabled);
        } else if fire_enabled && currently_disabled {
            commands.entity(entity).remove::<Disabled>();
        }
    }
}

/// Human-readable short label for a phaser bank id (`"port"` â†’ `"PORT"`).
/// Falls back to upper-casing the raw id for unknown banks.
fn bank_label(bank_id: &str) -> String {
    bank_id.to_uppercase()
}

// â”€â”€ Torpedo refresh system â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// Refresh torpedo count label, tube-status labels, and Fire Torpedo button.
fn refresh_torpedo_ui(
    mut commands: Commands,
    sim: Res<ClientSimState>,
    selected: Res<SelectedTube>,
    mut count_label: Query<&mut Text, With<TorpedoCountLabel>>,
    mut tube_status: Query<
        (&mut Text, &mut TextColor, &TubeStatusLabel),
        (Without<TorpedoCountLabel>, Without<FireTorpedoLabel>),
    >,
    mut fire_label: Query<
        (&mut Text, &mut TextColor),
        (With<FireTorpedoLabel>, Without<TorpedoCountLabel>, Without<TubeStatusLabel>),
    >,
    fire_btn: Query<(Entity, Has<Disabled>), With<FireTorpedoButton>>,
) {
    if !sim.is_changed() && !selected.is_changed() {
        return;
    }

    // Torpedo count.
    for mut text in count_label.iter_mut() {
        **text = format!("Torpedoes: {}", sim.torpedo_count);
    }

    // Per-tube status labels.
    for (mut text, mut color, label) in tube_status.iter_mut() {
        let tube_id = label.0.as_str();
        let loaded = is_tube_loaded(&sim, tube_id);
        let reload_secs = tube_reload_secs(&sim, tube_id);
        if loaded {
            **text = "LOADED".to_string();
            *color = TextColor(Color::srgb(0.3, 1.0, 0.3));
        } else {
            **text = format!("{:.0}s", reload_secs.ceil());
            *color = TextColor(Color::srgb(1.0, 0.6, 0.2));
        }
    }

    // Fire Torpedo button: enable/disable.
    let tube_ready = selected.0.as_ref().map(|t| is_tube_loaded(&sim, t)).unwrap_or(false);
    let can_fire = tube_ready && sim.torpedo_count > 0 && selected.0.is_some();

    for (entity, currently_disabled) in fire_btn.iter() {
        if !can_fire && !currently_disabled {
            commands.entity(entity).insert(Disabled);
        } else if can_fire && currently_disabled {
            commands.entity(entity).remove::<Disabled>();
        }
    }

    // Fire Torpedo button label.
    for (mut text, mut color) in fire_label.iter_mut() {
        if selected.0.is_none() {
            **text = "SELECT TUBE".to_string();
            *color = TextColor(Color::srgb(0.5, 0.5, 0.5));
        } else if sim.torpedo_count == 0 {
            **text = "NO TORPEDOES".to_string();
            *color = TextColor(Color::srgb(0.6, 0.3, 0.3));
        } else if !tube_ready {
            **text = "TUBE LOADING".to_string();
            *color = TextColor(Color::srgb(0.8, 0.5, 0.2));
        } else {
            **text = "FIRE TORPEDO".to_string();
            *color = TextColor(Color::srgb(0.3, 1.0, 0.3));
        }
    }
}

// â”€â”€ Radar entity bridge â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
// -- Radar entity bridge ----------------------------------------------

/// Thin per-console wrapper around bridge_sim_to_radar for the Tactical
/// (Weapons) radar widget.
fn bridge_client_sim_to_weapons_radar(
    mut commands: Commands,
    sim: Res<ClientSimState>,
    ship_view: Res<ShipView>,
    mut q: Query<(Entity, &ConsoleRadar, &mut RadarBlipMap)>,
) {
    let Some((widget, _, mut map)) =
        q.iter_mut().find(|(_, c, _)| **c == ConsoleRadar::Tactical)
    else {
        return;
    };
    // [radar-instr 13] dump all world entities to find why NPC blip is missing
    static LOGGED_13: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !LOGGED_13.swap(true, std::sync::atomic::Ordering::Relaxed) {
        for e in &sim.world.entities {
            crate::wasm_log!(
                "[radar-instr 13] world entity: uuid={} tags={:?} pos={:?}",
                e.uuid, e.tags, e.position
            );
        }
    }
    bridge_sim_to_radar(
        &mut commands,
        widget,
        &mut map,
        RadarCenterPose {
            x: ship_view.ship_x,
            z: ship_view.ship_z,
            yaw: ship_view.ship_yaw,
        },
        &sim.world.entities,
    );
}

/// Mirror the server-confirmed weapons target into the tactical widget's
/// `RadarTargetHighlight` so the arc renderer can highlight the locked blip.
///
/// Reads `ClientSimState::current_target_uuid`, which is populated from
/// `ServerMessage::WeaponsUpdate.target_uuid` at 10 Hz. Previously this
/// system read `last_phaser_target`, which is only set by `PhaserFired` —
/// meaning the lock indicator never appeared between tapping a blip and
/// actually firing, and a `SetTarget` followed by no fire was invisible
/// to the user.
fn sync_weapons_radar_target_highlight(
    sim: Res<ClientSimState>,
    mut widget: Query<&mut RadarTargetHighlight>,
) {
    for mut hl in widget.iter_mut() {
        if hl.0 != sim.current_target_uuid {
            crate::wasm_log!(
                "[radar-instr 10] sync_weapons_radar_target_highlight: {:?} -> {:?}",
                hl.0, sim.current_target_uuid
            );
            hl.0 = sim.current_target_uuid.clone();
        }
    }
}

// â”€â”€ Tests â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client_lobby::{ActiveConsole, LobbyState};
    use crate::messages::{GamePhase, Console, GameState, Player, ServerMessage, ShipClientConfig};
    use crate::stations_config::ShipStations;
    use std::collections::HashMap;

    fn player(token: &str, consoles: Vec<Console>) -> Player {
        Player { token: token.into(), name: "test".into(), consoles, connected: true }
    }

    fn game_state(phase: GamePhase, players: Vec<Player>) -> GameState {
        GameState { phase, players, complexity: HashMap::new(), world: None }
    }

    fn welcome(state: GameState) -> ServerMessage {
        ServerMessage::Welcome {
            state,
            ship_stations: ShipStations::default(),
            ship_config: ShipClientConfig::default(),
        }
    }

    fn in_progress_tactical_lobby(token: &str) -> LobbyState {
        let mut s = LobbyState::default();
        s.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player(token, vec![Console::Tactical])],
        )));
        s
    }

    fn no_tab() -> ActiveConsole { ActiveConsole(None) }
    fn tab(c: Console) -> ActiveConsole { ActiveConsole(Some(c)) }

    // ── weapons_panel_visible ─────────────────────────────────────────────

    #[test]
    fn weapons_panel_hidden_in_lobby_phase() {
        let lobby = LobbyState::default();
        let active = no_tab();
        assert!(!weapons_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn weapons_panel_hidden_when_player_not_tactical() {
        let mut lobby = LobbyState::default();
        lobby.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player("tok", vec![Console::Helm])],
        )));
        let active = no_tab();
        assert!(!weapons_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn weapons_panel_visible_when_sole_console_and_no_tab() {
        let lobby = in_progress_tactical_lobby("tok");
        let active = no_tab();
        assert!(weapons_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn weapons_panel_visible_when_multi_console_and_tactical_tab() {
        let mut lobby = LobbyState::default();
        lobby.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player("tok", vec![Console::Helm, Console::Tactical])],
        )));
        let active = tab(Console::Tactical);
        assert!(weapons_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn weapons_panel_hidden_when_multi_console_and_other_tab() {
        let mut lobby = LobbyState::default();
        lobby.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player("tok", vec![Console::Helm, Console::Tactical])],
        )));
        let active = tab(Console::Helm);
        assert!(!weapons_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn weapons_panel_hidden_when_multi_console_and_no_tab() {
        let mut lobby = LobbyState::default();
        lobby.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player("tok", vec![Console::Helm, Console::Tactical])],
        )));
        let active = no_tab();
        assert!(!weapons_panel_visible(&lobby, "tok", &active));
    }

    // â”€â”€ fire_phaser_message builder â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    #[test]
    fn fire_phaser_message_produces_fire_phaser() {
        use crate::messages::ClientMessage;
        let msg = fire_phaser_message("port");
        assert_eq!(msg, ClientMessage::FirePhaser { bank: "port".to_string() });
    }

    // â”€â”€ fire_torpedo_message builder â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    #[test]
    fn fire_torpedo_message_fore_port_no_target() {
        use crate::messages::ClientMessage;
        let msg = fire_torpedo_message("fore_port".to_string(), None);
        assert_eq!(msg, ClientMessage::FireTorpedo {
            tube: "fore_port".to_string(),
            target_uuid: None,
        });
    }

    #[test]
    fn fire_torpedo_message_aft_with_target() {
        use crate::messages::ClientMessage;
        let msg = fire_torpedo_message("aft".to_string(), Some("uuid-1".into()));
        assert_eq!(msg, ClientMessage::FireTorpedo {
            tube: "aft".to_string(),
            target_uuid: Some("uuid-1".into()),
        });
    }

    #[test]
    fn fire_torpedo_carries_current_target_uuid() {
        // on_fire_torpedo_pressed passes sim.current_target_uuid — verify the
        // fire_torpedo_message builder threads it through correctly.
        use crate::messages::ClientMessage;
        let msg = fire_torpedo_message("fore_port".to_string(), Some("enemy-1".into()));
        assert_eq!(msg, ClientMessage::FireTorpedo {
            tube: "fore_port".to_string(),
            target_uuid: Some("enemy-1".into()),
        });
    }

    #[test]
    fn fire_torpedo_with_no_current_target_sends_none() {
        // Dumb-fire (no current target) still works.
        use crate::messages::ClientMessage;
        let msg = fire_torpedo_message("fore_port".to_string(), None);
        assert_eq!(msg, ClientMessage::FireTorpedo {
            tube: "fore_port".to_string(),
            target_uuid: None,
        });
    }

    // â”€â”€ toggle_phaser_mode_message builder â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    #[test]
    fn toggle_phaser_mode_auto_produces_manual() {
        use crate::messages::{ClientMessage, PhaserMode};
        let msg = crate::client_sim::toggle_phaser_mode_message(PhaserMode::Auto);
        assert_eq!(msg, ClientMessage::SetPhaserMode { mode: PhaserMode::Manual });
    }

    #[test]
    fn toggle_phaser_mode_manual_produces_auto() {
        use crate::messages::{ClientMessage, PhaserMode};
        let msg = crate::client_sim::toggle_phaser_mode_message(PhaserMode::Manual);
        assert_eq!(msg, ClientMessage::SetPhaserMode { mode: PhaserMode::Auto });
    }

    // Per-bank `is_fire_button_enabled` semantics are covered in
    // `client_sim::tests::fire_button_*`; nothing to retest here.

    // â”€â”€ phaser_mode_label â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    #[test]
    fn phaser_mode_label_auto() {
        use crate::messages::PhaserMode;
        assert_eq!(phaser_mode_label(PhaserMode::Auto), "AUTO");
    }

    #[test]
    fn phaser_mode_label_manual() {
        use crate::messages::PhaserMode;
        assert_eq!(phaser_mode_label(PhaserMode::Manual), "MANUAL");
    }

    // â”€â”€ SelectedTube default â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    #[test]
    fn selected_tube_defaults_to_none() {
        let s = SelectedTube::default();
        assert_eq!(s.0, None);
    }

    // â”€â”€ Radar filter: ships + torpedoes only â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    #[test]
    fn weapons_radar_filter_includes_ships() {
        use crate::gui::is_on_radar;
        let filter = RadarFilter(std::collections::HashSet::from([
            "ship".to_string(),
            "missile".to_string(),
        ]));
        assert!(is_on_radar(&filter, &["ship".to_string()]));
    }

    #[test]
    fn weapons_radar_filter_includes_missiles() {
        use crate::gui::is_on_radar;
        let filter = RadarFilter(std::collections::HashSet::from([
            "ship".to_string(),
            "missile".to_string(),
        ]));
        assert!(is_on_radar(&filter, &["missile".to_string()]));
    }

    #[test]
    fn weapons_radar_filter_excludes_asteroids() {
        use crate::gui::is_on_radar;
        let filter = RadarFilter(std::collections::HashSet::from([
            "ship".to_string(),
            "missile".to_string(),
        ]));
        assert!(!is_on_radar(&filter, &["asteroid".to_string()]));
    }

    // â”€â”€ StateVisuals: five widget states render distinctly â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    #[test]
    fn fire_visuals_has_distinct_five_states() {
        use crate::gui::resolve_visual;
        let v = fire_visuals();
        let idle     = resolve_visual(&v, false, false, false, false).color;
        let hover    = resolve_visual(&v, false, false, false, true ).color;
        let active   = resolve_visual(&v, false, false, true,  false).color;
        let press    = resolve_visual(&v, false, true,  false, false).color;
        let disabled = resolve_visual(&v, true,  false, false, false).color;
        assert_ne!(idle, hover);
        assert_ne!(idle, active);
        assert_ne!(idle, press);
        assert_ne!(idle, disabled);
    }

    #[test]
    fn torpedo_fire_visuals_has_distinct_five_states() {
        use crate::gui::resolve_visual;
        let v = torpedo_fire_visuals();
        let idle     = resolve_visual(&v, false, false, false, false).color;
        let hover    = resolve_visual(&v, false, false, false, true ).color;
        let active   = resolve_visual(&v, false, false, true,  false).color;
        let press    = resolve_visual(&v, false, true,  false, false).color;
        let disabled = resolve_visual(&v, true,  false, false, false).color;
        assert_ne!(idle, hover);
        assert_ne!(idle, active);
        assert_ne!(idle, press);
        assert_ne!(idle, disabled);
    }

    #[test]
    fn tube_visuals_active_state_is_highlighted() {
        use crate::gui::resolve_visual;
        let v = tube_visuals();
        let idle   = resolve_visual(&v, false, false, false, false).color;
        let active = resolve_visual(&v, false, false, true,  false).color;
        assert_ne!(idle, active, "active tube should look different from idle");
    }

    #[test]
    fn mode_visuals_has_distinct_five_states() {
        use crate::gui::resolve_visual;
        let v = mode_visuals();
        let idle     = resolve_visual(&v, false, false, false, false).color;
        let hover    = resolve_visual(&v, false, false, false, true ).color;
        let active   = resolve_visual(&v, false, false, true,  false).color;
        let press    = resolve_visual(&v, false, true,  false, false).color;
        let disabled = resolve_visual(&v, true,  false, false, false).color;
        assert_ne!(idle, hover);
        assert_ne!(idle, active);
        assert_ne!(idle, press);
        assert_ne!(idle, disabled);
    }

    // ── on_tactical_radar_blip_clicked ────────────────────────────────────

    #[test]
    fn tactical_radar_blip_clicked_emits_set_target_with_source_uuid() {
        // End-to-end wiring test for the tap-to-target chain:
        //
        // 1. Spawn a "source" ECS entity carrying RadarEntityUuid (mimicking
        //    what the sim-to-radar bridge spawns for each visible entity).
        // 2. Spawn a "radar" entity and register the per-radar observer.
        // 3. Fire RadarBlipClicked at the radar with `source` in the payload
        //    (this is what `detect_radar_blip_press` does in production).
        // 4. Assert one OutboundClientMessage::SetTarget { uuid } was written,
        //    carrying the source entity's RadarEntityUuid.
        //
        // This is the regression test for the bug where the prior
        // `Pointer<Click>` observer silently returned because trigger.entity
        // was the blip UI node, not the radar Node.

        use bevy::ecs::message::Messages;
        use crate::client_app::OutboundClientMessage;
        use crate::messages::ClientMessage;

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_message::<OutboundClientMessage>();

        // The source entity is what RadarBlipClicked.source points to —
        // the ECS entity that carries the wire-level uuid.
        let source = app.world_mut()
            .spawn(RadarEntityUuid("uuid-deadbeef".to_string()))
            .id();

        // The radar entity is what the observer is registered on.
        let radar = app.world_mut().spawn_empty().id();
        app.world_mut()
            .entity_mut(radar)
            .observe(on_tactical_radar_blip_clicked);

        // Fire the event exactly as detect_radar_blip_press would.
        app.world_mut().trigger(RadarBlipClicked { radar, source });

        // Drain outbound queue and assert we got the expected SetTarget.
        let messages = app.world_mut()
            .resource_mut::<Messages<OutboundClientMessage>>()
            .drain()
            .collect::<Vec<_>>();
        assert_eq!(messages.len(), 1, "exactly one outbound message: {messages:?}");
        match &messages[0].0 {
            ClientMessage::SetTarget { uuid } => {
                assert_eq!(uuid, "uuid-deadbeef");
            }
            other => panic!("expected SetTarget, got {other:?}"),
        }
    }

    #[test]
    fn tactical_radar_blip_clicked_ignores_source_without_uuid() {
        // If the source entity somehow lacks RadarEntityUuid (e.g. a stale
        // blip mid-despawn), the observer must silently no-op rather than
        // panicking or sending a malformed SetTarget.

        use bevy::ecs::message::Messages;
        use crate::client_app::OutboundClientMessage;

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_message::<OutboundClientMessage>();

        let source = app.world_mut().spawn_empty().id(); // no RadarEntityUuid
        let radar = app.world_mut().spawn_empty().id();
        app.world_mut()
            .entity_mut(radar)
            .observe(on_tactical_radar_blip_clicked);

        app.world_mut().trigger(RadarBlipClicked { radar, source });

        let messages = app.world_mut()
            .resource_mut::<Messages<OutboundClientMessage>>()
            .drain()
            .collect::<Vec<_>>();
        assert!(messages.is_empty(), "no message expected: {messages:?}");
    }
}
