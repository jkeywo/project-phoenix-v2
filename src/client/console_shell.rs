//! ConsoleShell — reusable panel widget with embedded orientation-aware tab bar.
//!
//! [`ConsoleShell`] owns a background panel, an embedded tab bar (horizontal
//! in portrait, vertical in landscape), and two content slots (primary and
//! secondary).  Later slices migrate existing per-console panels to use this
//! framework.
//!
//! The embedded tab bar replaces the flat `TabBarRoot`-based tab bar in
//! `app.rs` once migration is complete (Issue #349 removes the old bar).

use bevy::prelude::*;

use crate::client_lobby::{ActiveConsole, LobbyState, LobbyView, LocalPlayerToken};
use crate::client::elements::{spawn_help_overlay_root, HelpButton, HelpPanel};
use crate::gui::{StateVisuals, Visual, WidgetState};
use crate::messages::{Console, GamePhase};
use crate::phone_border::framing::PhoneAssets;

// ── Return type for ConsoleShell::spawn ────────────────────────────

/// Entities created by [`ConsoleShell::spawn`].
///
/// Callers hold onto these IDs so they can later reparent or query the
/// primary / secondary content containers, or show / hide the tab bar.
pub struct ConsoleShellEntities {
    pub root: Entity,
    pub tab_bar: Entity,
    pub primary: Entity,
    pub secondary: Entity,
}

// ── Marker components ──────────────────────────────────────────────

/// Marks the embedded tab bar inside a [`ConsoleShell`].
///
/// The [`rebuild_embedded_tab_bars`] system finds these entities and
/// repopulates [`EmbeddedTabButton`] children each frame when the lobby
/// state or active console changes.
#[derive(Component)]
pub struct EmbeddedTabBar {
    /// `true` when the device is in landscape mode (tab bar is vertical).
    pub is_vertical: bool,
}

/// Marks a single tab button inside an [`EmbeddedTabBar`]; carries the
/// [`Console`] it selects when pressed.
#[derive(Component)]
pub struct EmbeddedTabButton(pub Console);

// ── Widget ─────────────────────────────────────────────────────────

/// Reusable panel widget with an embedded orientation-aware tab bar and
/// two content slots (primary and secondary).
pub struct ConsoleShell;

