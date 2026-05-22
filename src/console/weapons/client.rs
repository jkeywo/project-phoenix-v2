//! Client-side Weapons Panel plugin — migrated to `src/gui/` library widgets.
//!
//! Owns all Tactical console UI: fire phasers button (`GuiButton`), phaser mode
//! toggle (`GuiButton`), torpedo tube selector (`RadioGroup`), fire torpedo
//! button (`GuiButton`), torpedo count / tube status readouts, and a
//! `GenericRadar` (WorldFixed, Ships + Torpedoes filter).
//!
//! No per-button marker-component query systems remain. All callbacks are wired
//! via observers at spawn time.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use std::collections::HashMap;

use crate::client::console_shell::ConsoleShell;
use crate::client_app::{
    WeaponsPanel, OutboundClientMessage, RepairIconLabel,
    HideableElement, ComplexityPopupRoot, ComplexityPresetButton, ComplexityPopupConfirm,
    ComplexityDropdownRoot,
};
use crate::client_lobby::{ActiveConsole, LobbyState, LobbyView, LocalPlayerToken};
use crate::client_sim::{
    fire_phaser_message, nearest_entity_to_point, set_target_message,
    toggle_phaser_mode_message, fire_torpedo_message,
    is_fire_button_enabled, phaser_mode_label, ClientSimState,
};
use crate::gui::{
    default_layer_colour, is_on_radar, layer_to_icon, project_radar_entity,
    region_shape_from_snapshot, spawn_gui_button, tags_to_radar_layer, ButtonPressed, ButtonSize,
    GenericRadar, GenericRadarWidget, OnRadar, OrientationMode, RadarAppearance, RadarCenter,
    RadarClipMode, RadarFilter, RadarIcon, RadarLayer, StateVisuals, RadioButtonConfig, RadioGroup,
    RadioSelected, Disabled,
};
use crate::messages::{Console, GamePhase, TorpedoTube};
use crate::phone_border::framing::{DeviceOrientation, PhoneAssets};
use crate::ship_view::ShipView;

// ── Pure visibility helper ────────────────────────────────────────────

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

// ── Marker components ─────────────────────────────────────────────────

/// Marks the text label inside the Fire Phasers button (shows cooldown status).
#[derive(Component)]
struct FirePhaserLabel;

/// Marks the text label inside the Phaser Mode toggle button.
#[derive(Component)]
struct PhaserModeLabel;

/// Marks the text label inside the Fire Torpedo button.
#[derive(Component)]
struct FireTorpedoLabel;

/// Marks the torpedo count text label.
#[derive(Component)]
struct TorpedoCountLabel;

/// Marks the label that shows tube reload status. Stores which tube it displays.
#[derive(Component)]
struct TubeStatusLabel(TorpedoTube);

/// Marks the Fire Phasers `GuiButton` entity.
#[derive(Component)]
struct FirePhaserButton;

/// Marks the Fire Torpedo `GuiButton` entity.
#[derive(Component)]
struct FireTorpedoButton;

/// Marks the `RadioGroup` entity used for torpedo tube selection.
#[derive(Component)]
struct TubeRadioGroup;

// ── Resources ─────────────────────────────────────────────────────────

/// Tracks which torpedo tube is currently selected on the Weapons console.
///
/// `None` means no tube is selected. Updated when the `RadioGroup` fires
/// `RadioSelected`.
#[derive(Resource, Default, Clone, PartialEq, Eq, Debug)]
pub struct SelectedTube(pub Option<TorpedoTube>);

/// Persistent entity IDs for weapons-specific radar components.
#[derive(Resource, Default)]
struct WeaponsRadarEntities {
    center: Option<Entity>,
    blips: HashMap<String, Entity>,
}

// ── State visuals helpers ─────────────────────────────────────────────

/// Danger (red) button visuals — used for Fire Phasers.
fn fire_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.40, 0.10, 0.10), // idle
        Color::srgb(0.55, 0.12, 0.12), // hover
        Color::srgb(0.60, 0.10, 0.10), // active
        Color::srgb(0.70, 0.15, 0.15), // press
        Color::srgb(0.15, 0.05, 0.05), // disabled
    )
}

