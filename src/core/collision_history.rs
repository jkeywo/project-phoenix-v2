//! Collision attribution committed on every simulation host (#1316).
//!
//! Report telemetry is optional, but collision attribution is authoritative.
//! Collect it before the fixed-boundary digest/save readers, independently of
//! the headless report's frame-time reader. Keep every collision in producer
//! order for this mission; dropping older rows would erase attribution from a
//! saved continuation. Memory is proportional to produced collision events and
//! is released at the next ordinary mission start.

use bevy::{ecs::message::MessageCursor, prelude::*};
use serde::{Deserialize, Serialize};

use super::balance::{BalanceEvent, StampedBalanceEvent, VictimKind, WEAPON_KIND_COLLISION};

/// The existing snapshot collision row, now owned beside its live accumulator.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CollisionRecord {
    pub tick: u64,
    pub sim_t: f64,
    pub victim: String,
    pub victim_is_asteroid: bool,
    pub amount: f32,
    pub shield_absorbed: f32,
    pub hull_damage: f32,
}

impl CollisionRecord {
    fn from_event(event: &BalanceEvent, tick: u64, sim_t: f64) -> Option<Self> {
        match event {
            BalanceEvent::DamageApplied {
                weapon,
                victim,
                victim_kind,
                amount,
                shield_absorbed,
                hull_damage,
                ..
            } if weapon == WEAPON_KIND_COLLISION => Some(Self {
                tick,
                sim_t,
                victim: victim.clone(),
                victim_is_asteroid: matches!(victim_kind, VictimKind::Asteroid),
                amount: *amount,
                shield_absorbed: *shield_absorbed,
                hull_damage: *hull_damage,
            }),
            _ => None,
        }
    }

    pub(crate) fn stamped_event(&self) -> StampedBalanceEvent {
        StampedBalanceEvent {
            tick: self.tick,
            sim_t: self.sim_t,
            event: BalanceEvent::DamageApplied {
                attacker: None,
                victim: self.victim.clone(),
                victim_kind: if self.victim_is_asteroid {
                    VictimKind::Asteroid
                } else {
                    VictimKind::Ship
                },
                weapon: WEAPON_KIND_COLLISION.to_string(),
                amount: self.amount,
                shield_absorbed: self.shield_absorbed,
                hull_damage: self.hull_damage,
                system_hit: None,
            },
        }
    }
}

/// Ordered mission history. The cursor is derived input-consumption state;
/// capture and the digest read only records, never process-local message IDs.
#[derive(Resource, Default)]
pub struct CollisionHistory {
    records: Vec<CollisionRecord>,
    cursor: MessageCursor<BalanceEvent>,
}

impl CollisionHistory {
    pub fn records(&self) -> &[CollisionRecord] {
        &self.records
    }
}

struct CollisionHistoryPlugin;

impl Plugin for CollisionHistoryPlugin {
    fn build(&self, app: &mut App) {
        use crate::authoritative::{DeclareState, StateClass};
        app.add_message::<BalanceEvent>()
            .init_resource::<CollisionHistory>()
            // Ordered victim/amount/shield/hull fields fold. Tick/time/kind are
            // captured attribution metadata; the message cursor is derived and
            // reset at restore, never serialized or folded.
            .declare_state::<CollisionHistory>(
                StateClass::DeferredFold,
                "collision-attribution-history",
            )
            .add_systems(
                FixedLast,
                collect
                    .before(crate::lockstep::seal_tick_frame)
                    .before(crate::lockstep::sample_and_publish_digest)
                    .before(crate::sim_tick::advance_sim_tick),
            )
            // StateTransition runs after FixedUpdate. The opening transition
            // cannot inherit a previous mission's pending collision events.
            .add_systems(OnEnter(super::messages::GamePhase::InProgress), reset);
    }
}

pub(crate) fn register(app: &mut App) {
    if !app.is_plugin_added::<CollisionHistoryPlugin>() {
        app.add_plugins(CollisionHistoryPlugin);
    }
}

fn collect(
    events: Res<Messages<BalanceEvent>>,
    mut history: ResMut<CollisionHistory>,
    tick: Res<crate::sim_tick::SimTick>,
    time: Res<Time<Fixed>>,
) {
    let CollisionHistory { records, cursor } = &mut *history;
    records.extend(
        cursor.read(&events).filter_map(|event| {
            CollisionRecord::from_event(event, tick.0, time.elapsed_secs_f64())
        }),
    );
}

fn reset(events: Res<Messages<BalanceEvent>>, mut history: ResMut<CollisionHistory>) {
    history.records = Vec::new();
    history.cursor = events.get_cursor_current();
}

/// Replace captured history and consume only this reader's bootstrap backlog.
/// Other balance readers retain their events. GameOver entry effects emitted
/// after this call are new input and remain visible to the ordinary readers.
pub(crate) fn restore(world: &mut World, records: &[CollisionRecord]) {
    if world.contains_resource::<CollisionHistory>() || !records.is_empty() {
        let cursor = world
            .get_resource::<Messages<BalanceEvent>>()
            .map(Messages::get_cursor_current)
            .unwrap_or_default();
        world.insert_resource(CollisionHistory {
            records: records.to_vec(),
            cursor,
        });
    }
}

#[cfg(test)]
#[path = "collision_history_tests.rs"]
mod tests;
