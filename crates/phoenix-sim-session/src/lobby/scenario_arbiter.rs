//! First-valid-wins scenario + player-ship selection, as a pure rule set
//! (issue #755's arbiter, ported to Rust for issue #1326).
//!
//! # Why this exists in Rust at all
//!
//! On the browser host the arbiter is JavaScript — `gui/scenario-arbiter.js`,
//! driven by `server.html`, which intercepts
//! [`ClientMessage::SelectScenario`](crate::core::messages::ClientMessage::SelectScenario)
//! and
//! [`SelectPlayerShip`](crate::core::messages::ClientMessage::SelectPlayerShip)
//! on both the datachannel and the local-console path and never forwards them
//! to WASM. It can be JavaScript there because the browser host has no Bevy
//! `App` yet when the choice is made: `wasm_init` throws to unwind the JS stack
//! and therefore has to run *after* `wasm_load_world`.
//!
//! A native host has no JavaScript and, since issue #1326, no such ordering
//! escape either: its window and its lobby exist before a world is chosen, so
//! the arbitration has to happen inside the running process. This module is
//! that arbitration, and it is deliberately a **transcription** of
//! `gui/scenario-arbiter.js` rather than a second design:
//!
//! | `gui/scenario-arbiter.js` | here |
//! |---|---|
//! | `normalizeSelection` | [`ScenarioSelection::normalized`] |
//! | `findScenario` | [`find_scenario`] |
//! | `selectScenario` | [`select_scenario`] |
//! | `selectPlayerShip` | [`select_player_ship`] |
//! | `isComplete` | [`ScenarioSelection::is_complete`] |
//! | `worldPathFor` | [`world_path_for`] |
//! | `curatedShipsFor` | [`curated_ships_for`] |
//!
//! # The table that keeps them honest
//!
//! A transcription held together by a doc comment drifts, and this one had
//! already: `normalizeSelection` was mapped to `Default` — i.e. to nothing —
//! and its rule that a **falsy field is not a lock** was missing here, so an
//! empty id locked a native host and left a browser host open.
//!
//! `tests/fixtures/scenario-arbiter-parity.json` is the fix: one case table
//! read by this module's own tests AND by
//! `tests/client/scenario-arbiter-parity.test.js`, which drives the JS. Neither
//! side owns the cases, and a case only one side can satisfy is a bug in that
//! side rather than a reason to fork the table.
//!
//! The rules, restated so a reader need not open the JS: **the first request
//! that validates against the pre-load catalogue wins.** There is no voting and
//! no pre-ship captain authority — a phone and the host's own surface are equal
//! senders. A scenario id the catalogue does not list is *rejected*; a request
//! that arrives once the value is already locked is *ignored*. A ship may only
//! be chosen once a scenario is locked (hulls are scoped to their scenario,
//! issue #754 AC4) and must be one that scenario's catalogue entry offers —
//! which is already the `--manifest` curation filter, applied by
//! [`build_catalog`](crate::world::manifest::build_catalog).
//!
//! Pure and Bevy-free (AGENTS.md rule 10): the Bevy adapter that feeds it
//! inbound messages and acts on a completed selection is
//! [`crate::native_host::world_load`].

use phoenix_sim_contracts::catalogue::{ScenarioCatalog, ScenarioCatalogEntry};

/// What the host has locked so far. Both fields start `None`; each is written
/// at most once, by the first request that validates.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScenarioSelection {
    /// The locked scenario's catalogue id (`[[scenario]] id` in the manifest).
    pub scenario_id: Option<String>,
    /// The authored mission slot reserved by this selecting host.
    pub slot_id: Option<String>,
    /// The locked player hull's `template_path`.
    pub template_path: Option<String>,
}

impl ScenarioSelection {
    /// The locked scenario id, or `None` while nothing is locked.
    ///
    /// An **empty string is not a lock**. That is `normalizeSelection`'s rule in
    /// the JS — every field is coerced through `|| null`, so `''` reads as
    /// unlocked — and it was the one line of the arbiter this transcription
    /// originally left out. Without it a manifest entry with an empty `id` locks
    /// a native host into a selection the browser host would still treat as
    /// open, and the two arbiters answer the same request differently.
    pub fn scenario(&self) -> Option<&str> {
        self.scenario_id.as_deref().filter(|id| !id.is_empty())
    }

    /// The locked hull's `template_path`, under [`scenario`](Self::scenario)'s
    /// rule.
    pub fn ship(&self) -> Option<&str> {
        self.template_path.as_deref().filter(|p| !p.is_empty())
    }

    pub fn slot(&self) -> Option<&str> {
        self.slot_id.as_deref().filter(|id| !id.is_empty())
    }

    /// This selection with every falsy field coerced to `None` —
    /// `normalizeSelection` in the JS, which returns the normalised object on
    /// *every* path including `ignored` and `rejected`.
    pub fn normalized(&self) -> ScenarioSelection {
        ScenarioSelection {
            scenario_id: self.scenario().map(str::to_string),
            slot_id: self.slot().map(str::to_string),
            template_path: self.ship().map(str::to_string),
        }
    }

    /// True once both a scenario and a hull are locked — the moment the world
    /// may be loaded. `isComplete` in the JS.
    pub fn is_complete(&self) -> bool {
        self.scenario().is_some() && self.ship().is_some()
    }
}

