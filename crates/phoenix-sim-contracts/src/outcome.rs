/// Which side prevailed when a run reaches `GamePhase::GameOver`.
///
/// The engine's only structural end signal is `final_phase`
/// (`InProgress`/`GameOver`) plus the free-form `GameOverReason` string, and
/// that string is per-world — in `combat_test` it is even a strings.csv key, so
/// no substring reliably tells a victory from a defeat. Rather than string-match
/// (fragile and per-world), scenario authors *declare* the outcome on the
/// `game_over` trigger action, and the built-in player-death sites latch
/// [`Outcome::Defeat`]. The classifier reads this flag instead of guessing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Victory,
    Defeat,
}

impl Outcome {
    /// Parse the author-facing `outcome = "..."` field (case-insensitive).
    /// `Err` on any other value so a typo fails the world parse loudly rather
    /// than silently defaulting to the wrong side.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "victory" => Ok(Outcome::Victory),
            "defeat" => Ok(Outcome::Defeat),
            other => Err(format!(
                "unknown game_over outcome '{other}' (expected 'victory' or 'defeat')"
            )),
        }
    }

    /// Lowercase label — matches the `RunOutcome` spelling so the report reads
    /// consistently.
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Victory => "victory",
            Outcome::Defeat => "defeat",
        }
    }
}
