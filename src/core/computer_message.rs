//! The ship's-computer message vocabulary and authoritative state machine
//! (issue #1342, PRD #1337).
//!
//! A scenario shows one timed message on the Viewscreen: a stable id, a
//! localized message String Id, a severity (`info`/`advisory`/`warning`/
//! `critical`), a positive simulation-time duration, and an optional Station
//! cue. Only the LATEST message is ever active — a new one supersedes
//! whatever is showing immediately, unconditionally, with no priority queue
//! and no resumption of a superseded message once the newer one is gone.
//!
//! # Determinism and pause
//!
//! Expiry is measured in `SimTick` ticks, never wall-clock seconds: a
//! duration is converted to a tick count once, at `show` time, the same
//! rounding rule [`crate::world::script::schedule::seconds_to_ticks`] uses
//! (duplicated rather than imported — see the note on
//! [`duration_ticks`] for why `core` does not depend on `world`). Because
//! `SimTick` itself does not advance while the simulation is paused, checking
//! `now_tick >= expires_tick` each fixed tick freezes the countdown for free —
//! no scheduled callback, no timer that could drift from the tick the rest of
//! the sim is keyed on.
//!
//! # Presentation-only
//!
//! Messages are Viewscreen-only and never read by any AI system — nothing in
//! the fixed tick branches on [`ComputerMessageState::text`] or `severity`.
//! This state is classified `Presentation` at its registration site (the same
//! class [`crate::core::narrative::NarrativeMark`] carries): never folded into
//! the authoritative digest (`src/sim_digest.rs`), never captured by the
//! resume snapshot (`src/snapshot.rs`).

use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::core::messages::StationId;

/// The four severities a scenario may author.
///
/// Doubles as the ship-authored audio config's lookup key
/// (`ShipAudioConfig::computer_message`) and the CSS class the Viewscreen
/// banner selects — one vocabulary, one spelling, for both.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerMessageSeverity {
    Info,
    Advisory,
    Warning,
    Critical,
}

impl ComputerMessageSeverity {
    /// The stable snake_case label written into JSON, narrative telemetry
    /// detail, and the audio-cue wire shape. Hand-written, not derived, so
    /// the wire vocabulary is visible at the point it is promised — the same
    /// reason `NarrativeKind::as_str` is hand-written.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Advisory => "advisory",
            Self::Warning => "warning",
            Self::Critical => "critical",
        }
    }

    /// Parse the author-facing severity word, case-insensitively and
    /// trimmed. `Err` on anything else, so a typo raises at the script
    /// boundary (`ctx.effects.show_message(..)`) rather than silently
    /// defaulting to a severity the author did not choose — the same
    /// contract [`crate::core::narrative::NarrativeKind::parse_outcome`]
    /// holds for marked-entity outcomes.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "info" => Ok(Self::Info),
            "advisory" => Ok(Self::Advisory),
            "warning" => Ok(Self::Warning),
            "critical" => Ok(Self::Critical),
            other => Err(format!(
                "unknown computer-message severity '{other}' (expected one of: \
                 info, advisory, warning, critical)"
            )),
        }
    }
}

/// One authored `show_message(..)` call, already validated at the script
/// boundary (severity parsed, `duration_secs` checked positive) before it
/// ever reaches the effect queue this rides — see
/// `world::script::effects::register_effects`.
#[derive(Clone, Debug, PartialEq)]
pub struct ComputerMessageRequest {
    /// The author's own stable identifier for this message.
    pub id: String,
    /// `strings.csv` id — never localized prose (AGENTS.md rule 11).
    pub text: String,
    pub severity: ComputerMessageSeverity,
    /// Whole simulation seconds. Positive, enforced at the script boundary.
    pub duration_secs: i64,
    /// Optional Station cue (`"helm"`, `"tactical"`, …). Not validated
    /// against the ship's authored roster here — an unknown station simply
    /// renders no badge, the same tolerant handling other loosely-addressed
    /// station ids get.
    pub station: Option<StationId>,
}

/// The single active message's live state.
#[derive(Clone, Debug, PartialEq)]
pub struct ComputerMessageState {
    pub id: String,
    pub text: String,
    pub severity: ComputerMessageSeverity,
    pub station: Option<StationId>,
    /// The `SimTick` it was shown on.
    pub shown_tick: u64,
    /// The `SimTick` at or after which it expires.
    pub expires_tick: u64,
}

