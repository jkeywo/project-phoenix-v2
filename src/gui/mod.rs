//! Generic GUI widget library for all phone console panels.
//!
//! `GuiPlugin` registers the visual-resolution system shared by every widget.
//! External code only needs `WidgetState` (written by game logic) and
//! `StateVisuals` (configured at spawn) to drive all five visual states.

pub use foundation::{
    resolve_visual, Disabled, GuiPlugin, StateVisuals, Visual, WidgetState,
};

mod foundation;
