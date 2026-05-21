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
use std::collections::HashMap;

use crate::client_app::{
    WeaponsPanel, OutboundClientMessage, RepairIconLabel,
    HideableElement, ComplexityPopupRoot, ComplexityPresetButton, ComplexityPopupConfirm,
    ComplexityDropdownRoot,
};
use crate::client_lobby::{ActiveConsole, LobbyState, LobbyView, LocalPlayerToken};
use crate::client_sim::{
    fire_phaser_message, toggle_phaser_mode_message, fire_torpedo_message,
    is_fire_button_enabled, phaser_mode_label, ClientSimState,
};
use crate::gui::{
    layer_to_icon, spawn_gui_button, tags_to_radar_layer, ButtonPressed, ButtonSize, GenericRadar,
    OnRadar, OrientationMode, RadarAppearance, RadarCenter, RadarFilter, RadarIcon, RadarLayer,
    StateVisuals, RadioButtonConfig, RadioGroup, RadioSelected,
    Disabled,
};
use crate::messages::{Console, GamePhase, TorpedoTube};
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
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
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

/// Plugin that owns all Tactical console UI and systems.
pub struct WeaponsPanelPlugin;

impl Plugin for WeaponsPanelPlugin {
    fn build(&self, app: &mut App) {
        app
            .init_resource::<SelectedTube>()
            .init_resource::<WeaponsRadarEntities>()
            .add_systems(Startup, setup_weapons_ui)
            .add_systems(Update, (
                toggle_weapons_panel_visibility,
                add_tube_button_labels,
                refresh_weapons_panel,
                sync_fire_phaser_disabled,
                refresh_torpedo_ui,
                bridge_client_sim_to_weapons_radar,
            ));
    }
}

// ── Setup ─────────────────────────────────────────────────────────────