/// What one selection request did.
///
/// Named for the JS's three outcome strings so the two implementations can be
/// read against each other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionOutcome {
    /// The request locked a new value; the returned selection replaces the held
    /// one.
    Accepted,
    /// Already locked. Deliberately not an error: a phone that pressed the same
    /// button a moment too late is not misbehaving.
    Ignored,
    /// Failed catalogue validation — an unknown scenario id, a hull this
    /// scenario does not offer, or a hull request before any scenario is locked.
    Rejected,
}

/// Find a catalogue entry by scenario id. `findScenario` in the JS.
pub fn find_scenario<'a>(
    catalog: &'a ScenarioCatalog,
    scenario_id: &str,
) -> Option<&'a ScenarioCatalogEntry> {
    catalog.scenarios.iter().find(|s| s.id == scenario_id)
}

/// Apply a scenario request to `selection`, returning the outcome and the
/// selection that should be held afterwards.
///
/// The selection is returned rather than mutated in place so a rejected request
/// cannot half-write the held state — the JS returns a fresh object for the
/// same reason.
pub fn select_scenario(
    selection: &ScenarioSelection,
    catalog: &ScenarioCatalog,
    scenario_id: &str,
) -> (SelectionOutcome, ScenarioSelection) {
    if selection.scenario().is_some() {
        return (SelectionOutcome::Ignored, selection.normalized());
    }
    if find_scenario(catalog, scenario_id).is_none() {
        return (SelectionOutcome::Rejected, selection.normalized());
    }
    (
        SelectionOutcome::Accepted,
        ScenarioSelection {
            scenario_id: Some(scenario_id.to_string()),
            slot_id: None,
            // Deliberately clears the hull, exactly as the JS does: a hull is
            // only meaningful against the scenario that offered it.
            template_path: None,
        },
    )
}

/// Reserve an authored mission slot after the scenario is locked.
pub fn select_ship_slot(
    selection: &ScenarioSelection,
    catalog: &ScenarioCatalog,
    slot_id: &str,
) -> (SelectionOutcome, ScenarioSelection) {
    let Some(scenario_id) = selection.scenario() else {
        return (SelectionOutcome::Rejected, selection.normalized());
    };
    if selection.slot().is_some() {
        return (SelectionOutcome::Ignored, selection.normalized());
    }
    let Some(entry) = find_scenario(catalog, scenario_id) else {
        return (SelectionOutcome::Rejected, selection.normalized());
    };
    if !entry.slots.iter().any(|slot| slot.id == slot_id) {
        return (SelectionOutcome::Rejected, selection.normalized());
    }
    (
        SelectionOutcome::Accepted,
        ScenarioSelection {
            scenario_id: Some(scenario_id.to_string()),
            slot_id: Some(slot_id.to_string()),
            template_path: None,
        },
    )
}

/// Apply a player-hull request. Requires a locked scenario, and the hull must
/// be one that scenario's catalogue entry offers.
pub fn select_player_ship(
    selection: &ScenarioSelection,
    catalog: &ScenarioCatalog,
    template_path: &str,
) -> (SelectionOutcome, ScenarioSelection) {
    let Some(scenario_id) = selection.scenario() else {
        return (SelectionOutcome::Rejected, selection.normalized());
    };
    if selection.ship().is_some() {
        return (SelectionOutcome::Ignored, selection.normalized());
    }
    let Some(entry) = find_scenario(catalog, scenario_id) else {
        return (SelectionOutcome::Rejected, selection.normalized());
    };
    let offered = selection
        .slot()
        .and_then(|slot_id| entry.slots.iter().find(|slot| slot.id == slot_id))
        .map(|slot| slot.ships.as_slice())
        .unwrap_or(entry.ships.as_slice());
    if !offered.iter().any(|s| s.template_path == template_path) {
        return (SelectionOutcome::Rejected, selection.normalized());
    }
    (
        SelectionOutcome::Accepted,
        ScenarioSelection {
            scenario_id: Some(scenario_id.to_string()),
            slot_id: selection.slot().map(str::to_string),
            template_path: Some(template_path.to_string()),
        },
    )
}

/// The world TOML path the locked scenario names, or `None` while nothing is
/// locked. `worldPathFor` in the JS.
pub fn world_path_for<'a>(
    catalog: &'a ScenarioCatalog,
    selection: &ScenarioSelection,
) -> Option<&'a str> {
    let id = selection.scenario()?;
    find_scenario(catalog, id).map(|entry| entry.world.as_str())
}

/// The hulls the locked scenario's catalogue entry offers — issue #917's
/// curated allowlist, in the world's own authored order.
///
/// This is exactly what `wasm_load_world`'s `curated_ships` argument carries in
/// the browser, and what [`NativeHostConfig::curated_ships`] carries natively:
/// empty means unrestricted, matching
/// [`ScenarioEntry::ships`](crate::world::manifest::ScenarioEntry)'s own
/// semantics.
///
/// [`NativeHostConfig::curated_ships`]: crate::native_host::NativeHostConfig::curated_ships
pub fn curated_ships_for(catalog: &ScenarioCatalog, selection: &ScenarioSelection) -> Vec<String> {
    let Some(id) = selection.scenario() else {
        return Vec::new();
    };
    find_scenario(catalog, id)
        .map(|entry| {
            entry
                .ships
                .iter()
                .map(|s| s.template_path.clone())
                .collect()
        })
        .unwrap_or_default()
}
