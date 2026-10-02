//! The pure, Bevy-free heart of the transfer umbilical (issue #1160).
//!
//! Two things live here and nothing else: the **flow-arithmetic module**
//! ([`plan_flow`]) — given both docked ends' level and headroom, the authored
//! rate and direction, the docked/powered/undamaged state and the fractional
//! carry, how much moves this tick and which way — and the authored `[umbilical]`
//! terms ([`UmbilicalConfig`]) with the refusal vocabulary
//! ([`UmbilicalRefusal`]). The sibling [`crate::umbilical::server`] adapter
//! gathers the live world, calls in, and applies what comes back, deciding
//! nothing itself (AGENTS.md rule 10).
//!
//! # Why this is a module of its own, Bevy-free
//!
//! The arithmetic — how much of an authored capacity crosses the umbilical this
//! tick, clamped so it never drains a source past empty or fills a destination
//! past its ceiling, in either direction — is decided here in isolation and
//! unit-tested here, with plain integers, an `f32` rate and no app, world or
//! schedule. The adapter reads the two hulls' capacity ledgers and the dock
//! state off the live world and passes them straight in; nothing here imports
//! `bevy`. This copies the split the tractor keeps between `coupling` and
//! `server` and the dock keeps between `mating` and `server`.
//!
//! # The carry
//!
//! An authored rate is *per second*, but a tick is a fraction of a second, and a
//! capacity ledger is a whole-number count. So the module produces `rate * dt`
//! each tick, keeps the fractional remainder in a `carry` the adapter stores back
//! on the component, and moves only the whole units — the same way a continuous
//! quantity is metered onto a discrete counter. The carry holds the sub-unit
//! remainder only, never a backlog of whole units the clamp discarded, so a
//! source that runs dry and is later refilled does not suddenly dump a hoarded
//! debt (no over-shoot), and a destination that frees up headroom resumes at the
//! authored rate (no under-shoot).

use serde::{Deserialize, Serialize};

/// The authored transfer-umbilical terms for a hull's `[umbilical]` table (issue
/// #1160).
///
/// Every field is a designer's number, read from TOML: AGENTS.md rule 11, no
/// hardcoded gameplay values. A hull that authors no `[umbilical]` table carries
/// no [`crate::umbilical::server::TransferUmbilical`] component and is unchanged
/// in every way — it can move nothing across a dock. The **power group** the
/// umbilical draws from is NOT here — it is the `power_group` field of the
/// umbilical `[[system]]` block, the one authoritative place a system names its
/// group — and the adapter resolves it at spawn, exactly as the tractor and dock
/// do.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UmbilicalConfig {
    /// The `[[infrastructure.capacity]]` id being moved. Both docked ends must
    /// carry one under this id — that is what makes them the two ledgers the
    /// umbilical bridges. A dock whose partner declares no such capacity is
    /// refused ([`UmbilicalRefusal::NoCapacity`]). Not display text — a machine
    /// name, the same as a `transfer`'s capacity id.
    pub capacity: String,
    /// How much of the capacity crosses per second, in the capacity's own
    /// authored units. Metered onto the whole-number ledger through the carry, so
    /// a rate finer than one unit per tick still moves at the authored rate over
    /// time.
    pub rate: f32,
    /// Which way it flows, from the OPERATOR's point of view — the operator is
    /// who the crew are, so `Deliver` sends the operator's capacity to the docked
    /// partner and `Collect` draws the partner's into the operator.
    pub direction: UmbilicalDirection,
    /// The lowest power-group level at which the flow runs. Below it a running
    /// flow stops ([`UmbilicalRefusal::Unpowered`]). Authored, not derived from
    /// the group's nominal rung, the same as the dock's `min_power_level`.
    pub min_power_level: u8,
}