fn setup_weapons_ui(mut commands: Commands) {
    // ── Root panel ────────────────────────────────────────────────────
    let panel = commands
        .spawn((
            WeaponsPanel,
            Node {
                position_type: PositionType::Absolute,
                left:   Val::Px(0.0),
                top:    Val::Px(0.0),
                right:  Val::Px(0.0),
                bottom: Val::Px(0.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                row_gap: Val::Px(16.0),
                padding: UiRect::all(Val::Px(24.0)),
                ..default()
            },
            Visibility::Hidden,
        ))
        .id();

    // ── Tactical radar (GenericRadar, WorldFixed, Ships + Torpedoes) ──
    let radar_filter = RadarFilter(std::collections::HashSet::from([
        RadarLayer::Ship,
        RadarLayer::Missile,
    ]));
    let radar = GenericRadar::spawn(
        &mut commands,
        crate::client_sim::WEAPONS_RADAR_RANGE,
        OrientationMode::WorldFixed,
        radar_filter,
        None,
        None,
    );
    commands.entity(radar).insert(Node {
        width:  Val::Px(240.0),
        height: Val::Px(240.0),
        border: UiRect::all(Val::Px(1.0)),
        aspect_ratio: Some(1.0),
        position_type: PositionType::Relative,
        ..default()
    });
    commands.entity(panel).add_child(radar);

    // ── Title row ─────────────────────────────────────────────────────
    let title_row = commands
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        })
        .id();
    commands.entity(title_row).with_children(|row| {
        row.spawn((
            Text::new("Weapons Console"),
            TextFont { font_size: 24.0, ..default() },
            TextColor(Color::srgb(1.0, 0.5, 0.2)),
        ));
        crate::client_elements::spawn_help_button(row, crate::client_elements::HelpPanel::Tactical, 16.0);
    });
    commands.entity(panel).add_child(title_row);
    crate::client_elements::spawn_help_overlay_root(&mut commands, crate::client_elements::HelpPanel::Tactical);

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
    commands.entity(panel).add_child(torpedo_container);

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

    // ── Torpedo tube RadioGroup ────────────────────────────────────────
    let tube_btn_configs: Vec<RadioButtonConfig> = (0..3)
        .map(|_| RadioButtonConfig {
            size: ButtonSize::Rect { width: 80.0, height: 36.0 },
            click_sound: None,
        })
        .collect();

    let radio_group = RadioGroup::spawn(
        &mut commands,
        tube_btn_configs,
        tube_visuals(),
        None,
    );
    commands.entity(radio_group)
        .insert(TubeRadioGroup)
        .observe(on_tube_selected);
    commands.entity(torpedo_container).add_child(radio_group);

    // Add text labels to each radio member button.
    // RadioGroup::spawn creates children in config order; retrieve them.
    // We know there are 3 children and label them FWD PORT, FWD STBD, AFT.
    // The labels are added as grandchildren of radio_group via with_children
    // deferred at spawn. We do this by querying children right after—but
    // since the world hasn't applied the child commands yet, we instead
    // use a one-shot post-spawn system to finish wiring.
    //
    // As a simpler approach: spawn the labels as an independent row below
    // the RadioGroup and use the TubeStatusLabel instead of labelling each
    // button, and insert text directly into each button child via the
    // deferred approach in add_tube_button_labels system.
    commands.insert_resource(TubeButtonLabelsPending);

    // Tube status labels row (one per tube, left to right order matches buttons)
    let status_row = commands
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(8.0),
            ..default()
        })
        .id();
    for tube in [
        TorpedoTube::ForePort,
        TorpedoTube::ForeStarboard,
        TorpedoTube::Aft,
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

    // ── Fire Torpedo button ────────────────────────────────────────────
    let fire_torpedo_btn = spawn_gui_button(
        &mut commands,
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
        &mut commands,
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
    commands.entity(panel).add_child(mode_btn);

    // ── Fire Phasers button ───────────────────────────────────────────
    let fire_phaser_btn = spawn_gui_button(
        &mut commands,
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
    commands.entity(panel).add_child(fire_phaser_btn);

    // ── Repair icon label ─────────────────────────────────────────────
    let repair_label = commands
        .spawn((
            RepairIconLabel,
            Text::new(""),
            TextFont { font_size: 14.0, ..default() },
            TextColor(Color::srgb(0.8, 0.5, 0.2)),
        ))
        .id();
    commands.entity(panel).add_child(repair_label);

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
    commands.entity(panel).add_child(dropdown);

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
    commands.entity(panel).add_child(popup);
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
        TorpedoTube::ForePort,
        TorpedoTube::ForeStarboard,
        TorpedoTube::Aft,
    ];
    if let Ok(children) = children_q.get(group) {
        for (idx, child) in children.iter().enumerate() {
            if child == member {
                if let Some(&tube) = tubes.get(idx) {
                    selected.0 = Some(tube);
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
    let Some(tube) = selected.0 else { return };
    let loaded = match tube {
        TorpedoTube::ForePort       => sim.fore_port_loaded,
        TorpedoTube::ForeStarboard  => sim.fore_starboard_loaded,
        TorpedoTube::Aft            => sim.aft_loaded,
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
        let (loaded, reload_secs) = match label.0 {
            TorpedoTube::ForePort       => (sim.fore_port_loaded,      sim.fore_port_reload_secs),
            TorpedoTube::ForeStarboard  => (sim.fore_starboard_loaded, sim.fore_starboard_reload_secs),
            TorpedoTube::Aft            => (sim.aft_loaded,            sim.aft_reload_secs),
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
    let tube_ready = selected.0.map(|t| match t {
        TorpedoTube::ForePort       => sim.fore_port_loaded,
        TorpedoTube::ForeStarboard  => sim.fore_starboard_loaded,
        TorpedoTube::Aft            => sim.aft_loaded,
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
    };
    match radar.center {
        Some(e) => {
            commands.entity(e).insert((
                RadarCenter {
                    world_x: ship_view.ship_x,
                    world_z: ship_view.ship_z,
                    yaw: ship_view.ship_yaw,
                },
                OnRadar(RadarLayer::Ship),
                ship_appearance,
                Transform::from_xyz(ship_view.ship_x, 0.0, ship_view.ship_z),
            ));
        }
        None => {
            let e = commands
                .spawn((
                    RadarCenter {
                        world_x: ship_view.ship_x,
                        world_z: ship_view.ship_z,
                        yaw: ship_view.ship_yaw,
                    },
                    OnRadar(RadarLayer::Ship),
                    ship_appearance,
                    Transform::from_xyz(ship_view.ship_x, 0.0, ship_view.ship_z),
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

        // Weapons radar only shows ships and torpedoes; everything else
        // (asteroids, stations, planets, stars, regions) is filtered out.
        let layer = match tags_to_radar_layer(&snapshot.tags) {
            Some(l @ (RadarLayer::Ship | RadarLayer::Missile)) => l,
            _ => continue,
        };
        let icon = layer_to_icon(layer);
        // Tactical paints ships and torpedoes in alert-red shades rather
        // than the helm-default colours.
        let default_color = match layer {
            RadarLayer::Ship => Color::srgb(1.0, 0.4, 0.4),
            RadarLayer::Missile => Color::srgb(1.0, 0.4, 0.2),
            _ => unreachable!("filtered above"),
        };

        let colour = snapshot.colour.map(|c| Color::srgb(c[0], c[1], c[2]));
        let world_size = snapshot
            .radar_world_size
            .or(Some(snapshot.radius_or_zero()))
            .filter(|s| *s > 0.0)
            .unwrap_or(4.0);
        let appearance = RadarAppearance {
            icon,
            world_size,
            color: colour.unwrap_or(default_color),
        };

        if let Some(existing) = radar.blips.get(uuid) {
            commands.entity(*existing).insert((
                OnRadar(layer),
                appearance,
                Transform::from_xyz(snapshot.x(), 0.0, snapshot.z()),
            ));
        } else {
            let blip = commands
                .spawn((
                    OnRadar(layer),
                    appearance,
                    Transform::from_xyz(snapshot.x(), 0.0, snapshot.z()),
                    GlobalTransform::default(),
                ))
                .id();
            radar.blips.insert(uuid.clone(), blip);
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
        assert_eq!(msg, ClientMessage::FirePhaser);
    }

    // ── fire_torpedo_message builder ──────────────────────────────────

    #[test]
    fn fire_torpedo_message_fore_port_no_target() {
        use crate::messages::ClientMessage;
        let msg = fire_torpedo_message(TorpedoTube::ForePort, None);
        assert_eq!(msg, ClientMessage::FireTorpedo {
            tube: TorpedoTube::ForePort,
            target_uuid: None,
        });
    }

    #[test]
    fn fire_torpedo_message_aft_with_target() {
        use crate::messages::ClientMessage;
        let msg = fire_torpedo_message(TorpedoTube::Aft, Some("uuid-1".into()));
        assert_eq!(msg, ClientMessage::FireTorpedo {
            tube: TorpedoTube::Aft,
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