/// Neutral (blue-grey) button visuals — used for Phaser Mode toggle.
fn mode_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.15, 0.15, 0.35), // idle
        Color::srgb(0.20, 0.20, 0.45), // hover
        Color::srgb(0.25, 0.25, 0.55), // active
        Color::srgb(0.30, 0.30, 0.60), // press
        Color::srgb(0.08, 0.08, 0.20), // disabled
    )
}

/// Safe (green) button visuals — used for Fire Torpedo.
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

// ── Plugin ────────────────────────────────────────────────────────────

/// Marker resource set once the weapons UI has been spawned.
#[derive(Resource)]
pub struct WeaponsPanelSpawned;

// ── Plugin ────────────────────────────────────────────────────────────

/// Plugin that owns all Tactical console UI and systems.
pub struct WeaponsPanelPlugin;

impl Plugin for WeaponsPanelPlugin {
    fn build(&self, app: &mut App) {
        app
            .init_resource::<SelectedTube>()
            .init_resource::<WeaponsRadarEntities>()
            .add_systems(Update, (
                spawn_weapons_ui.run_if(not(resource_exists::<WeaponsPanelSpawned>)),
                toggle_weapons_panel_visibility,
                add_tube_button_labels,
                refresh_weapons_panel,
                sync_fire_phaser_disabled,
                refresh_torpedo_ui,
                bridge_client_sim_to_weapons_radar,
                respawn_weapons_on_orientation_change,
            ));
    }
}

// ── Spawn (ConsoleShell) ──────────────────────────────────────────────

fn spawn_weapons_ui(
    mut commands: Commands,
    assets: Option<Res<PhoneAssets>>,
    old_panel: Query<Entity, With<WeaponsPanel>>,
    old_help: Query<(Entity, &crate::client::elements::HelpOverlay)>,
    orientation: Option<Res<DeviceOrientation>>,
) {
    let Some(assets) = assets else { return };
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

    let shell = ConsoleShell::spawn(
        &mut commands,
        assets.helm_panel_bg.clone(),
        is_landscape,
        crate::client::elements::HelpPanel::Tactical,
        |commands: &mut Commands, primary: Entity| {
            fill_tactical_radar(commands, primary);
        },
        |commands: &mut Commands, secondary: Entity| {
            fill_tactical_controls(commands, secondary);
        },
        &assets,
    );

    commands.entity(shell.root).insert((WeaponsPanel, Visibility::Hidden));
}

// ── Fill helpers ──────────────────────────────────────────────────────

/// Primary slot: tactical radar (GenericRadar, WorldFixed, Ships + Torpedoes).
fn fill_tactical_radar(commands: &mut Commands, container: Entity) {
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

    let radar_filter = RadarFilter(std::collections::HashSet::from([
        RadarLayer::Ship,
        RadarLayer::Missile,
        RadarLayer::Station,
    ]));
    let radar = GenericRadar::spawn(
        commands,
        crate::client_sim::WEAPONS_RADAR_RANGE,
        OrientationMode::WorldFixed,
        radar_filter,
        None,
        None,
        RadarClipMode::Circle,
    );
    commands.entity(radar).insert((Node {
        width:  Val::Px(240.0),
        height: Val::Px(240.0),
        border: UiRect::all(Val::Px(1.0)),
        aspect_ratio: Some(1.0),
        position_type: PositionType::Relative,
        ..default()
    }, TacticalRadarTapTarget));
    commands.entity(radar).observe(on_tactical_radar_tap);
    commands.entity(col).add_child(radar);
}

/// Marker for the tactical-radar `Node` that owns the tap-to-target observer.
#[derive(Component)]
pub struct TacticalRadarTapTarget;

/// Convert a tap (in **logical** window pixels, as delivered by Bevy's picking
/// `Pointer<Click>::pointer_location.position`) into the radar Node's local
/// **physical** pixel space, given the node's physical top-left, its physical
/// size and the window's `scale_factor`.
///
/// The renderer (`gui/radar.rs`) lays out blips using `ComputedNode::size()`,
/// which is in physical pixels. To compare a tap against those blip centres
/// we must therefore promote the logical tap into the same physical basis.
///
/// Returns `(local_x, local_y)` with origin at the node's top-left.
pub fn radar_local_pixel(
    tap_logical: Vec2,
    node_physical_top_left: Vec2,
    _node_physical_size: Vec2,
    scale_factor: f32,
) -> Vec2 {
    let tap_physical = tap_logical * scale_factor;
    tap_physical - node_physical_top_left
}