impl UmbilicalConfig {
    /// Reject an authored `[umbilical]` table that describes a flow that could
    /// never run (issue #1160). A blank capacity id, a non-positive or non-finite
    /// rate, or a zero minimum power level are author mistakes whose only other
    /// symptom would be a control the crew can start that never moves anything.
    pub fn validate(&self) -> Result<(), String> {
        if self.capacity.trim().is_empty() {
            return Err("[umbilical] capacity must name a non-empty capacity id".to_string());
        }
        if !self.rate.is_finite() || self.rate <= 0.0 {
            return Err(format!(
                "[umbilical] rate must be a positive finite number of units per second, got {}",
                self.rate
            ));
        }
        if self.min_power_level == 0 {
            return Err(
                "[umbilical] min_power_level must be at least 1 — a flow that runs at level 0 \
                 would never lose its allocation"
                    .to_string(),
            );
        }
        Ok(())
    }
}

/// Which way a [`UmbilicalConfig`] moves its capacity, named from the
/// **operator's** point of view (issue #1160).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UmbilicalDirection {
    /// Operator → docked partner.
    Deliver,
    /// Docked partner → operator.
    Collect,
}

/// The one reason a flow did not start (or stopped) this tick (issue #1160), as
/// the console shows it — a `strings.csv` id, never English. Copies the
/// tractor's and dock's refusal-plus-`string_id` shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UmbilicalRefusal {
    /// The umbilical is not docked to anything — the flow can bridge nothing.
    Undocked,
    /// The umbilical's power group is below the authored `min_power_level`.
    Unpowered,
    /// The umbilical system is damaged to `Disabled` (or `Destroyed`).
    Disabled,
    /// One or both docked ends declare no capacity under the authored id — most
    /// often the partner carries nothing to bridge to.
    NoCapacity,
}

impl UmbilicalRefusal {
    /// The `strings.csv` id the console resolves through `t()`. A `match`, not a
    /// composed id, so `check-strings.mjs` can see every id a new variant needs a
    /// row for.
    pub fn string_id(self) -> &'static str {
        match self {
            UmbilicalRefusal::Undocked => "umbilical.refused.undocked",
            UmbilicalRefusal::Unpowered => "umbilical.refused.unpowered",
            UmbilicalRefusal::Disabled => "umbilical.refused.disabled",
            UmbilicalRefusal::NoCapacity => "umbilical.refused.no_capacity",
        }
    }
}

/// One end of an umbilical flow, as the adapter reads it off an entity's
/// `[infrastructure]` capacities (issue #1160). The mirror of the operations
/// `CapacityReading`, kept in this module so the arithmetic is unit-testable
/// without the condition track.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CapacityEnd {
    /// How much is there now.
    pub level: i64,
    /// How much more the authored ceiling would still admit.
    pub headroom: i64,
}

/// The two hulls' capacity readings for the authored id (issue #1160). Either is
/// `None` when that hull declares no capacity under the id — the adapter reads a
/// missing ledger, not a zero one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FlowEnds {
    /// The operator's own capacity reading, or `None` when it carries none.
    pub operator: Option<CapacityEnd>,
    /// The docked partner's capacity reading, or `None` when it carries none (or
    /// there is no partner).
    pub partner: Option<CapacityEnd>,
}

/// The live gating state the adapter reads off the world for one tick (issue
/// #1160): the docked/powered/undamaged facts, the tick length, and the carry
/// stored from last tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlowContext {
    /// Whether the umbilical's own hull is docked to a partner this tick — the
    /// gate the whole slice hangs on (#1159's docked relationship).
    pub docked: bool,
    /// Whether the umbilical's power group is at or above its `min_power_level`.
    pub powered: bool,
    /// Whether the umbilical system is damaged to `Disabled`/`Destroyed`.
    pub disabled: bool,
    /// This tick's length in seconds.
    pub dt: f32,
    /// The fractional carry stored from last tick, in `[0.0, 1.0)`.
    pub carry: f32,
}