/// What showing a new message displaced, if anything — carries just enough
/// for the narrative emitter to log a `superseded` event about the id that
/// stopped showing.
#[derive(Clone, Debug, PartialEq)]
pub struct SupersededMessage {
    pub id: String,
}

/// The authoritative "one message at a time" state (issue #1342).
///
/// `Resource::default()` — nothing showing — is also what a fresh mission and
/// a lobby return both reset to (see `clear`).
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct ActiveComputerMessage {
    pub current: Option<ComputerMessageState>,
}

impl ActiveComputerMessage {
    /// Show `request`, superseding whatever is currently active —
    /// immediately and unconditionally: there is no priority comparison and
    /// a superseded message is never resumed once the new one is gone.
    /// Returns the message it displaced, if any.
    pub fn show(
        &mut self,
        request: &ComputerMessageRequest,
        now_tick: u64,
        tick_hz: f32,
    ) -> Option<SupersededMessage> {
        let superseded = self.current.take().map(|s| SupersededMessage { id: s.id });
        let ticks = duration_ticks(request.duration_secs, tick_hz);
        self.current = Some(ComputerMessageState {
            id: request.id.clone(),
            text: request.text.clone(),
            severity: request.severity,
            station: request.station.clone(),
            shown_tick: now_tick,
            expires_tick: now_tick + ticks,
        });
        superseded
    }

    /// If the active message's time is up at `now_tick`, clear it and return
    /// its id. `None` when nothing is showing or nothing is due yet — so a
    /// caller can tell "no message" apart from "not expired yet" without a
    /// second query.
    pub fn expire_if_due(&mut self, now_tick: u64) -> Option<String> {
        let due = self
            .current
            .as_ref()
            .is_some_and(|s| now_tick >= s.expires_tick);
        if !due {
            return None;
        }
        self.current.take().map(|s| s.id)
    }

    /// Unconditionally clear the active message — mission end, lobby return —
    /// returning its id if there was one. Not itself a narrative beat (see
    /// `crate::narrative`'s computer-message emitter): the issue's shown /
    /// superseded / expired trio does not include this transition.
    pub fn clear(&mut self) -> Option<String> {
        self.current.take().map(|s| s.id)
    }
}