/// Observer: when the tactical radar is tapped, find the nearest ship/missile
/// blip to the tap point and dispatch `SetTarget { uuid }`.
///
/// `pointer_location.position` is in **logical** window pixels, while
/// `ComputedNode::size()` and `GlobalTransform::translation()` for UI nodes are
/// in **physical** pixels. We convert the tap to physical pixels via the
/// primary window's `scale_factor()` so the comparison is consistent on
/// high-DPI displays (phones).
fn on_tactical_radar_tap(
    trigger: On<Pointer<Click>>,
    radars: Query<(&ComputedNode, &GlobalTransform, &GenericRadarWidget), With<TacticalRadarTapTarget>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    sim: Res<ClientSimState>,
    ship_view: Res<ShipView>,
    mut commands: Commands,
) {
    let entity = trigger.entity;
    let Ok((computed, gt, widget)) = radars.get(entity) else {
        return;
    };
    let Ok(window) = windows.single() else {
        return;
    };
    let scale = window.scale_factor() as f32;

    // Radar centre + size in physical pixels (UI GlobalTransform is centred).
    let size = computed.size();
    let radar_radius_px = size.x.min(size.y) * 0.5;
    if radar_radius_px <= 0.0 {
        return;
    }
    let centre = gt.translation();
    let centre_xy = Vec2::new(centre.x, centre.y);
    let top_left = centre_xy - size * 0.5;

    // Tap point in radar-local physical pixel space (origin top-left).
    let tap = radar_local_pixel(
        trigger.event().pointer_location.position,
        top_left,
        size,
        scale,
    );

    // Convert the tap to the same (left, top) basis the renderer uses for
    // blip centres: blip centre = (radius + nx*radius, radius - ny*radius).
    let click_local = (tap.x, tap.y);

    // Project every entity matching the radar's filter to radar-local pixels.
    let projected: Vec<(String, f32, f32)> = sim
        .world
        .entities
        .iter()
        .filter_map(|snap| {
            let layer = tags_to_radar_layer(&snap.tags)?;
            if !is_on_radar(&widget.filter, layer) {
                return None;
            }
            let (nx, ny) = project_radar_entity(
                snap.x(),
                snap.z(),
                ship_view.ship_x,
                ship_view.ship_z,
                ship_view.ship_yaw,
                widget.range,
                snap.radius_or_zero(),
                &widget.orientation,
            )?;
            // Cull blips outside the circular radar boundary.
            if nx * nx + ny * ny > 1.0 {
                return None;
            }
            let px = radar_radius_px + nx * radar_radius_px;
            let py = radar_radius_px - ny * radar_radius_px;
            Some((snap.uuid.clone(), px, py))
        })
        .collect();

    if let Some(uuid) = nearest_entity_to_point(click_local, &projected) {
        let msg = set_target_message(uuid);
        commands.queue(|world: &mut World| {
            world.write_message(OutboundClientMessage(msg));
        });
    }
}

