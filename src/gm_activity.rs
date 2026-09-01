//! Peer-local bounded Game Master activity feed (issue #1297 / PRD #930).
//!
//! The feed is a presentation projection over the simulation's existing
//! unconditional [`BalanceEvent`] stream. It deliberately invents no GM event
//! bus and reaches only the browser Host Channel -- never `ServerMessage`, a
//! mesh frame, `SimOutbox`, snapshot, replay, or the authoritative digest.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::bounded_history::BoundedRing;
use crate::console_bridge::GmActivityFeedChanged;
use crate::core::balance::{BalanceEvent, VictimKind};
use crate::core::messages::GamePhase;
use crate::entities::spawner::{EntityName, EntityUuid};
use crate::gm_projection::{BrowserGameMaster, GmEntityReference};
use crate::server_app::AsteroidUuid;

/// The two existing structured event families projected in M1.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GmActivityCategory {
    Damage,
    Destruction,
}

/// Damage-only detail retained exactly from `BalanceEvent::DamageApplied`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct GmActivityDamage {
    pub victim_kind: VictimKind,
    pub weapon: String,
    pub amount: f32,
    pub shield_absorbed: f32,
    pub hull_damage: f32,
    pub system_hit: Option<String>,
}

/// One tick-stamped activity row. `source` is the attacker or killer; `None`
/// remains an explicit environmental source rather than a fabricated entity.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct GmActivityEntry {
    pub tick: u64,
    pub category: GmActivityCategory,
    pub victim: GmEntityReference,
    pub source: Option<GmEntityReference>,
    pub damage: Option<GmActivityDamage>,
}

/// Absolute oldest-first bounded page payload.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct GmActivityFeedPayload {
    pub capacity: usize,
    pub entries: Vec<GmActivityEntry>,
}

/// Pure bounded reduction used by the production resource and unit tests.
#[derive(Clone, Debug, PartialEq)]
pub struct GmActivityHistory {
    entries: BoundedRing<GmActivityEntry>,
}

impl GmActivityHistory {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: BoundedRing::new(capacity),
        }
    }

    pub fn capacity(&self) -> usize {
        self.entries.capacity()
    }

    /// Re-author the bound, evicting oldest rows through `BoundedRing`.
    pub fn set_capacity(&mut self, capacity: usize) -> bool {
        if self.entries.capacity() == capacity {
            return false;
        }
        self.entries.set_capacity(capacity);
        true
    }

    /// Append every row without deduplication. Repeated facts are distinct
    /// occurrences even when every field compares equal.
    pub fn append(&mut self, entries: impl IntoIterator<Item = GmActivityEntry>) -> bool {
        let mut changed = false;
        for entry in entries {
            self.entries.push(entry);
            changed = true;
        }
        changed
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn payload(&self) -> GmActivityFeedPayload {
        GmActivityFeedPayload {
            capacity: self.entries.capacity(),
            entries: self.entries.iter().cloned().collect(),
        }
    }

    fn references_identity(&self, identity: &str) -> bool {
        self.entries.iter().any(|entry| {
            entry.victim.entity_id == identity
                || entry
                    .source
                    .as_ref()
                    .is_some_and(|source| source.entity_id == identity)
        })
    }
}

fn reference(identity: &str, names: &BTreeMap<String, String>) -> GmEntityReference {
    GmEntityReference {
        entity_id: identity.to_owned(),
        name: names
            .get(identity)
            .cloned()
            .unwrap_or_else(|| identity.to_owned()),
    }
}

fn category_rank(category: GmActivityCategory) -> u8 {
    match category {
        GmActivityCategory::Damage => 0,
        GmActivityCategory::Destruction => 1,
    }
}

fn compare_damage(left: &Option<GmActivityDamage>, right: &Option<GmActivityDamage>) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => left
            .victim_kind
            .as_str()
            .cmp(right.victim_kind.as_str())
            .then_with(|| left.weapon.cmp(&right.weapon))
            .then_with(|| left.amount.total_cmp(&right.amount))
            .then_with(|| left.shield_absorbed.total_cmp(&right.shield_absorbed))
            .then_with(|| left.hull_damage.total_cmp(&right.hull_damage))
            .then_with(|| left.system_hit.cmp(&right.system_hit)),
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn compare_entries(left: &GmActivityEntry, right: &GmActivityEntry) -> Ordering {
    category_rank(left.category)
        .cmp(&category_rank(right.category))
        .then_with(|| left.victim.entity_id.cmp(&right.victim.entity_id))
        .then_with(|| {
            left.source
                .as_ref()
                .map(|source| source.entity_id.as_str())
                .cmp(
                    &right
                        .source
                        .as_ref()
                        .map(|source| source.entity_id.as_str()),
                )
        })
        .then_with(|| compare_damage(&left.damage, &right.damage))
}

/// Project only the two unconditional sources in scope, then impose one
/// canonical order for the current tick. Stable sorting plus no deduplication
/// preserves exact repeated rows.
pub fn project_balance_events<'a>(
    tick: u64,
    events: impl IntoIterator<Item = &'a BalanceEvent>,
    names: &BTreeMap<String, String>,
) -> Vec<GmActivityEntry> {
    let mut projected: Vec<GmActivityEntry> = events
        .into_iter()
        .filter_map(|event| match event {
            BalanceEvent::DamageApplied {
                attacker,
                victim,
                victim_kind,
                weapon,
                amount,
                shield_absorbed,
                hull_damage,
                system_hit,
            } => Some(GmActivityEntry {
                tick,
                category: GmActivityCategory::Damage,
                victim: reference(victim, names),
                source: attacker.as_deref().map(|source| reference(source, names)),
                damage: Some(GmActivityDamage {
                    victim_kind: *victim_kind,
                    weapon: weapon.clone(),
                    amount: *amount,
                    shield_absorbed: *shield_absorbed,
                    hull_damage: *hull_damage,
                    system_hit: system_hit.clone(),
                }),
            }),
            BalanceEvent::EntityDestroyed { victim, killer } => Some(GmActivityEntry {
                tick,
                category: GmActivityCategory::Destruction,
                victim: reference(victim, names),
                source: killer.as_deref().map(|source| reference(source, names)),
                damage: None,
            }),
            _ => None,
        })
        .collect();
    projected.sort_by(compare_entries);
    projected
}