/// Convert a positive `duration_secs` to a tick count, rounding the same way
/// [`crate::world::script::schedule::seconds_to_ticks`] does.
///
/// Not a call to that function: this module lives under `core`, which must
/// not depend on `world` (see `crate::narrative`'s module doc for the same
/// rule applied to the narrative-event producers). The one-line rounding
/// formula is duplicated rather than shared through an extra crate seam for
/// two call sites. Floors at 1 tick, so a message always shows for at least
/// one tick even authored at a fractional-second duration below the tick
/// period, and can never expire on the very tick it was shown.
fn duration_ticks(duration_secs: i64, tick_hz: f32) -> u64 {
    ((duration_secs.max(1) as f64) * (tick_hz as f64))
        .round()
        .max(1.0) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    const HZ: f32 = 60.0;

    fn request(id: &str, secs: i64) -> ComputerMessageRequest {
        ComputerMessageRequest {
            id: id.into(),
            text: "world.probe.computer_message.reach".into(),
            severity: ComputerMessageSeverity::Advisory,
            duration_secs: secs,
            station: None,
        }
    }

    // ── ComputerMessageSeverity::parse ────────────────────────────────────

    #[test]
    fn parse_accepts_every_severity_case_insensitively_and_trimmed() {
        assert_eq!(
            ComputerMessageSeverity::parse("Info"),
            Ok(ComputerMessageSeverity::Info)
        );
        assert_eq!(
            ComputerMessageSeverity::parse(" ADVISORY "),
            Ok(ComputerMessageSeverity::Advisory)
        );
        assert_eq!(
            ComputerMessageSeverity::parse("warning"),
            Ok(ComputerMessageSeverity::Warning)
        );
        assert_eq!(
            ComputerMessageSeverity::parse("CRITICAL"),
            Ok(ComputerMessageSeverity::Critical)
        );
    }

    #[test]
    fn parse_rejects_an_unknown_severity() {
        assert!(ComputerMessageSeverity::parse("urgent").is_err());
        assert!(ComputerMessageSeverity::parse("").is_err());
    }

    #[test]
    fn severity_labels_round_trip_through_parse() {
        for s in [
            ComputerMessageSeverity::Info,
            ComputerMessageSeverity::Advisory,
            ComputerMessageSeverity::Warning,
            ComputerMessageSeverity::Critical,
        ] {
            assert_eq!(ComputerMessageSeverity::parse(s.as_str()), Ok(s));
        }
    }

    // ── show / supersede ──────────────────────────────────────────────────

    #[test]
    fn showing_the_first_message_supersedes_nothing() {
        let mut active = ActiveComputerMessage::default();
        let superseded = active.show(&request("hail_debris", 10), 0, HZ);
        assert!(superseded.is_none());
        let current = active.current.as_ref().unwrap();
        assert_eq!(current.id, "hail_debris");
        assert_eq!(current.shown_tick, 0);
        assert_eq!(current.expires_tick, 600, "10s at 60Hz is tick 600");
    }

    #[test]
    fn a_second_message_supersedes_the_first_immediately() {
        let mut active = ActiveComputerMessage::default();
        active.show(&request("first", 100), 0, HZ);
        let superseded = active.show(&request("second", 5), 10, HZ);
        assert_eq!(superseded, Some(SupersededMessage { id: "first".into() }));
        assert_eq!(active.current.as_ref().unwrap().id, "second");
        assert_eq!(active.current.as_ref().unwrap().expires_tick, 10 + 300);
    }

    #[test]
    fn a_superseded_message_is_never_resumed() {
        // Once "second" expires there is nothing left to fall back to — the
        // state holds only ONE message, never a stack.
        let mut active = ActiveComputerMessage::default();
        active.show(&request("first", 100), 0, HZ);
        active.show(&request("second", 5), 0, HZ);
        active.expire_if_due(300);
        assert!(active.current.is_none(), "no resumption of 'first'");
    }

    #[test]
    fn show_carries_severity_and_station_through() {
        let mut active = ActiveComputerMessage::default();
        let req = ComputerMessageRequest {
            id: "charge_ready".into(),
            text: "world.probe.computer_message.charge".into(),
            severity: ComputerMessageSeverity::Critical,
            duration_secs: 8,
            station: Some(StationId("tactical".into())),
        };
        active.show(&req, 0, HZ);
        let current = active.current.as_ref().unwrap();
        assert_eq!(current.severity, ComputerMessageSeverity::Critical);
        assert_eq!(current.station, Some(StationId("tactical".into())));
        assert_eq!(current.text, "world.probe.computer_message.charge");
    }

    // ── expiry ─────────────────────────────────────────────────────────────

    #[test]
    fn expiry_does_not_fire_before_its_tick() {
        let mut active = ActiveComputerMessage::default();
        active.show(&request("hail_debris", 10), 0, HZ);
        assert_eq!(active.expire_if_due(599), None, "one tick early");
        assert!(active.current.is_some());
    }

    #[test]
    fn expiry_fires_exactly_on_its_due_tick_and_clears_state() {
        let mut active = ActiveComputerMessage::default();
        active.show(&request("hail_debris", 10), 0, HZ);
        assert_eq!(active.expire_if_due(600), Some("hail_debris".to_string()));
        assert!(active.current.is_none());
    }

    #[test]
    fn expiry_on_an_empty_state_is_a_silent_none() {
        let mut active = ActiveComputerMessage::default();
        assert_eq!(active.expire_if_due(1_000_000), None);
    }

    #[test]
    fn a_zero_or_negative_duration_still_arms_at_least_one_tick_out() {
        // Not reachable in practice — the script boundary rejects a
        // non-positive duration before this is ever constructed — but the
        // pure state machine must not divide-by/collapse-to zero if it ever
        // is.
        let mut active = ActiveComputerMessage::default();
        active.show(&request("degenerate", 0), 5, HZ);
        let current = active.current.as_ref().unwrap();
        assert!(
            current.expires_tick > 5,
            "must not expire on the same tick it was shown"
        );
    }

    // ── clear ────────────────────────────────────────────────────────────

    #[test]
    fn clear_on_empty_state_returns_none() {
        let mut active = ActiveComputerMessage::default();
        assert_eq!(active.clear(), None);
    }

    #[test]
    fn clear_returns_the_cleared_id_and_empties_state() {
        let mut active = ActiveComputerMessage::default();
        active.show(&request("mission_wrap", 30), 0, HZ);
        assert_eq!(active.clear(), Some("mission_wrap".to_string()));
        assert!(active.current.is_none());
    }

    #[test]
    fn default_state_is_empty() {
        assert_eq!(ActiveComputerMessage::default().current, None);
    }
}