impl ConsoleShell {
    /// Spawn a new `ConsoleShell` tree.
    ///
    /// `panel_bg` — background image texture (e.g. `helm_panel_bg` from
    /// [`PhoneAssets`]).
    ///
    /// `is_landscape` — layout mode: `true` puts the tab bar on the left
    /// (vertical), `false` puts it on top (horizontal).
    ///
    /// `help_panel` — which [`HelpPanel`] the top-left "?" button opens.
    /// The button is spawned inside the shell root; its overlay is
    /// spawned at window root so it can render above the tab bar and
    /// bezel as a full-screen modal (see [`spawn_help_overlay_root`]).
    ///
    /// `fill_primary` / `fill_secondary` — called after the content
    /// containers exist.  Each receives `(&mut Commands, Entity)` where
    /// the `Entity` is the primary or secondary container, so the caller
    /// can spawn children into the slot.
    ///
    /// Returns the [`ConsoleShellEntities`] containing the root, tab bar,
    /// primary, and secondary entity IDs.
    pub fn spawn(
        commands: &mut Commands,
        panel_bg: Handle<Image>,
        is_landscape: bool,
        help_panel: HelpPanel,
        fill_primary: impl FnOnce(&mut Commands, Entity),
        fill_secondary: impl FnOnce(&mut Commands, Entity),
        _phone_assets: &PhoneAssets,
    ) -> ConsoleShellEntities {
        // Layout constants depend on orientation.
        let (root_flex, tab_w, tab_h, tab_dir) = if is_landscape {
            (
                FlexDirection::Row,
                Val::Px(116.0),
                Val::Percent(100.0),
                FlexDirection::Column,
            )
        } else {
            (
                FlexDirection::Column,
                Val::Percent(100.0),
                Val::Px(36.0),
                FlexDirection::Row,
            )
        };

        // Scaffold IDs so the closure can capture them.
        let mut tab_bar_id = Entity::PLACEHOLDER;
        let mut primary_id = Entity::PLACEHOLDER;
        let mut secondary_id = Entity::PLACEHOLDER;

        // ── Root ────────────────────────────────────────────────────
        // Fills its parent. Before `reparent_panels_into_bezel` runs, the
        // parent is the window root; afterwards it is `BorderContentArea`,
        // which itself is inset by the bezel's corner/edge thickness — so
        // we want zero offsets here, not a hard-coded inset (which would
        // double up on the bezel's safe zone and push the tab strip
        // outside it on top of the border art).
        let root_id = commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    right: Val::Px(0.0),
                    top: Val::Px(0.0),
                    bottom: Val::Px(0.0),
                    flex_direction: root_flex,
                    ..default()
                },
                ImageNode::new(panel_bg),
                ZIndex(1),
            ))
            .with_children(|bezel|
                let inset_id = bezel
                    .spawn(
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(44.0),
                            right: Val::Px(44.0),
                            top: Val::Px(44.0),
                            bottom: Val::Px(44.0),
                            flex_direction: root_flex,
                            ..default()
                    },
                )    
            )
            .with_children(|root| {
                // ── Tab bar ─────────────────────────────────────────
                // Leading padding leaves space for the absolutely-positioned
                // help button in the top-left corner of the shell root.
                let tab_pad_lead = Val::Px(36.0);
                tab_bar_id = root
                    .spawn((
                        EmbeddedTabBar {
                            is_vertical: is_landscape,
                        },
                        Node {
                            width: tab_w,
                            height: tab_h,
                            flex_direction: tab_dir,
                            column_gap: Val::Px(2.0),
                            padding: if is_landscape {
                                UiRect {
                                    left: Val::Px(2.0),
                                    right: Val::Px(2.0),
                                    top: tab_pad_lead,
                                    bottom: Val::Px(2.0),
                                }
                            } else {
                                UiRect {
                                    left: tab_pad_lead,
                                    right: Val::Px(2.0),
                                    top: Val::Px(2.0),
                                    bottom: Val::Px(2.0),
                                }
                            },
                            align_items: AlignItems::Center,
                            overflow: Overflow::clip(),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.05, 0.05, 0.15, 0.92)),
                        Visibility::Hidden,
                    ))
                    .id();

                // ── Primary content ─────────────────────────────────
                primary_id = root
                    .spawn((Node {
                        flex_grow: 2.0,
                        width: if is_landscape {
                            Val::Percent(100.0)
                        } else {
                            Val::Auto
                        },
                        height: if is_landscape {
                            Val::Auto
                        } else {
                            Val::Percent(100.0)
                        },
                        overflow: Overflow::clip(),
                        ..default()
                    },))
                    .id();

                // ── Secondary content ───────────────────────────────
                secondary_id = root
                    .spawn((Node {
                        flex_shrink: 0.0,
                        flex_grow: 1.0,
                        width: if is_landscape {
                            Val::Auto
                        } else {
                            Val::Percent(100.0)
                        },
                        height: if is_landscape {
                            Val::Percent(100.0)
                        } else {
                            Val::Auto
                        },
                        overflow: Overflow::clip(),
                        padding: UiRect::all(Val::Px(12.0)),
                        ..default()
                    },))
                    .id();
            })
            .id();

        // ── Help "?" button (top-left of shell root) ────────────────
        // Absolutely positioned so it sits above the tab bar without
        // disrupting the flex layout. Spawned as a child of the shell
        // root so it lives inside BorderContentArea after reparenting.
        commands.entity(root_id).with_children(|root| {
            root.spawn((
                HelpButton(help_panel),
                Button,
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(4.0),
                    left: Val::Px(4.0),
                    width: Val::Px(28.0),
                    height: Val::Px(28.0),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
                BackgroundColor(Color::srgba(0.2, 0.2, 0.3, 0.7)),
                ZIndex(50),
            ))
            .with_children(|btn| {
                btn.spawn((
                    Text::new("?"),
                    TextFont {
                        font_size: 18.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.7, 0.8, 1.0)),
                ));
            });
        });

        // ── Help overlay (full-screen modal at window root) ─────────
        // Lives at top level so it covers the bezel + tab bar when
        // visible. Matched to its button by HelpPanel discriminant via
        // handle_help_button_press.
        spawn_help_overlay_root(commands, help_panel);

        // Invoke fill closures so callers can populate the slots.
        fill_primary(commands, primary_id);
        fill_secondary(commands, secondary_id);

        ConsoleShellEntities {
            root: root_id,
            tab_bar: tab_bar_id,
            primary: primary_id,
            secondary: secondary_id,
        }
    }
}