/// What [`plan_flow`] decides for one running umbilical this tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FlowVerdict {
    /// The flow could not run — the adapter stops it (clearing the running
    /// intent) and shows the reason. What has already moved has moved.
    Refused(UmbilicalRefusal),
    /// The flow ran. `operator_delta` and `partner_delta` are the signed moves to
    /// apply to the two ledgers (they always sum to zero), and `carry` is the
    /// fractional remainder to store for next tick. Both deltas are zero on a
    /// tick that produced less than one whole unit, or that clamped to nothing at
    /// a depleted source or a full destination — the flow keeps running, it just
    /// moved nothing this tick.
    Flowing {
        /// Units added to the operator's capacity this tick (negative when
        /// delivering).
        operator_delta: i64,
        /// Units added to the partner's capacity this tick — the exact negation
        /// of `operator_delta`, so nothing is created or lost across the dock.
        partner_delta: i64,
        /// The fractional carry to store for next tick, in `[0.0, 1.0)`.
        carry: f32,
    },
}

/// **The flow-arithmetic module.** Decide how much of the authored capacity
/// crosses the umbilical this tick, and which way (issue #1160).
///
/// Pure: the adapter reads both docked ends' capacity ledgers, the dock/power/
/// damage state and the stored carry off the live world and passes them here.
///
/// Gating runs most-actionable-first, the tractor's and dock's check order:
/// a knocked-out (`Disabled`) or unpowered umbilical is reported before the dock,
/// and an undocked one before the capacity, because there is no partner ledger to
/// read until a dock has formed. Any gate that fails returns
/// [`FlowVerdict::Refused`] and the adapter stops the flow, keeping what has
/// already moved.
///
/// Past the gates the arithmetic resolves source and destination by direction
/// (the same resolution the operations `transfer_possible` performs), produces
/// `carry + rate * dt` units, and clamps the whole part by the source's level and
/// the destination's headroom so it can neither drain past empty nor fill past
/// the ceiling — in both directions, without over- or under-shooting. The
/// fractional remainder is carried; the clamp discards no whole-unit backlog.
pub fn plan_flow(config: &UmbilicalConfig, ends: &FlowEnds, ctx: &FlowContext) -> FlowVerdict {
    // ── Gates, most-actionable-first ─────────────────────────────────────────
    if ctx.disabled {
        return FlowVerdict::Refused(UmbilicalRefusal::Disabled);
    }
    if !ctx.powered {
        return FlowVerdict::Refused(UmbilicalRefusal::Unpowered);
    }
    if !ctx.docked {
        return FlowVerdict::Refused(UmbilicalRefusal::Undocked);
    }
    let (Some(operator), Some(partner)) = (ends.operator, ends.partner) else {
        // One or both docked ends carry no ledger under the authored id — most
        // often the partner is a hull that bridges nothing.
        return FlowVerdict::Refused(UmbilicalRefusal::NoCapacity);
    };

    // ── The arithmetic ───────────────────────────────────────────────────────
    // Resolve which end is the source and which the destination from the
    // operator-relative direction, exactly as `transfer_possible` does.
    let (source, destination) = match config.direction {
        UmbilicalDirection::Deliver => (operator, partner),
        UmbilicalDirection::Collect => (partner, operator),
    };

    // Produce this tick's units, keep only the sub-unit remainder as carry. A
    // non-finite or negative product (an author or caller mistake the validator
    // already guards) falls to zero rather than corrupting the ledger.
    let produced = ctx.carry + config.rate * ctx.dt;
    let produced = if produced.is_finite() {
        produced.max(0.0)
    } else {
        0.0
    };
    let whole = produced.floor();
    let desired = whole as i64;
    let carry = produced - whole;

    // Clamp by source depletion and destination headroom — the backstop that
    // makes over/undershoot impossible in either direction.
    let moved = desired.min(source.level).min(destination.headroom).max(0);

    let (operator_delta, partner_delta) = match config.direction {
        UmbilicalDirection::Deliver => (-moved, moved),
        UmbilicalDirection::Collect => (moved, -moved),
    };
    FlowVerdict::Flowing {
        operator_delta,
        partner_delta,
        carry,
    }
}

#[cfg(test)]
#[path = "flow_tests.rs"]
mod tests;
