//! ConsoleShell — reusable panel root for per-console panels.
//!
//! After issue #442, the embedded tab bar and bezel inset that this widget
//! used to provide are owned by the HTML/JS shell (`client.html`). The Rust
//! widget keeps only the parts the per-console panels still rely on:
//!
//! - a full-viewport root node carrying the panel background image,
//! - two flex slots (`primary` / `secondary`) for the panel to fill.
//!
//! The "?" help button + overlay this shell used to spawn were ported to pure
//! JS in issue #462 (gui/help-panel.js + gui/console-core.js), so the
//! `help_panel` parameter was dropped. This widget is itself dead code (no
//! caller spawns it any more) and is slated for deletion in the #463 teardown;
//! it is kept compiling here only so this slice stays scoped.

use bevy::prelude::*;

use crate::phone_border::framing::PhoneAssets;

// ── Return type for ConsoleShell::spawn ────────────────────────────

/// Entities created by [`ConsoleShell::spawn`].
///
/// Callers hold onto these IDs so they can later query the primary /
/// secondary content containers, or insert per-panel marker components on
/// the root.
pub struct ConsoleShellEntities {
    pub root: Entity,
    pub primary: Entity,
    pub secondary: Entity,
}

// ── Widget ─────────────────────────────────────────────────────────

/// Reusable panel root with two content slots and a help button.
///
/// The HTML bezel positions the Bevy canvas inside its safe content area,
/// so the shell root deliberately fills its parent (the window root) with
/// zero offset — the bezel inset is provided entirely by CSS in
/// `client.html`. The HTML tab bar (z-index 16) sits above the canvas and
/// drives `ActiveConsole` via `wasm_client_set_active_console`.
pub struct ConsoleShell;

impl ConsoleShell {
    /// Spawn a new `ConsoleShell` tree.
    ///
    /// `panel_bg` — background image texture (e.g. `helm_panel_bg` from
    /// [`PhoneAssets`]).
    ///
    /// `is_landscape` — layout mode for the primary/secondary stack:
    /// `true` lays them out as a row, `false` as a column.
    ///
    /// `fill_primary` / `fill_secondary` — called after the content
    /// containers exist.  Each receives `(&mut Commands, Entity)` where
    /// the `Entity` is the primary or secondary container, so the caller
    /// can spawn children into the slot.
    ///
    /// `_phone_assets` is retained for API compatibility with the
    /// pre-#442 signature; the parameter is unused inside the shell now
    /// that the embedded tab bar is gone, but every per-console panel
    /// still passes its `PhoneAssets` reference.
    pub fn spawn(
        commands: &mut Commands,
        panel_bg: Handle<Image>,
        is_landscape: bool,
        fill_primary: impl FnOnce(&mut Commands, Entity),
        fill_secondary: impl FnOnce(&mut Commands, Entity),
        _phone_assets: &PhoneAssets,
    ) -> ConsoleShellEntities {
        let root_flex = if is_landscape {
            FlexDirection::Row
        } else {
            FlexDirection::Column
        };

        // Scaffold IDs so the closure can capture them.
        let mut primary_id = Entity::PLACEHOLDER;
        let mut secondary_id = Entity::PLACEHOLDER;

        // ── Root ────────────────────────────────────────────────────
        // Fills the window — the HTML bezel handles the safe-zone inset
        // via CSS, so we deliberately use zero offsets here. Adding a
        // Rust-side inset on top would double up on the bezel padding.
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
            .with_children(|root| {
                // ── Primary content ─────────────────────────────────
                primary_id = root
                    .spawn((Node {
                        flex_grow: 2.0,
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
                        ..default()
                    },))
                    .id();

                // ── Secondary content ───────────────────────────────
                secondary_id = root
                    .spawn((Node {
                        flex_shrink: 0.0,
                        flex_grow: 1.0,
                        min_width: Val::Px(0.0),
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

        // The "?" help button + overlay this shell used to spawn here were
        // ported to pure JS in issue #462 (gui/help-panel.js + console-core.js).

        // Invoke fill closures so callers can populate the slots.
        fill_primary(commands, primary_id);
        fill_secondary(commands, secondary_id);

        ConsoleShellEntities {
            root: root_id,
            primary: primary_id,
            secondary: secondary_id,
        }
    }
}