#[derive(Resource)]
struct GmActivityState {
    names: BTreeMap<String, String>,
    history: GmActivityHistory,
    dirty: bool,
}

impl Default for GmActivityState {
    fn default() -> Self {
        Self {
            names: BTreeMap::new(),
            history: GmActivityHistory::new(
                crate::entities::config::GlobalConfig::default().gm_activity_history_depth as usize,
            ),
            // The first absolute payload clears any page state left by an old
            // WASM instance before this one receives an event.
            dirty: true,
        }
    }
}

pub struct GmActivityPlugin;

impl Plugin for GmActivityPlugin {
    fn build(&self, app: &mut App) {
        use crate::authoritative::{DeclareState, StateClass};

        app.init_resource::<GmActivityState>()
            .declare_state::<GmActivityState>(
                StateClass::Presentation,
                "gm-t2-map-feed-and-inspector-shell",
            )
            .add_message::<GmActivityFeedChanged>()
            .add_systems(
                FixedUpdate,
                cache_identity_directory
                    .before(crate::sim_sets::SimSet::Input)
                    .run_if(resource_exists::<BrowserGameMaster>),
            )
            .add_systems(
                FixedLast,
                collect_and_publish
                    .before(crate::sim_tick::advance_sim_tick)
                    .run_if(resource_exists::<BrowserGameMaster>),
            )
            .add_systems(
                OnEnter(GamePhase::Lobby),
                reset_on_lobby.run_if(resource_exists::<BrowserGameMaster>),
            );
    }
}

type StableIdentityQuery<'w, 's> = Query<
    'w,
    's,
    (
        Option<&'static EntityUuid>,
        Option<&'static AsteroidUuid>,
        Option<&'static EntityName>,
    ),
    Or<(With<EntityUuid>, With<AsteroidUuid>)>,
>;

/// Snapshot display identity before any fixed-tick combat producer can despawn
/// its victim. Collection prunes this cache back to live identities plus the
/// identities referenced by bounded history.
fn cache_identity_directory(mut state: ResMut<GmActivityState>, identities: StableIdentityQuery) {
    for (entity_uuid, asteroid_uuid, name) in &identities {
        // Usually an entity carries exactly one stable UUID component. Cache
        // both if an authored/lifecycle adapter ever carries both: whichever
        // existing producer chose for its BalanceEvent must resolve to the
        // same retained display identity.
        for identity in [
            entity_uuid.map(|uuid| uuid.0.as_str()),
            asteroid_uuid.map(|uuid| uuid.0.as_str()),
        ]
        .into_iter()
        .flatten()
        {
            if let Some(name) = name {
                state.names.insert(identity.to_owned(), name.0.clone());
            } else {
                state
                    .names
                    .entry(identity.to_owned())
                    .or_insert_with(|| identity.to_owned());
            }
        }
    }
}

/// Preserve GameOver for post-run facilitation, but clear both history and the
/// retained name directory at the Lobby boundary before another run can start.
fn reset_on_lobby(mut state: ResMut<GmActivityState>) {
    state.names.clear();
    state.history.clear();
    state.dirty = true;
}

/// Runs after every FixedUpdate producer and before `SimTick` advances, so all
/// rows in one batch carry the exact current tick and one canonical ordering.
fn collect_and_publish(
    mut state: ResMut<GmActivityState>,
    mut balance: MessageReader<BalanceEvent>,
    tick: Res<crate::sim_tick::SimTick>,
    world: Option<Res<crate::world::config::WorldConfig>>,
    identities: StableIdentityQuery,
    mut changed: MessageWriter<GmActivityFeedChanged>,
) {
    let capacity = world
        .as_deref()
        .map(|world| world.global.gm_activity_history_depth as usize)
        .unwrap_or_else(|| {
            crate::entities::config::GlobalConfig::default().gm_activity_history_depth as usize
        });
    if state.history.set_capacity(capacity) {
        state.dirty = true;
    }

    let events: Vec<&BalanceEvent> = balance.read().collect();
    let projected = project_balance_events(tick.0, events, &state.names);
    if state.history.append(projected) {
        state.dirty = true;
    }

    // Asteroid streaming mints fresh UUIDs, so keeping every name observed
    // before combat would make this presentation state unbounded even though
    // its rows are bounded. Commands from FixedUpdate have been applied by
    // FixedLast: retain current ECS identities and the identities needed to
    // keep retained rows readable, then discard everything else.
    let live: BTreeSet<String> = identities
        .iter()
        .flat_map(|(entity_uuid, asteroid_uuid, _)| {
            [
                entity_uuid.map(|uuid| uuid.0.clone()),
                asteroid_uuid.map(|uuid| uuid.0.clone()),
            ]
            .into_iter()
            .flatten()
        })
        .collect();
    let GmActivityState { names, history, .. } = &mut *state;
    names.retain(|identity, _| {
        live.contains(identity) || history.references_identity(identity.as_str())
    });

    if state.dirty {
        changed.write(GmActivityFeedChanged {
            payload: state.history.payload(),
        });
        state.dirty = false;
    }
}

#[cfg(test)]
#[path = "gm_activity_tests.rs"]
mod tests;
