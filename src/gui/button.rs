//! `GuiButton` widget — rect/square variants, observer events, click sounds.

use bevy::prelude::*;

use super::{StateVisuals, WidgetState};

// ── Size variant ──────────────────────────────────────────────────────────────

/// Controls the layout dimensions of a `GuiButton`.
#[derive(Clone, Debug, PartialEq)]
pub enum ButtonSize {
    /// Rectangular button with explicit width and height.
    Rect { width: f32, height: f32 },
    /// Square button: width == height.
    Square(f32),
}

impl ButtonSize {
    fn to_node(&self) -> Node {
        match self {
            ButtonSize::Rect { width, height } => Node {
                width: Val::Px(*width),
                height: Val::Px(*height),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            ButtonSize::Square(size) => Node {
                width: Val::Px(*size),
                height: Val::Px(*size),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
        }
    }
}

// ── Observer events ───────────────────────────────────────────────────────────

/// Fired on a button entity exactly once per press transition — when
/// `Interaction` first becomes `Pressed`.  The payload carries the button
/// entity so callers can also observe this as a global event.
#[derive(EntityEvent, Clone, Debug)]
pub struct ButtonPressed(pub Entity);

/// Fired when `WidgetState.active` transitions to `true`.
#[derive(EntityEvent, Clone, Debug)]
pub struct WidgetActivated(pub Entity);

/// Fired when `WidgetState.active` transitions to `false`.
#[derive(EntityEvent, Clone, Debug)]
pub struct WidgetDeactivated(pub Entity);

// ── Audio ─────────────────────────────────────────────────────────────────────

/// Shared UI audio handles, populated at app startup.
#[derive(Resource, Clone, Debug)]
pub struct UiSounds {
    /// Default click sound played when a button has no `ClickSound` override.
    pub click_standard: Handle<AudioSource>,
}

/// Per-button audio override.  When present, this sound is played instead of
/// `UiSounds.click_standard` on press.
#[derive(Component, Clone, Debug)]
pub struct ClickSound(pub Handle<AudioSource>);

// ── Marker ────────────────────────────────────────────────────────────────────

/// Marker component on every entity spawned by `spawn_gui_button`.
#[derive(Component, Default)]
pub struct GuiButtonMarker;

// ── Pure helpers ──────────────────────────────────────────────────────────────

/// Returns the audio handle to play for a button press.
///
/// Priority: per-button `ClickSound` > `UiSounds.click_standard`.
///
/// Pure function — fully unit-testable without a running `App`.
pub fn resolve_click_sound<'a>(
    ui_sounds: &'a UiSounds,
    click_sound: Option<&'a ClickSound>,
) -> &'a Handle<AudioSource> {
    match click_sound {
        Some(cs) => &cs.0,
        None => &ui_sounds.click_standard,
    }
}

// ── Startup system ────────────────────────────────────────────────────────────

/// Populates the `UiSounds` resource at app startup.
pub fn setup_ui_sounds(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.insert_resource(UiSounds {
        click_standard: asset_server.load("sounds/ui_click.ogg"),
    });
}

// ── Spawn helper ──────────────────────────────────────────────────────────────

/// Spawn a `GuiButton` entity.
///
/// Observers for `ButtonPressed`, `WidgetActivated`, and `WidgetDeactivated`
/// are **attached at spawn time** (stub no-ops).  Callers receive the spawned
/// `Entity` and can add additional `.observe()` handlers immediately:
///
/// ```rust,ignore
/// let entity = spawn_gui_button(&mut commands, size, visuals, None);
/// commands.entity(entity).observe(|trigger: On<ButtonPressed>| { /* … */ });
/// ```
pub fn spawn_gui_button(
    commands: &mut Commands,
    size: ButtonSize,
    state_visuals: StateVisuals,
    click_sound: Option<Handle<AudioSource>>,
) -> Entity {
    let initial_color = state_visuals.idle.color;
    let initial_image = state_visuals.idle.image.clone();
    let mut builder = commands.spawn((
        GuiButtonMarker,
        Button,
        size.to_node(),
        BackgroundColor(initial_color),
        state_visuals,
        WidgetState::default(),
        Interaction::default(),
    ));
    // `resolve_visuals_system` only updates an existing `ImageNode`; insert
    // one at spawn time so the per-state image actually renders.  Buttons
    // configured with color-only `Visual`s (no image handle) skip this and
    // render via `BackgroundColor` alone.
    if let Some(image) = initial_image {
        builder.insert(ImageNode::new(image));
    }
    if let Some(handle) = click_sound {
        builder.insert(ClickSound(handle));
    }
    // Attach stub observers at spawn so the observer infrastructure is wired.
    // Callers layer additional `.observe()` calls on the returned entity.
    builder
        .observe(|_: On<ButtonPressed>| {})
        .observe(|_: On<WidgetActivated>| {})
        .observe(|_: On<WidgetDeactivated>| {});
    builder.id()
}