// ── Plugin ─────────────────────────────────────────────────────────

pub struct ConsoleShellPlugin;

impl Plugin for ConsoleShellPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (rebuild_embedded_tab_bars, handle_embedded_tab_press),
        );
    }
}

// ── Systems ────────────────────────────────────────────────────────

/// Helper to build a [`StateVisuals`] using the `btn_small_*` image assets.
fn tab_button_visuals(phone_assets: &PhoneAssets) -> StateVisuals {
    StateVisuals {
        idle: Visual {
            image: Some(phone_assets.btn_small_idle.clone()),
            color: Color::NONE,
        },
        hover: Visual {
            image: Some(phone_assets.btn_small_hover.clone()),
            color: Color::NONE,
        },
        active: Visual {
            image: Some(phone_assets.btn_small_active.clone()),
            color: Color::NONE,
        },
        press: Visual {
            image: Some(phone_assets.btn_small_press.clone()),
            color: Color::NONE,
        },
        disabled: Visual {
            image: None,
            color: Color::srgba(0.1, 0.1, 0.2, 0.5),
        },
    }
}

/// Rebuilds tab-button children on every [`EmbeddedTabBar`] when the lobby
/// state or active console changes.
///
/// Rules:
/// - Single-console player: tab bar hidden, no buttons spawned.
/// - 2–4 consoles: buttons show `console.display_name()`.
/// - 5+ consoles: buttons show `console.initial()`.
/// - Active tab gets `WidgetState { active: true }`; inactive tabs don't.
fn rebuild_embedded_tab_bars(
    mut commands: Commands,
    lobby: Res<LobbyState>,
    token: Res<LocalPlayerToken>,
    active: Res<ActiveConsole>,
    phone_assets: Res<PhoneAssets>,
    mut tab_bars: Query<(Entity, &EmbeddedTabBar, &mut Visibility)>,
) {
    if !lobby.is_changed() && !token.is_changed() && !active.is_changed() {
        return;
    }

    let view = LobbyView::new(&lobby, &token.0);
    let my_consoles = view.my_consoles();
    let in_game = lobby.phase == GamePhase::InProgress;
    let show_tabs = in_game && my_consoles.len() >= 2;

    for (tab_bar_entity, embedded, mut vis) in tab_bars.iter_mut() {
        // Landscape (vertical tab strip) has room for full names regardless of
        // count; only the portrait (horizontal) bar collapses to initials when
        // there are too many tabs to fit.
        let use_initials = !embedded.is_vertical && my_consoles.len() >= 5;
        // Despawn old tab-button children.
        commands
            .entity(tab_bar_entity)
            .despawn_related::<Children>();

        if !show_tabs {
            *vis = Visibility::Hidden;
            continue;
        }

        *vis = Visibility::Visible;

        // Build tab buttons.
        let visuals = tab_button_visuals(&phone_assets);
        commands.entity(tab_bar_entity).with_children(|parent| {
            for console in my_consoles {
                let is_active = active.0.as_ref() == Some(console);
                let label = if use_initials {
                    console.initial()
                } else {
                    console.display_name()
                };

                let default_image = if is_active {
                    phone_assets.btn_small_active.clone()
                } else {
                    phone_assets.btn_small_idle.clone()
                };

                let button_node = if embedded.is_vertical {
                    Node {
                        width: Val::Percent(100.0),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        padding: if use_initials {
                            UiRect::axes(Val::Px(6.0), Val::Px(6.0))
                        } else {
                            UiRect::axes(Val::Px(14.0), Val::Px(6.0))
                        },
                        ..default()
                    }
                } else {
                    Node {
                        flex_grow: 1.0,
                        flex_basis: Val::Px(0.0),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        padding: if use_initials {
                            UiRect::axes(Val::Px(6.0), Val::Px(6.0))
                        } else {
                            UiRect::axes(Val::Px(14.0), Val::Px(6.0))
                        },
                        ..default()
                    }
                };

                parent
                    .spawn((
                        EmbeddedTabButton(console.clone()),
                        Button,
                        ImageNode::new(default_image),
                        visuals.clone(),
                        button_node,
                        BackgroundColor(Color::NONE),
                        WidgetState { active: is_active },
                    ))
                    .with_children(|inner| {
                        inner.spawn((
                            Text::new(label),
                            TextFont {
                                font_size: 14.0,
                                ..default()
                            },
                            TextColor(if is_active {
                                Color::srgb(0.9, 0.9, 1.0)
                            } else {
                                Color::srgb(0.6, 0.6, 0.8)
                            }),
                        ));
                    });
            }
        });
    }
}