/// Secondary slot: torpedo section + phaser mode + fire phasers + repair label
/// + complexity controls.
fn fill_tactical_controls(commands: &mut Commands, container: Entity) {
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

    // ── Torpedo section container (hideable as "torpedo_tube_selector") ──
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

    // Torpedo tube RadioGroup
    let tube_btn_configs: Vec<RadioButtonConfig> = (0..3)
        .map(|_| RadioButtonConfig {
            size: ButtonSize::Rect { width: 80.0, height: 36.0 },
            click_sound: None,
        })
        .collect();

    let radio_group = RadioGroup::spawn(
        commands,
        tube_btn_configs,
        tube_visuals(),
        None,
    );
    commands.entity(radio_group)
        .insert(TubeRadioGroup)
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
    for tube in [
        "fore_port".to_string(),
        "fore_starboard".to_string(),
        "aft".to_string(),
    ] {
        let label = commands
            .spawn((
                TubeStatusLabel(tube),
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
        None,
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

    // ── Phaser Mode toggle button (hideable as "phaser_mode_selector") ──
    let mode_btn = spawn_gui_button(
        commands,
        ButtonSize::Rect { width: 200.0, height: 44.0 },
        mode_visuals(),
        None,
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

    // ── Fire Phasers button ───────────────────────────────────────────
    let fire_phaser_btn = spawn_gui_button(
        commands,
        ButtonSize::Rect { width: 200.0, height: 52.0 },
        fire_visuals(),
        None,
    );
    let fire_phaser_label = commands
        .spawn((
            FirePhaserLabel,
            Text::new("FIRE PHASERS"),
            TextFont { font_size: 22.0, ..default() },
            TextColor(Color::srgb(1.0, 0.5, 0.2)),
        ))
        .id();
    commands.entity(fire_phaser_btn)
        .insert(FirePhaserButton)
        .add_child(fire_phaser_label)
        .observe(on_fire_phaser_pressed);
    commands.entity(col).add_child(fire_phaser_btn);

    // ── Repair icon label ─────────────────────────────────────────────
    let repair_label = commands
        .spawn((
            RepairIconLabel,
            Text::new(""),
            TextFont { font_size: 14.0, ..default() },
            TextColor(Color::srgb(0.8, 0.5, 0.2)),
        ))
        .id();
    commands.entity(col).add_child(repair_label);

    // ── Complexity dropdown row ───────────────────────────────────────
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

    // ── Complexity pop-up overlay ─────────────────────────────────────
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

// ── Orientation respawn ──────────────────────────────────────────────

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
}

// ── Tube button labels post-setup ─────────────────────────────────────

/// Resource flag: tube button labels haven't been added yet.
#[derive(Resource)]
struct TubeButtonLabelsPending;

/// One-shot system: once the RadioGroup children exist (deferred spawn),
/// add text label children to each member button.
fn add_tube_button_labels(
    mut commands: Commands,
    pending: Option<Res<TubeButtonLabelsPending>>,
    groups: Query<&Children, With<TubeRadioGroup>>,
) {
    if pending.is_none() {
        return;
    }
    let tube_labels = ["FWD PORT", "FWD STBD", "AFT"];
    for children in groups.iter() {
        if children.len() < 3 {
            // Children not yet resolved — try again next frame.
            return;
        }
        for (idx, child) in children.iter().take(3).enumerate() {
            if let Some(&label_text) = tube_labels.get(idx) {
                commands.entity(child).with_children(|btn| {
                    btn.spawn((
                        Text::new(label_text),
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

// ── Visibility system ─────────────────────────────────────────────────

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

// ── RadioGroup → SelectedTube observer ───────────────────────────────

/// Observer on the `TubeRadioGroup` entity: maps selected member to a
/// `TorpedoTube` by children order and updates `SelectedTube`.
fn on_tube_selected(
    trigger: On<RadioSelected>,
    children_q: Query<&Children>,
    mut selected: ResMut<SelectedTube>,
) {
    let group = trigger.entity;
    let member = trigger.event().member;

    let tubes = [
        "fore_port".to_string(),
        "fore_starboard".to_string(),
        "aft".to_string(),
    ];
    if let Ok(children) = children_q.get(group) {
        for (idx, child) in children.iter().enumerate() {
            if child == member {
                if let Some(tube) = tubes.get(idx) {
                    selected.0 = Some(tube.clone());
                    return;
                }
            }
        }
    }
}

// ── Button observers ──────────────────────────────────────────────────

fn on_fire_phaser_pressed(
    _trigger: On<ButtonPressed>,
    sim: Res<ClientSimState>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    if !is_fire_button_enabled(&sim) {
        return;
    }
    outbound.write(OutboundClientMessage(fire_phaser_message()));
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
    let loaded = match tube.as_str() {
        "fore_port"      => sim.fore_port_loaded,
        "fore_starboard" => sim.fore_starboard_loaded,
        "aft"            => sim.aft_loaded,
        _                => false,
    };
    if !loaded || sim.torpedo_count == 0 {
        return;
    }
    outbound.write(OutboundClientMessage(fire_torpedo_message(tube, None)));
}

// ── Phaser refresh system ─────────────────────────────────────────────

/// Updates Fire Phasers button label and Phaser Mode label from `ClientSimState`.
fn refresh_weapons_panel(
    sim: Res<ClientSimState>,
    mut fire_label: Query<(&mut Text, &mut TextColor), With<FirePhaserLabel>>,
    mut mode_label: Query<&mut Text, (With<PhaserModeLabel>, Without<FirePhaserLabel>)>,
) {
    if !sim.is_changed() {
        return;
    }
    let fire_enabled = is_fire_button_enabled(&sim);

    // Fire Phasers button label.
    for (mut text, mut color) in fire_label.iter_mut() {
        if sim.on_cooldown {
            **text = "COOLING DOWN".to_string();
            *color = TextColor(Color::srgb(0.5, 0.2, 0.2));
        } else {
            **text = "FIRE PHASERS".to_string();
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

/// Inserts/removes `Disabled` on the Fire Phasers button as `sim` changes.
fn sync_fire_phaser_disabled(
    mut commands: Commands,
    sim: Res<ClientSimState>,
    fire_btn: Query<(Entity, Has<Disabled>), With<FirePhaserButton>>,
) {
    if !sim.is_changed() {
        return;
    }
    let fire_enabled = is_fire_button_enabled(&sim);
    for (entity, currently_disabled) in fire_btn.iter() {
        if !fire_enabled && !currently_disabled {
            commands.entity(entity).insert(Disabled);
        } else if fire_enabled && currently_disabled {
            commands.entity(entity).remove::<Disabled>();
        }
    }
}

// ── Torpedo refresh system ────────────────────────────────────────────

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
        let (loaded, reload_secs) = match label.0.as_str() {
            "fore_port"      => (sim.fore_port_loaded,      sim.fore_port_reload_secs),
            "fore_starboard" => (sim.fore_starboard_loaded, sim.fore_starboard_reload_secs),
            "aft"            => (sim.aft_loaded,            sim.aft_reload_secs),
            _                => (true, 0.0),
        };
        if loaded {
            **text = "LOADED".to_string();
            *color = TextColor(Color::srgb(0.3, 1.0, 0.3));
        } else {
            **text = format!("{:.0}s", reload_secs.ceil());
            *color = TextColor(Color::srgb(1.0, 0.6, 0.2));
        }
    }

    // Fire Torpedo button: enable/disable.
    let tube_ready = selected.0.as_ref().map(|t| match t.as_str() {
        "fore_port"      => sim.fore_port_loaded,
        "fore_starboard" => sim.fore_starboard_loaded,
        "aft"            => sim.aft_loaded,
        _                => false,
    }).unwrap_or(false);
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

// ── Radar entity bridge ───────────────────────────────────────────────

/// Bridges `ClientSimState` entity snapshots into ECS entities with
/// `OnRadar` / `RadarAppearance` for the `GenericRadar` widget.
///
/// Weapons radar shows ships (other vessels) and torpedoes (missiles).
fn bridge_client_sim_to_weapons_radar(
    mut commands: Commands,
    sim: Res<ClientSimState>,
    ship_view: Res<ShipView>,
    mut radar: ResMut<WeaponsRadarEntities>,
) {
    // ── Radar center (player ship) ────────────────────────────────────
    let ship_appearance = RadarAppearance {
        icon: RadarIcon::Ship,
        world_size: 6.0,
        color: Color::srgb(0.95, 0.95, 1.0),
        region_colour: None,
        region_shape: None,
    };
    let ship_yaw = ship_view.ship_yaw;
    let ship_t = Transform::from_xyz(ship_view.ship_x, 0.0, ship_view.ship_z)
        .with_rotation(Quat::from_rotation_y(ship_yaw));
    match radar.center {
        Some(e) => {
            commands.entity(e).insert((
                RadarCenter {
                    world_x: ship_view.ship_x,
                    world_z: ship_view.ship_z,
                    yaw: ship_yaw,
                },
                OnRadar(RadarLayer::Ship),
                ship_appearance,
                ship_t,
            ));
        }
        None => {
            let e = commands
                .spawn((
                    RadarCenter {
                        world_x: ship_view.ship_x,
                        world_z: ship_view.ship_z,
                        yaw: ship_yaw,
                    },
                    OnRadar(RadarLayer::Ship),
                    ship_appearance,
                    ship_t,
                    GlobalTransform::default(),
                ))
                .id();
            radar.center = Some(e);
        }
    }

    // ── Entity blips ──────────────────────────────────────────────────
    let mut seen = std::collections::HashSet::new();

    for snapshot in &sim.world.entities {
        let uuid = &snapshot.uuid;
        if !seen.insert(uuid.clone()) {
            continue;
        }

        // Weapons radar shows ships, torpedoes, stations, and regions.
        let layer = match tags_to_radar_layer(&snapshot.tags) {
            Some(
                l @ (RadarLayer::Ship
                | RadarLayer::Missile
                | RadarLayer::Station
                | RadarLayer::Region),
            ) => l,
            _ => continue,
        };

        let entity_yaw = snapshot.yaw.unwrap_or(0.0);
        let colour = snapshot.colour.map(|c| Color::srgb(c[0], c[1], c[2]));

        if layer == RadarLayer::Region {
            // ── Region entity: render as shape ────────────────────────────────
            let region_colour = colour.unwrap_or(default_layer_colour(layer));
            let region_shape = region_shape_from_snapshot(snapshot);
            let world_size = snapshot
                .radar_world_size
                .or(Some(snapshot.radius_or_zero()))
                .filter(|s| *s > 0.0)
                .unwrap_or(4.0);
            let appearance = RadarAppearance {
                icon: RadarIcon::Star,
                world_size,
                color: Color::WHITE,
                region_colour: Some(region_colour),
                region_shape,
            };
            let t = Transform::from_xyz(snapshot.x(), 0.0, snapshot.z())
                .with_rotation(Quat::from_rotation_y(entity_yaw));
            if let Some(existing) = radar.blips.get(uuid) {
                commands.entity(*existing).insert((OnRadar(layer), appearance, t));
            } else {
                let blip = commands
                    .spawn((OnRadar(layer), appearance, t, GlobalTransform::default()))
                    .id();
                radar.blips.insert(uuid.clone(), blip);
            }
        } else {
            // ── Point entity: render as icon ──────────────────────────────────
            let icon = layer_to_icon(layer);
            let default_color = match layer {
                RadarLayer::Ship => Color::srgb(1.0, 0.4, 0.4),
                RadarLayer::Missile => Color::srgb(1.0, 0.4, 0.2),
                RadarLayer::Station => Color::srgb(0.3, 0.8, 0.6),
                _ => unreachable!("filtered above"),
            };
            let world_size = snapshot
                .radar_world_size
                .or(Some(snapshot.radius_or_zero()))
                .filter(|s| *s > 0.0)
                .unwrap_or(4.0);
            let appearance = RadarAppearance {
                icon,
                world_size,
                color: colour.unwrap_or(default_color),
                region_colour: None,
                region_shape: None,
            };
            let t = Transform::from_xyz(snapshot.x(), 0.0, snapshot.z())
                .with_rotation(Quat::from_rotation_y(entity_yaw));
            if let Some(existing) = radar.blips.get(uuid) {
                commands.entity(*existing).insert((OnRadar(layer), appearance, t));
            } else {
                let blip = commands
                    .spawn((OnRadar(layer), appearance, t, GlobalTransform::default()))
                    .id();
                radar.blips.insert(uuid.clone(), blip);
            }
        }
    }

    // Despawn blips no longer in sim state.
    radar.blips.retain(|uuid, entity| {
        if seen.contains(uuid) {
            true
        } else {
            commands.entity(*entity).despawn();
            false
        }
    });
}

// ── Tests ─────────────────────────────────────────────────────────────

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

    // ── radar_local_pixel coordinate conversion ───────────────────────

    #[test]
    fn radar_local_pixel_scale_factor_one_is_identity_minus_top_left() {
        // Desktop: logical == physical. Node at (100,200), size 400x400, tap
        // at logical (300, 400) → local physical (200, 200) (i.e. the centre).
        let out = radar_local_pixel(
            Vec2::new(300.0, 400.0),
            Vec2::new(100.0, 200.0),
            Vec2::new(400.0, 400.0),
            1.0,
        );
        assert_eq!(out, Vec2::new(200.0, 200.0));
    }

    #[test]
    fn radar_local_pixel_scale_factor_two_centre_of_node() {
        // High-DPI phone: node is 400x400 physical px sitting at physical
        // top-left (200, 200). In logical pixels that's top-left (100, 100)
        // and size 200x200, so the logical centre tap is (200, 200).
        // Expected local physical: (200, 200) — the centre of the node.
        let out = radar_local_pixel(
            Vec2::new(200.0, 200.0),
            Vec2::new(200.0, 200.0),
            Vec2::new(400.0, 400.0),
            2.0,
        );
        assert_eq!(out, Vec2::new(200.0, 200.0));
    }

    #[test]
    fn radar_local_pixel_scale_factor_non_integer() {
        // scale 1.5: node physical top-left (150, 75), size 300x300.
        // Logical top-left = (100, 50), logical size = 200x200.
        // Logical centre tap = (200, 150) → physical tap (300, 225) →
        // local = (150, 150), the physical centre of the node.
        let out = radar_local_pixel(
            Vec2::new(200.0, 150.0),
            Vec2::new(150.0, 75.0),
            Vec2::new(300.0, 300.0),
            1.5,
        );
        assert_eq!(out, Vec2::new(150.0, 150.0));
    }

    #[test]
    fn radar_local_pixel_scale_factor_two_top_left_corner() {
        // Tap at the node's logical top-left corner should map to local (0,0).
        // Node physical top-left (200, 200), scale 2.0 → logical top-left (100, 100).
        let out = radar_local_pixel(
            Vec2::new(100.0, 100.0),
            Vec2::new(200.0, 200.0),
            Vec2::new(400.0, 400.0),
            2.0,
        );
        assert_eq!(out, Vec2::ZERO);
    }

    // ── weapons_panel_visible ─────────────────────────────────────────

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

    // ── fire_phaser_message builder ───────────────────────────────────

    #[test]
    fn fire_phaser_message_produces_fire_phaser() {
        use crate::messages::ClientMessage;
        let msg = fire_phaser_message();
        assert_eq!(msg, ClientMessage::FirePhaser { bank: "port".to_string() });
    }

    // ── fire_torpedo_message builder ──────────────────────────────────

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

    // ── toggle_phaser_mode_message builder ────────────────────────────

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

    // ── is_fire_button_enabled ────────────────────────────────────────

    #[test]
    fn fire_button_enabled_when_ready_and_no_cooldown() {
        use crate::client_sim::is_fire_button_enabled;
        let mut state = ClientSimState::default();
        state.fire_ready = true;
        state.on_cooldown = false;
        assert!(is_fire_button_enabled(&state));
    }

    #[test]
    fn fire_button_disabled_when_on_cooldown() {
        use crate::client_sim::is_fire_button_enabled;
        let mut state = ClientSimState::default();
        state.fire_ready = true;
        state.on_cooldown = true;
        assert!(!is_fire_button_enabled(&state));
    }

    #[test]
    fn fire_button_disabled_when_not_ready() {
        use crate::client_sim::is_fire_button_enabled;
        let mut state = ClientSimState::default();
        state.fire_ready = false;
        state.on_cooldown = false;
        assert!(!is_fire_button_enabled(&state));
    }

    // ── phaser_mode_label ─────────────────────────────────────────────

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

    // ── SelectedTube default ──────────────────────────────────────────

    #[test]
    fn selected_tube_defaults_to_none() {
        let s = SelectedTube::default();
        assert_eq!(s.0, None);
    }

    // ── Radar filter: ships + torpedoes only ──────────────────────────

    #[test]
    fn weapons_radar_filter_includes_ships() {
        use crate::gui::is_on_radar;
        let filter = RadarFilter(std::collections::HashSet::from([
            RadarLayer::Ship,
            RadarLayer::Missile,
        ]));
        assert!(is_on_radar(&filter, RadarLayer::Ship));
    }

    #[test]
    fn weapons_radar_filter_includes_missiles() {
        use crate::gui::is_on_radar;
        let filter = RadarFilter(std::collections::HashSet::from([
            RadarLayer::Ship,
            RadarLayer::Missile,
        ]));
        assert!(is_on_radar(&filter, RadarLayer::Missile));
    }

    #[test]
    fn weapons_radar_filter_excludes_asteroids() {
        use crate::gui::is_on_radar;
        let filter = RadarFilter(std::collections::HashSet::from([
            RadarLayer::Ship,
            RadarLayer::Missile,
        ]));
        assert!(!is_on_radar(&filter, RadarLayer::Asteroid));
    }

    // ── StateVisuals: five widget states render distinctly ────────────

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
}
