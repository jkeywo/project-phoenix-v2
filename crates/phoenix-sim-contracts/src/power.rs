pub const WEAPONS_POWER_GROUP: &str = "weapons";

/// The level a SCRIPTED power order asks a group to be commanded to (issue
/// #1398).
///
/// Two variants rather than a bare `u8` because a scenario script cannot know a
/// hull's authored numbers and must not have to. `ctx.effects.hold_fire(name)`
/// and `release_fire(name)` are the two verbs that carry this today: hold means
/// [`Self::Exact`]`(0)`, and release means "put it back where the hull says it
/// belongs", which is [`Self::AuthoredDefault`] — the group's
/// `[power_groups.<id>] default_level`, resolved by the applier against the
/// ship's own config rather than restated in the world file. A script that
/// guessed `2` would be authoring one hull's number into every scenario that
/// used the verb, which is exactly the hardcoding AGENTS.md rule 11 forbids.
///
/// [`Self::Exact`] is the general form the two verbs are special cases of. It is
/// what a future `set_power(entity, group, level)` verb would push, and it is
/// how `hold_fire` expresses itself — `Exact(0)`, one rung the applier clamps to
/// the group's own authored `min_level` like any other order.
///
/// Bevy-free and pure, beside the reactor vocabulary it speaks in: `world`'s
/// dispatch layer names it in [`crate::world::dispatch::ActionCmd`] and
/// `ship::power` resolves it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptedPowerLevel {
    /// Command the group to this exact level. Clamped by the applier to
    /// `[floor_for(group), GROUP_LEVEL_MAX]` exactly as a console order is, so a
    /// hull whose `min_level` is 1 refuses `Exact(0)` and stays warm.
    Exact(u8),
    /// Command the group back to its hull's authored `default_level` — or to
    /// [`crate::ship::config::default_power_level`], the level a group with no
    /// `[power_groups.<id>]` block boots at, when the hull describes none.
    AuthoredDefault,
}