/// Handles tab button presses by updating [`ActiveConsole`].
///
/// Mirrors the logic of the existing `handle_tab_button_press` in `app.rs`
/// but operates on [`EmbeddedTabButton`] components instead of `TabButton`.
fn handle_embedded_tab_press(
    mut interactions: Query<
        (&Interaction, &EmbeddedTabButton),
        (Changed<Interaction>, With<Button>),
    >,
    mut active: ResMut<ActiveConsole>,
) {
    for (interaction, btn) in interactions.iter_mut() {
        if *interaction == Interaction::Pressed {
            active.0 = Some(btn.0.clone());
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::Player;

    /// Build a minimal [`PhoneAssets`] with weak default handles (no real
    /// asset pipeline needed for unit tests).
    fn test_phone_assets() -> PhoneAssets {
        let d: Handle<Image> = Handle::default();
        PhoneAssets {
            compass_ring: d.clone(),
            needle: d.clone(),
            tab_corner: d.clone(),
            font_display: Handle::default(),
            font_mono: Handle::default(),
            btn_normal_idle: d.clone(),
            btn_normal_hover: d.clone(),
            btn_normal_active: d.clone(),
            btn_normal_press: d.clone(),
            btn_small_idle: d.clone(),
            btn_small_hover: d.clone(),
            btn_small_active: d.clone(),
            btn_small_press: d.clone(),
            helm_panel_bg: d.clone(),
            impulse_ready: d.clone(),
            impulse_idle: d.clone(),
            impulse_hover: d.clone(),
            impulse_active: d.clone(),
            impulse_press: d.clone(),
            joystick_knob_idle: d.clone(),
            joystick_knob_hover: d.clone(),
            joystick_knob_active: d.clone(),
            joystick_knob_press: d.clone(),
            joystick_pad_idle: d.clone(),
            joystick_pad_active: d.clone(),
            radar_bg: d.clone(),
            radar_surround: d.clone(),
            captain_panel_bg: d.clone(),
            red_alert_idle: d.clone(),
            red_alert_hover: d.clone(),
            red_alert_active: d.clone(),
            red_alert_press: d.clone(),
            red_alert_armed: d.clone(),
            inset_card: d.clone(),
            radar_icons: crate::phone_border::framing::RadarIconHandles {
                ship: d.clone(),
                asteroid: d.clone(),
                station: d.clone(),
                planet: d.clone(),
                star: d.clone(),
                torpedo: d.clone(),
            },
        }
    }

    /// Create a minimal `App` with `ConsoleShellPlugin` systems registered
    /// and the shared resources initialised.
    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.init_resource::<LobbyState>();
        app.init_resource::<LocalPlayerToken>();
        app.init_resource::<ActiveConsole>();
        app.insert_resource(test_phone_assets());
        app.add_systems(
            Update,
            (rebuild_embedded_tab_bars, handle_embedded_tab_press),
        );
        app
    }

    /// Helper: spawn an [`EmbeddedTabBar`] entity for testing.
    fn spawn_tab_bar(app: &mut App) -> Entity {
        app.world_mut()
            .spawn((EmbeddedTabBar { is_vertical: false }, Visibility::Visible))
            .id()
    }

    /// Helper: set up the lobby so the local player holds the given consoles.
    fn set_player_consoles(app: &mut App, token: &str, consoles: Vec<Console>) {
        app.world_mut().resource_mut::<LocalPlayerToken>().0 = token.to_string();

        let mut lobby = app.world_mut().resource_mut::<LobbyState>();
        lobby.phase = GamePhase::InProgress;
        lobby.players = vec![Player {
            token: token.to_string(),
            name: "Test".into(),
            consoles,
            connected: true,
        }];
    }

    /// Helper: count how many [`EmbeddedTabButton`] entities exist in the world.
    fn tab_button_count(app: &mut App) -> usize {
        app.world_mut()
            .query::<&EmbeddedTabButton>()
            .iter(app.world())
            .count()
    }

    #[test]
    fn single_console_hides_tab_bar() {
        let mut app = test_app();

        set_player_consoles(&mut app, "me", vec![Console::Helm]);
        let tab_bar = spawn_tab_bar(&mut app);

        // First update: build (or not) the tab buttons.
        app.update();

        // Tab bar should be hidden.
        let vis = app.world().get::<Visibility>(tab_bar).unwrap();
        assert_eq!(
            *vis,
            Visibility::Hidden,
            "single-console bar must be hidden"
        );

        // No tab buttons should exist.
        assert_eq!(
            tab_button_count(&mut app),
            0,
            "single-console player must have zero tab buttons"
        );
    }

    #[test]
    fn two_console_player_shows_two_tab_buttons() {
        let mut app = test_app();

        set_player_consoles(&mut app, "me", vec![Console::CaptainChair, Console::Helm]);
        let tab_bar = spawn_tab_bar(&mut app);

        app.update();

        // Tab bar should be visible.
        let vis = app.world().get::<Visibility>(tab_bar).unwrap();
        assert_eq!(
            *vis,
            Visibility::Visible,
            "multi-console bar must be visible"
        );

        // Two tab buttons should exist.
        assert_eq!(tab_button_count(&mut app), 2);
    }

    #[test]
    fn five_consoles_uses_initials() {
        let mut app = test_app();

        set_player_consoles(
            &mut app,
            "me",
            vec![
                Console::CaptainChair,
                Console::Helm,
                Console::Tactical,
                Console::Repair,
                Console::Sensors,
            ],
        );
        let _tab_bar = spawn_tab_bar(&mut app);

        app.update();

        // Five tab buttons should exist.
        assert_eq!(tab_button_count(&mut app), 5);

        // Verify each button has the correct console variant (insertion order).
        let seen: Vec<Console> = app
            .world_mut()
            .query::<&EmbeddedTabButton>()
            .iter(app.world())
            .map(|btn| btn.0.clone())
            .collect();

        assert_eq!(seen.len(), 5, "expected 5 tab buttons");

        // Consoles appear in the same order as set_player_consoles.
        assert_eq!(seen[0], Console::CaptainChair);
        assert_eq!(seen[1], Console::Helm);
        assert_eq!(seen[2], Console::Tactical);
        assert_eq!(seen[3], Console::Repair);
        assert_eq!(seen[4], Console::Sensors);
    }

    #[test]
    fn pressing_tab_updates_active_console() {
        let mut app = test_app();

        set_player_consoles(&mut app, "me", vec![Console::CaptainChair, Console::Helm]);
        let _tab_bar = spawn_tab_bar(&mut app);

        // First update: build tabs.
        app.update();

        // ActiveConsole should start as None (default).
        assert_eq!(
            app.world().resource::<ActiveConsole>().0,
            None,
            "initial active console must be None"
        );

        // Locate the Helm tab button entity.
        let helm_entity = {
            let mut q = app.world_mut().query::<(Entity, &EmbeddedTabButton)>();
            q.iter(app.world())
                .find(|(_, btn)| btn.0 == Console::Helm)
                .map(|(e, _)| e)
                .expect("Helm tab button must exist")
        };

        // Simulate a press.
        let mut interaction = app
            .world_mut()
            .get_mut::<Interaction>(helm_entity)
            .expect("tab button must have Interaction component");
        *interaction = Interaction::Pressed;

        // Second update: system should react to the press.
        app.update();

        assert_eq!(
            app.world().resource::<ActiveConsole>().0,
            Some(Console::Helm),
            "pressing Helm tab must set ActiveConsole to Helm"
        );
    }
}
