//! The quiet-time advisory and the crew-activity adapter behind it
//! (issue #1436, PRD #1419 M4, stories 9 and 14).
//!
//! One `Background` row appears on the [`crate::gm_attention`] queue when the
//! crew has done nothing meaningful for the authored quiet interval — 120
//! SIMULATION seconds by default. It never escalates: a Game Master who has
//! decided the lull is fine is not asked about it again more loudly ten minutes
//! later. Meaningful activity resolves it, and the next lull is a genuinely new
//! occurrence.
//!
//! # Simulation seconds, not real ones
//!
//! Quiet time is measured in [`crate::sim_tick::SimTick`] steps divided by the
//! world's authored `[global] sim_tick_hz`. That is the whole of the pause
//! contract: pausing stops `Time<Virtual>`, which starves the fixed schedule,
//! which stops `SimTick`. A paused session's quiet clock is frozen because the
//! simulation it measures is frozen, not because anything here checks
//! [`crate::gm_action::SimulationPaused`].
//!
//! The row's *age* still runs on real time like every other occurrence: age is
//! how long the GM has had the row on the desk, which is the basis a personal
//! snooze expires on. The two clocks answer two different questions and are
//! deliberately not the same clock.
//!
//! # What counts as meaningful crew activity
//!
//! Three existing sources, and nothing else. There is no keystroke telemetry
//! here, no per-operator input counter, no inference about who is at a console
//! or how fast they work — the adapter reads facts the simulation already
//! recorded for its own reasons and asks one question of them: did the crew
//! change anything.
//!
//! 1. **A semantic operator action the owning System carried out.**
//!    [`crate::core::messages::ServerMessage::ActionFeedback`] with
//!    [`ActionFeedbackOutcome::Applied`] — the terminal result the console
//!    lifecycle already settles on. Its other arm is `Refused`, so a refused or
//!    rejected press is excluded by reading the outcome rather than by a second
//!    rule. AI traffic mints no `feedback_correlation`
//!    ([`crate::command_admission::finish_admitted_action_feedback`] is a
//!    documented no-op without one), so no AI decision can reach this source at
//!    all. A completed Comms response is exactly this: `RespondToMessage` is on
//!    the correlated allowlist and `handle_respond_to_message` settles it
//!    `Applied` when the answer lands.
//!
//! 2. **A control a seat worked that has no terminal result to settle.** The
//!    helm axes are the case the issue names — sustained control, not a
//!    semantic action, so nothing ever answers "did it work?" for them — and a
//!    power reallocation, a repair dispatch or a shield-arc focus are the same
//!    shape. These are read from each ship's ordinary
//!    [`AdmittedCommands`]: admission has already checked Station tenure and
//!    the System's control source, so a command that is there is one a seat was
//!    entitled to give, and a refused one never arrives. A hand holding the
//!    stick over for three minutes admits a command every tick, which is what
//!    "sustained effective controls count" means.
//!
//!    Three exclusions, each structural rather than a taste:
//!    * an AI decision, by its `ai:` token
//!      ([`is_ai_token`](crate::command_admission::ai_emit::is_ai_token)) — the
//!      same discriminator admission itself routes on;
//!    * a payload that CAN settle a correlated result, which is counted in
//!      source 1 instead and only when its System actually applied it, so one
//!      press is never counted twice and a refused press is never counted at
//!      all; and
//!    * the two things that are not a seat working: an axis at exactly its
//!      neutral rest position, and
//!      [`SystemControlPayload::AssignStationRating`], which the ship host mints
//!      to replicate a connect/disconnect and which no operator ever sends.
//!
//! 3. **Objective progress.** [`BalanceEvent::ObjectiveChanged`] — the mission
//!    actually moved.
//!
//! Everything the issue excludes falls out of that list rather than being
//! filtered out of it: browsing a panel issues no command, a passive tick
//! produces no result, and routine AI-to-AI Coordination
//! ([`crate::core::messages::CoordinationPayload`]) is not a command, not a
//! correlated action and not an objective transition.
//!
//! Reading `response_token` here is deliberate and bounded: the field's contract
//! is that a HANDLER must not branch on it for a behavioral decision, and
//! nothing here is a handler or a decision — it is a peer-local advisory read of
//! who spoke, on the same convention admission itself routes on.
//!
//! ## The one boundary worth knowing
//!
//! A command relayed from another host arrives with `feedback_correlation: None`
//! (see `lockstep::apply_mesh_inbox`), so on a peer that is not the crew's own
//! host a remote seat's *semantic actions* produce no terminal result to
//! observe; their sustained and uncorrelated controls (source 2, which reads the
//! relayed `AdmittedCommands` on every peer) and the mission's objective
//! progress (source 3) still count. The browser Game Master this milestone
//! supports IS the crew's host, where every source is present. Native/multi-host
//! GM integration is M6, and this is the shape of the gap it inherits.

use bevy::ecs::message::MessageCursor;
use bevy::prelude::*;

