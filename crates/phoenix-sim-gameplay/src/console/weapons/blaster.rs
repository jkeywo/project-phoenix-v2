use bevy::prelude::*;

/// Wraps the pure-Rust blaster system(s) so they can be used as a Bevy
/// component on each ship entity (issue #631).
///
/// Each element corresponds to one `[[weapons_console.blaster_banks]]` entry.
/// A ship with no blaster banks will have an empty `Vec`.
#[derive(Resource, Component, Clone, Default)]
pub struct BlasterSystemResource(pub Vec<crate::weapons::blaster::BlasterSystem>);

/// Per-ship map of each blaster bank's inline stateless open-fire policy
/// (issue #781), keyed by `BlasterBankConfig.id`. The blaster twin of
/// [`crate::console::weapons::PhaserBankAiPolicies`]: built at spawn from each
/// bank's authored `ai` block, falling back to the canonical
/// [`crate::entities::config::default_blaster_bank_ai_config`] (unconditional
/// fire) so a bank without an authored policy keeps auto-firing exactly as before
/// (AC1). Read by [`tick_blaster_auto_fire`].
#[derive(Component, Default, Clone, Debug)]
pub struct BlasterBankAiPolicies(
    pub  std::collections::HashMap<
        crate::entities::config::BlasterBankId,
        crate::ai::policy::AiPolicy,
    >,
);

/// Seed the per-tick policy fact snapshot for one blaster bank's open-fire
/// decision (issue #781), the blaster twin of
/// [`crate::console::weapons::seed_phaser_bank_facts`]. Closes the #779 empty-facts
/// edge for blaster banks: the host resolves the bank's live readiness before
/// calling this, so a `fact(...)` guard evaluates over real per-bank state while
/// `policy.rs` stays Bevy-free.
/// `posture` carries the firing readings — the ship's alert (issue #872), the
/// captain's hold (issue #1041) and whether THIS bank's authored power group is
/// cold (issue #1396); see
/// [`crate::console::weapons::seed_phaser_bank_facts`] and
/// [`crate::console::weapons::WeaponsAlertPosture`] for the contract. Seeded
/// unconditionally so an authored guard reads a real `0.0`, never an absent
/// fact.
pub fn seed_blaster_bank_facts(
    target_valid: bool,
    on_cooldown: bool,
    cooldown_remaining: f32,
    in_range: bool,
    in_arc: bool,
    posture: crate::console::weapons::WeaponsAlertPosture,
) -> crate::world::flags::AiFacts {
    use crate::entities::ai_flag_hosts as fid;
    let mut facts = crate::world::flags::AiFacts::new();
    facts.set_fact(fid::TARGET_VALID, if target_valid { 1.0 } else { 0.0 });
    facts.set_fact(fid::ON_COOLDOWN, if on_cooldown { 1.0 } else { 0.0 });
    facts.set_fact(fid::COOLDOWN_REMAINING, cooldown_remaining as f64);
    facts.set_fact(fid::IN_RANGE, if in_range { 1.0 } else { 0.0 });
    facts.set_fact(fid::IN_ARC, if in_arc { 1.0 } else { 0.0 });
    facts.set_fact(fid::RED_ALERT, posture.alert_fact_value());
    facts
}

/// Resolve a blaster bank's policy to a bare "open fire this tick?" boolean
/// (issue #781). Returns `true` only when a guard fires on the `blaster_fire`
/// channel yielding `FireBlaster`; `None`/idle/mismatched verbs "hold".
pub fn blaster_bank_policy_fires(
    policy: &crate::ai::policy::AiPolicy,
    facts: &crate::world::flags::AiFacts,
    flags: &[&crate::world::flags::FlagStore],
) -> bool {
    policy.resolve_channel(crate::entities::config::BLASTER_FIRE_CHANNEL, facts, flags)
        == Some(&crate::ai::policy::AiPolicyVerb::FireBlaster)
}
