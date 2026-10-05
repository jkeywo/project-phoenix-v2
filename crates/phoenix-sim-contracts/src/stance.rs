use serde::{Deserialize, Serialize};

/// One authored stance in a station's catalogue (issue #1107).
///
/// A stance supplies a posture FACT and policy choices to the station's ordinary
/// AI hosts "in the same broad manner that Red Alert currently informs
/// behavior" — it never applies a hidden statistical bonus and never operates a
/// fine System directly. See `docs/gdd/mechanics/command-and-crew-control.md`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StationStanceConfig {
    /// Stable stance id, referenced by `SetStationStance` on the wire.
    pub id: String,
    /// `strings.csv` id for the stance's display label. Carried to the Command
    /// console, which resolves it through `gui/strings.js`; never emitted
    /// English. Empty falls back to the raw id on the console.
    #[serde(default)]
    pub label: String,
    /// Which of the three authored kinds this stance is.
    pub kind: StanceKind,
    /// The alert posture this stance seeds for the station's AI hosts: `true`
    /// behaves as "at high alert" (the migrated Red Alert branch fires), `false`
    /// as "stood down". Validated to agree with `kind` for the two neutral
    /// fallbacks; a `standard` stance authors it freely.
    #[serde(default)]
    pub high_alert: bool,
    /// Stance lifecycle: when `true` the stance persists behind a human handoff
    /// on the directed station; when `false` (the default) it resets to the
    /// appropriate alert-neutral stance. Neutral stances are their own reset
    /// target, so the flag is meaningful only on `standard` stances.
    #[serde(default)]
    pub persist_behind_human: bool,
    /// The posture an AI-operated Command seat adopts for this station when the
    /// ship is at high (red) alert (issue #1109).
    ///
    /// This is the authored answer to "what does an uncrewed Command choose?":
    /// exactly one `standard` stance per catalogue may set it, and an AI Command
    /// operator selects that stance (through the ordinary admitted-order path a
    /// human uses) while the ship is at Red Alert, tracking the alert-neutral
    /// otherwise. Never a hard-coded stance id in Rust — the choice is authored
    /// data on the catalogue itself. Meaningful only on `standard` stances (a
    /// neutral is already the low-alert tracking default); [`validate`] rejects
    /// it on a neutral or on more than one stance.
    #[serde(default)]
    pub ai_engaged: bool,
}

/// The three authored stance kinds (issue #1107). Every station catalogue that
/// exists at all authors exactly one `normal_alert_neutral` and one
/// `high_alert_neutral`; `standard` stances are the authored postures a Command
/// operator can additionally select.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StanceKind {
    /// An authored non-neutral posture (e.g. "weapons free", "escort").
    Standard,
    /// The fallback stance for the normal (not-red) alert level.
    NormalAlertNeutral,
    /// The fallback stance for the high (red) alert level.
    HighAlertNeutral,
}