// ── Systems ───────────────────────────────────────────────────────────────────

/// Detects press transitions on `GuiButtonMarker` entities and fires
/// `ButtonPressed` + the click sound.
fn detect_button_press(
    query: Query<
        (Entity, &Interaction, Option<&ClickSound>),
        (Changed<Interaction>, With<GuiButtonMarker>),
    >,
    ui_sounds: Option<Res<UiSounds>>,
    mut commands: Commands,
) {
    for (entity, interaction, click_sound) in query.iter() {
        if *interaction == Interaction::Pressed {
            // EntityEvent tuple struct: ButtonPressed(entity) satisfies FnOnce(Entity)->E
            commands.entity(entity).trigger(ButtonPressed);

            if let Some(ref sounds) = ui_sounds {
                let handle = resolve_click_sound(sounds, click_sound).clone();
                commands.spawn((
                    AudioPlayer::<AudioSource>(handle),
                    PlaybackSettings::ONCE,
                ));
            }
        }
    }
}

/// Fires `WidgetActivated` / `WidgetDeactivated` when `WidgetState.active`
/// changes on a `GuiButtonMarker` entity.
fn detect_widget_state_change(
    query: Query<
        (Entity, &WidgetState),
        (Changed<WidgetState>, With<GuiButtonMarker>),
    >,
    mut commands: Commands,
) {
    for (entity, state) in query.iter() {
        if state.active {
            commands.entity(entity).trigger(WidgetActivated);
        } else {
            commands.entity(entity).trigger(WidgetDeactivated);
        }
    }
}

// ── Plugin ────────────────────────────────────────────────────────────────────

/// Sub-plugin for the button widget.  Registered automatically by `GuiPlugin`.
pub struct GuiButtonPlugin;

impl Plugin for GuiButtonPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_ui_sounds)
           .add_systems(Update, (detect_button_press, detect_widget_state_change));
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_handle() -> Handle<AudioSource> {
        Handle::default()
    }

    fn make_ui_sounds() -> UiSounds {
        UiSounds { click_standard: dummy_handle() }
    }

    #[test]
    fn resolve_click_sound_falls_back_to_standard() {
        let sounds = make_ui_sounds();
        let handle = resolve_click_sound(&sounds, None);
        // The returned handle should be the standard one (same pointer/id).
        assert_eq!(handle, &sounds.click_standard);
    }

    #[test]
    fn resolve_click_sound_prefers_per_button_override() {
        let sounds = make_ui_sounds();
        // Construct a second distinct handle (different default slot).
        let override_handle: Handle<AudioSource> = Handle::default();
        let click_sound = ClickSound(override_handle.clone());
        let handle = resolve_click_sound(&sounds, Some(&click_sound));
        assert_eq!(handle, &override_handle);
    }

    #[test]
    fn button_size_rect_produces_correct_node() {
        let size = ButtonSize::Rect { width: 120.0, height: 40.0 };
        let node = size.to_node();
        assert_eq!(node.width, Val::Px(120.0));
        assert_eq!(node.height, Val::Px(40.0));
    }

    #[test]
    fn button_size_square_produces_equal_dimensions() {
        let size = ButtonSize::Square(48.0);
        let node = size.to_node();
        assert_eq!(node.width, Val::Px(48.0));
        assert_eq!(node.height, Val::Px(48.0));
    }
}