use crate::command_admission::ai_emit::is_ai_token;
use crate::core::balance::BalanceEvent;
use crate::core::messages::{
    ActionFeedbackOutcome, AdmittedCommand, AdmittedCommands, GamePhase, ServerMessage,
    SystemControlPayload,
};
use crate::lobby::OutboundMessage;
use crate::sim_tick::SimTick;
use crate::world::config::WorldConfig;

/// The quiet interval a world that says nothing gets: 120 simulation seconds
/// (PRD #1419). An author overrides it with `[gm_attention] quiet_time_secs`
/// and switches it off with the independent `quiet_time_disabled`, both on the
/// shared [`GmAttentionSettings`](crate::gm_attention::GmAttentionSettings)
/// table issue #1435 introduced for the queue's other advisory.
pub const DEFAULT_QUIET_SECONDS: f32 = 120.0;

/// The occurrence id prefix every quiet row carries, so a page can tell one
/// row's family from another's without parsing the category twice.
pub const QUIET_ID_PREFIX: &str = "quiet:";

/// The String Table id a quiet row explains itself with. Takes `{seconds}`, the
/// authored interval — the row states the interval that elapsed and nothing
/// whatever about who was or was not at a console.
pub const QUIET_REASON: &str = "server.gm.attention.reason.quiet_time";

/// This peer's own crew-activity clock.
///
/// `Presentation`: derived from results, the command log and the balance stream,
/// all already classified, and read by nothing authoritative. It is not folded
/// into the digest, not captured in a snapshot and never leaves this page.
#[derive(Resource, Debug, Default)]
pub struct GmCrewActivity {
    /// The [`SimTick`] of the last meaningful crew activity this peer observed.
    last_activity_tick: u64,
    /// Which quiet occurrence is on the desk. Incremented as the condition
    /// STARTS holding, so a snooze taken against one lull cannot hide the next.
    ordinal: u64,
    /// Was the row published at the previous look?
    open: bool,
}

impl GmCrewActivity {
    /// The tick the last meaningful crew activity was observed on.
    pub fn last_activity_tick(&self) -> u64 {
        self.last_activity_tick
    }

    /// Record meaningful activity at `tick`, resolving any open quiet row.
    ///
    /// Public for the tests that drive the adapter directly; production writes
    /// through [`observe_crew_activity`].
    pub fn observe(&mut self, tick: u64) {
        self.last_activity_tick = tick;
    }

    /// How many simulation TICKS have passed since the last activity.
    ///
    /// `saturating_sub` because a snapshot restore moves `SimTick` backwards;
    /// [`rebase_after_restore`] rebases the clock, and this keeps a single frame
    /// between the two from reading a wrapped interval. Ticks rather than
    /// seconds because the authored interval is converted to an exact tick count
    /// once, at the same rounding every peer applies
    /// ([`GmAttentionSettings::quiet_time_ticks`]), so the boundary is an
    /// integer comparison rather than a float one.
    pub fn quiet_ticks(&self, tick: u64) -> u64 {
        tick.saturating_sub(self.last_activity_tick)
    }

    /// Start the clock again from `tick` with nothing waiting, ending any open
    /// occurrence so the next lull is a fresh one.
    fn rebase(&mut self, tick: u64) {
        self.last_activity_tick = tick;
        self.open = false;
    }
}

/// The commands that arrive on a seat's channel without a seat having worked
/// anything: an axis resting at exactly neutral, and the host's own
/// Station-rating replication.
///
/// Exactly-neutral is excluded deliberately: a centred stick is the ship not
/// being flown, and a console that re-states neutral is not somebody working.
fn is_inert(payload: &SystemControlPayload) -> bool {
    match payload {
        SystemControlPayload::SetThrust { value }
        | SystemControlPayload::SetSteering { value }
        | SystemControlPayload::LateralThrustInput { lateral: value }
        | SystemControlPayload::VerticalThrustInput { vertical: value } => *value == 0.0,
        SystemControlPayload::AssignStationRating { .. } => true,
        _ => false,
    }
}

/// Does this admitted command count as effective human control on its own?
///
/// Only when nothing else can answer for it. A payload whose System settles a
/// correlated result is counted from THAT result instead (source 1), so a
/// refused press is excluded and no press is counted twice. See the module
/// header for the three exclusions and why each one is structural.
pub fn counts_as_worked_control(command: &AdmittedCommand) -> bool {
    if is_ai_token(command.response_token.as_deref()) {
        return false;
    }
    if crate::command_admission::supports_correlated_action_feedback_for_kind(
        &command.target,
        &command.payload,
        None,
    ) {
        return false;
    }
    !is_inert(&command.payload)
}

