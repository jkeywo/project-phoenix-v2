//! Entity schema: repair. Public paths remain in the parent module.
use super::*;

/// Config block for the repair-team state machine in a ship TOML.
///
/// Loaded from `[repair]` in the ship entity TOML (and any NPC ship TOML
/// that wishes to override repair pacing). All fields are optional; missing
/// fields fall back to the same defaults as `RepairTimings::default()` and
/// to the historical hardcoded constants (`TRAVEL_DURATION = 5.0`,
/// `REPAIR_RATE_HP_PER_SEC = 0.5`).
///
/// The same values are forwarded to the client via
/// `ShipClientConfig.repair_travel_secs` and
/// `ShipClientConfig.repair_rate_hp_per_sec` so that the Repair panel UI
/// can derive its progress-bar timings without redefining the constants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairConfig {
    /// Number of repair teams available to this ship. Absent ⇒ 0 ⇒ this ship
    /// has no repair teams — see [`Self::declares_teams`].
    #[serde(default)]
    pub repair_team_count: u32,
    /// Seconds a team spends travelling to a console (and the same again returning).
    #[serde(default = "default_repair_travel_duration_secs")]
    pub travel_duration_secs: f32,
    /// HP restored per second while a team is at a console.
    #[serde(default = "default_repair_rate_hp_per_sec")]
    pub repair_rate_hp_per_sec: f32,
    /// Inline per-system target selector (issue #785). Loaded from
    /// `[repair.selector]`; absent ⇒ the canonical
    /// [`default_repair_target_selector_config`] is synthesised at spawn.
    /// `operate_repair_ai` runs it once per free team to rank the ship's
    /// damaged stations into ordinary admitted `DispatchRepairTeam` inputs.
    ///
    /// This is the first selector block that is NOT inside a `*_console`
    /// section: repair teams are a ship-wide engineering capability whose
    /// tunables already live under `[repair]`, so the selector joins them there
    /// rather than inventing a `[repair_console]` table the wire never uses.
    #[serde(default)]
    pub selector: Option<FineSystemAiSelectorToml>,
    /// External repair-team dispatch (issue #1161). Loaded from
    /// `[repair.external_dispatch]`; present on a hull whose repair console can
    /// send a team to a nearby ally or structure, absent for everything else —
    /// which carries no `ExternalRepairDispatch` component and cannot dispatch a
    /// team abroad. It joins the other repair tunables under `[repair]` for
    /// `selector`'s reason: a team crossing over is a repair-console capability,
    /// not a `[[system]]` of its own. The reach and the repair rate are its two
    /// authored numbers (AGENTS.md rule 11).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_dispatch: Option<crate::console::repair::external::ExternalRepairConfig>,
}

fn default_repair_travel_duration_secs() -> f32 {
    5.0
}
fn default_repair_rate_hp_per_sec() -> f32 {
    0.5
}

impl Default for RepairConfig {
    fn default() -> Self {
        Self {
            repair_team_count: 0,
            travel_duration_secs: default_repair_travel_duration_secs(),
            repair_rate_hp_per_sec: default_repair_rate_hp_per_sec(),
            selector: None,
            external_dispatch: None,
        }
    }
}

impl RepairConfig {
    /// Whether this block gives the ship repair TEAMS, as opposed to existing
    /// only to carry `[repair.selector]`.
    ///
    /// Until #885b every NPC hull that authored no `[repair]` block had no
    /// teams, and the spawner used the block's mere PRESENCE as the gate. That
    /// stopped working the moment every hull had to author `[repair.selector]`
    /// to satisfy PRD #774 US7: a selector is a ranking policy, and TOML has no
    /// way to write `[repair.selector]` without also bringing `[repair]` into
    /// existence. Presence would then have handed two repair teams to six NPC
    /// hulls that never had any — a gameplay change smuggled in by a table
    /// header.
    ///
    /// So the gate is the count: **a ship has repair teams when its TOML says
    /// how many.** `repair_team_count = 0`, or omitted, means none. The
    /// `[repair.selector]` block is attached to every AI-bearing ship either
    /// way — the teams component is what gates dispatch, so a ship that gains
    /// teams later already has its ranking.
    ///
    /// The PLAYER ship does not come through here: `spawn_game_start_entities`
    /// gates its teams on `[hull]` and keeps its own `unwrap_or(2)` fallback,
    /// so a player hull that omits the count still crews two teams.
    pub fn declares_teams(&self) -> bool {
        self.repair_team_count > 0
    }

    /// Convert this TOML config into a runtime `RepairTimings`.
    pub fn to_runtime(&self) -> crate::modifiers::repair_teams::RepairTimings {
        crate::modifiers::repair_teams::RepairTimings {
            travel_duration: self.travel_duration_secs,
            repair_rate_hp_per_sec: self.repair_rate_hp_per_sec,
        }
    }
}
