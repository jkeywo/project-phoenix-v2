//! Client-side UI element helpers (formerly the help system).
//!
//! The hideable-element registry that once lived here (used by the complexity
//! preset system) was ported to pure JS in issue #461; see
//! gui/hideable-elements.js.
//!
//! The help system (the 9 `HelpPanel` variants, `help_sections()` static text,
//! the `HelpButton` / `HelpOverlay` components, the spawn helpers, and the
//! `handle_help_button_press` / `handle_help_overlay_dismiss` systems) was
//! ported to pure JS in issue #462; see gui/help-panel.js (static text + modal
//! machinery) and gui/console-core.js (per-console mount). Nothing Bevy-side
//! remains in this module.
//!
//! The file is intentionally left in place — its deletion (and the
//! `pub mod elements;` / `pub use elements::*;` re-exports in
//! `src/client/mod.rs`, plus the `client_elements` alias in `lib.rs`) is part
//! of the issue #463 teardown.