/// Read the three activity sources and stamp the clock.
///
/// Runs in `FixedLast`, before [`crate::sim_tick::advance_sim_tick`], so the
/// tick it stamps is the step it observed. Ungated: the clock has to keep
/// running whether or not a Game Master is looking, or the first look after a
/// busy minute would report a lull that never happened.
#[allow(clippy::too_many_arguments)]
pub fn observe_crew_activity(
    tick: Res<SimTick>,
    admitted: Query<&AdmittedCommands>,
    outbound: Option<Res<Messages<OutboundMessage>>>,
    balance: Option<Res<Messages<BalanceEvent>>>,
    mut outbound_cursor: Local<MessageCursor<OutboundMessage>>,
    mut balance_cursor: Local<MessageCursor<BalanceEvent>>,
    mut activity: ResMut<GmCrewActivity>,
) {
    let mut active = false;

    // 1. A semantic operator action its owning System actually carried out.
    if let Some(messages) = outbound.as_deref() {
        for message in outbound_cursor.read(messages) {
            if matches!(
                message.msg,
                ServerMessage::ActionFeedback {
                    outcome: ActionFeedbackOutcome::Applied,
                    ..
                }
            ) {
                active = true;
            }
        }
    }

    // 2. A control a seat worked that has no terminal result to settle.
    //
    // `AdmittedCommands` is cleared and refilled by `admit_system_commands`
    // every tick, so this reads exactly this step's traffic and needs no cursor
    // of its own. Every hull is walked, not only `LocalShip`: on a peer that is
    // not the crew's own host, a relayed command is the only trace their seat
    // leaves.
    if admitted
        .iter()
        .flat_map(|commands| commands.0.iter())
        .any(counts_as_worked_control)
    {
        active = true;
    }

    // 3. The mission moved.
    if let Some(messages) = balance.as_deref() {
        for event in balance_cursor.read(messages) {
            if matches!(event, BalanceEvent::ObjectiveChanged { .. }) {
                active = true;
            }
        }
    }

    if active {
        activity.observe(tick.0);
    }
}

/// Start the quiet clock at the beginning of a run.
///
/// Without this a scenario that sat in the lobby for three minutes would open
/// with a quiet advisory about a crew that has not had a chance to do anything
/// yet: `SimTick` counts fixed steps, and the lobby takes them too.
pub fn reset_on_run_start(tick: Res<SimTick>, mut activity: ResMut<GmCrewActivity>) {
    activity.rebase(tick.0);
}

/// A snapshot/join installs a different tick into a live app. Rebase the quiet
/// clock onto it, so a restored session neither opens on a lull it did not have
/// nor inherits the pre-restore one.
pub fn rebase_after_restore(world: &mut World) {
    let tick = world.get_resource::<SimTick>().map_or(0, |tick| tick.0);
    let Some(mut activity) = world.get_resource_mut::<GmCrewActivity>() else {
        return;
    };
    activity.rebase(tick);
}

/// This world's quiet-time settings: the authored interval in seconds, the same
/// interval as an exact tick count, and whether the advisory is wanted at all.
pub fn quiet_settings(world: Option<&WorldConfig>) -> (f32, u64, bool) {
    let settings = world
        .map(|world| world.gm_attention.clone())
        .unwrap_or_default();
    let hz = world.map_or_else(
        || crate::entities::config::GlobalConfig::default().sim_tick_hz,
        |world| world.global.sim_tick_hz,
    );
    (
        settings.quiet_time_secs,
        settings.quiet_time_ticks(hz),
        !settings.quiet_time_disabled,
    )
}

/// Everything one quiet row needs, or `None` while the crew is working.
///
/// Split from the row-building in [`crate::gm_attention`] so the whole rule —
/// the interval, the disable, the recurrence ordinal — is testable without a
/// `World`, and so the projection keeps one place that builds a row.
pub struct QuietRow {
    /// The occurrence id, with its recurrence ordinal already applied.
    pub id: String,
    /// The authored interval, in whole simulation seconds, for the sentence.
    pub seconds: u64,
}

/// Decide whether the quiet row is on the desk this look, minting a fresh
/// occurrence id each time the condition starts holding.
///
/// The BAND is not a parameter and is not authorable: a quiet row is always
/// `Background`, which is how "age alone never escalates it" is made true by
/// construction rather than by a rule somebody could later relax.
pub fn quiet_row(
    world: Option<&WorldConfig>,
    tick: u64,
    activity: &mut GmCrewActivity,
) -> Option<QuietRow> {
    let (seconds, ticks, enabled) = quiet_settings(world);
    let holds = enabled && activity.quiet_ticks(tick) >= ticks;
    if !holds {
        activity.open = false;
        return None;
    }
    if !activity.open {
        activity.ordinal += 1;
        activity.open = true;
    }
    Some(QuietRow {
        id: format!("{QUIET_ID_PREFIX}{}", activity.ordinal),
        seconds: seconds.round().max(0.0) as u64,
    })
}

/// Registers the crew-activity clock. Separate from
/// [`crate::gm_attention::GmAttentionPlugin`]'s publisher because the clock runs
/// on every peer that simulates, not only on one that is showing a queue.
pub struct GmQuietPlugin;

impl Plugin for GmQuietPlugin {
    fn build(&self, app: &mut App) {
        use crate::authoritative::{DeclareState, StateClass};

        app.init_resource::<GmCrewActivity>()
            .declare_state::<GmCrewActivity>(StateClass::Presentation, "gm-t3-quiet-time")
            .add_systems(
                FixedLast,
                observe_crew_activity.before(crate::sim_tick::advance_sim_tick),
            )
            .add_systems(OnEnter(GamePhase::InProgress), reset_on_run_start);
    }
}

#[cfg(test)]
#[path = "gm_quiet_tests.rs"]
mod tests;
