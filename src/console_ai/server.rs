//! Server-side console AI orchestrator.
//!
//! Complexity preset machinery (ComplexityRules, ConsoleComplexityState,
//! build_complexity_rules, track_complexity_changes) removed in B4 (issue #534).
//! AI behaviour is now gated by StationRatingConfig.ai_tuning.

// AI rule keys — match the keys used in [[station.rating]].ai_tuning tables.
pub const AI_RULE_TORPEDO_AUTO_FIRE: &str = "torpedo_auto_fire";
pub const AI_RULE_FREQUENCY_MATCH: &str = "frequency_match";
pub const AI_RULE_AUTO_HINT: &str = "auto_hint";
pub const AI_RULE_MOVEMENT_RULE: &str = "movement_rule";
pub const AI_RULE_RED_ALERT_RULE: &str = "red_alert_rule";

/// Plugin stub — all systems removed in B4.
pub struct ConsoleAiPlugin;

impl bevy::prelude::Plugin for ConsoleAiPlugin {
    fn build(&self, _app: &mut bevy::prelude::App) {}
}
