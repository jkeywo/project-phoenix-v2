//! Peer-local bounded Game Master activity feed (issues #1297/#1298 / PRD #930).
//!
//! One presentation ring projects unconditional simulation facts and terminal
//! operational results onto the browser's existing `gm_activity` Host Channel.
//! It is not a `ServerMessage`, mesh frame, snapshot, digest, replay field, or a
//! second GM event bus.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::bounded_history::BoundedRing;
use crate::console_bridge::GmActivityFeedChanged;
use crate::core::balance::{BalanceEvent, VictimKind};
use crate::core::messages::{GamePhase, ObjectiveStatus};
use crate::entities::spawner::{EntityName, EntityUuid};
use crate::gm_projection::{BrowserGameMaster, GmEntityReference};
use crate::server_app::AsteroidUuid;

/// Complete M1 category vocabulary in canonical display order.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GmActivityCategory {
    Damage,
    Destruction,
    Objective,
    Trigger,
    RedAlert,
    Connection,
    GmAction,
}

/// Why an entity reference is linked from one row.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum GmActivityLinkRole {
    Source,
    Victim,
    Target,
    Ship,
}

/// A selectable entity reference kept separate from row semantics and filters.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmActivityLink {
    pub role: GmActivityLinkRole,
    pub entity: GmEntityReference,
}

/// Damage detail retained exactly from `BalanceEvent::DamageApplied`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct GmActivityDamage {
    pub victim_kind: VictimKind,
    pub weapon: String,
    pub amount: f32,
    pub shield_absorbed: f32,
    pub hull_damage: f32,
    pub system_hit: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GmActivityObjectiveStatus {
    Active,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmActivityObjective {
    pub objective_id: String,
    pub status: GmActivityObjectiveStatus,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmActivityTrigger {
    pub trigger_id: String,
    pub origin: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmActivityRedAlert {
    pub active: bool,
}

/// Public identity used by connection and operational rows. This never carries
/// a session token, peer id, mesh slot, or reconnect capability.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmActivityPublicIdentity {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum GmActivityConnectionRole {
    Crew,
    Spectator,
    GameMaster,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum GmActivityConnectionState {
    Connected,
    Disconnected,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmActivityConnection {
    pub identity: GmActivityPublicIdentity,
    pub role: GmActivityConnectionRole,
    pub state: GmActivityConnectionState,
    pub ship: Option<GmEntityReference>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GmActivityAction {
    SetSessionPaused {
        active: bool,
    },
    ForceStart,
    /// One authored GM-operable event was fired (issue #1301). `event` is its
    /// layer-qualified stable id; the crew see only the fictional consequence
    /// the handler produces, while the GM feed names the operator who caused it.
    FireGmEvent {
        event: String,
    },
    /// One directed world effect landed on a named entity (issue #1310).
    ///
    /// The crew never see this row: they see the ordinary damage and
    /// destruction consequences the same balance events produce for a beam
    /// hit. What this adds, on the GM's own feed, is who caused it and what
    /// the hull actually absorbed against what was asked for.
    ApplyDirectEffect {
        entity: String,
        heal: bool,
        applied_milli_hp: u32,
        discarded_milli_hp: u32,
        destroyed: bool,
    },
    /// One authored palette entry was placed on the map (issue #1305).
    /// `palette` is the `[[gm_palette]]` id — never a template path, which the
    /// browser is never handed. The crew see only the hull that arrived; the GM
    /// feed names the operator who placed it.
    SpawnPaletteEntity {
        palette: String,
    },
    /// One authored GM-operable event was paused or resumed (issue #1303).
    /// `active` is the absolute state the GM asked for, so the feed says
    /// "paused" or "resumed" rather than "toggled".
    ///
    /// A separate variant from [`Self::FireGmEvent`] even though both belong to
    /// [`crate::gm_action::GmActionKind::EventControl`]: the kind routes a
    /// result to the mission surface, while this vocabulary is the sentence a
    /// GM reads, and "fired the breach alarm" is simply not what happened.
    SetEventPaused {
        event: String,
        active: bool,
    },
    /// One authored GM-operable event had its next occurrence armed for Skip
    /// (issue #1304). `event` is the same layer-qualified stable id a Fire
    /// names — a different lever on the same control, and a different sentence,
    /// which is exactly why it is a variant rather than a flag on the Fire row.
    ///
    /// The crew never see this either, and in the strongest sense: what the GM
    /// bought is that NOTHING happens where something would have.
    ArmGmEventSkip {
        event: String,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum GmActivityActionOutcome {
    Applied,
    NoOp,
    Refused,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmActivityGmAction {
    pub operator: GmActivityPublicIdentity,
    pub correlation: String,
    pub action: GmActivityAction,
    pub outcome: GmActivityActionOutcome,
    pub reason: Option<String>,
    pub order: Option<crate::gm_action::GmActionOrder>,
}

/// Type-safe detail discriminant. The outer category makes filtering cheap;
/// strict producers and browser parsing keep it paired with the same detail.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum GmActivityDetail {
    Damage(GmActivityDamage),
    Destruction,
    Objective(GmActivityObjective),
    Trigger(GmActivityTrigger),
    RedAlert(GmActivityRedAlert),
    Connection(GmActivityConnection),
    GmAction(GmActivityGmAction),
}

/// One common tick-stamped row. `ships` is semantic scope from actual `Ship`
/// components; an empty vector is global and therefore matches only All ships.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct GmActivityEntry {
    pub tick: u64,
    pub category: GmActivityCategory,
    pub ships: Vec<GmEntityReference>,
    pub links: Vec<GmActivityLink>,
    pub detail: GmActivityDetail,
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

    pub fn set_capacity(&mut self, capacity: usize) -> bool {
        if self.entries.capacity() == capacity {
            return false;
        }
        self.entries.set_capacity(capacity);
        true
    }

    /// Repeated equal facts remain distinct occurrences. Re-sort the complete
    /// bounded timeline because PostUpdate operational facts can arrive before
    /// FixedUpdate source facts carrying the same logical tick.
    pub fn append(&mut self, entries: impl IntoIterator<Item = GmActivityEntry>) -> bool {
        let incoming = entries.into_iter().collect::<Vec<_>>();
        if incoming.is_empty() {
            return false;
        }

        let capacity = self.entries.capacity();
        let mut ordered = self
            .entries
            .iter()
            .cloned()
            .chain(incoming)
            .collect::<Vec<_>>();
        ordered.sort_by(compare_timeline_entries);
        if ordered.len() > capacity {
            ordered.drain(..ordered.len() - capacity);
        }

        self.entries.clear();
        for entry in ordered {
            self.entries.push(entry);
        }
        true
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
            entry.ships.iter().any(|ship| ship.entity_id == identity)
                || entry
                    .links
                    .iter()
                    .any(|link| link.entity.entity_id == identity)
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct IdentityRecord {
    name: String,
    is_ship: bool,
}

fn reference(identity: &str, identities: &BTreeMap<String, IdentityRecord>) -> GmEntityReference {
    GmEntityReference {
        entity_id: identity.to_owned(),
        name: identities
            .get(identity)
            .map(|record| record.name.clone())
            .unwrap_or_else(|| identity.to_owned()),
    }
}

fn linked(
    identity: &str,
    role: GmActivityLinkRole,
    identities: &BTreeMap<String, IdentityRecord>,
) -> GmActivityLink {
    GmActivityLink {
        role,
        entity: reference(identity, identities),
    }
}

fn scoped_ships(
    links: &[GmActivityLink],
    identities: &BTreeMap<String, IdentityRecord>,
) -> Vec<GmEntityReference> {
    let mut ships = BTreeMap::new();
    for link in links {
        if identities
            .get(&link.entity.entity_id)
            .is_some_and(|record| record.is_ship)
        {
            ships.insert(link.entity.entity_id.clone(), link.entity.clone());
        }
    }
    ships.into_values().collect()
}

fn entry(
    tick: u64,
    category: GmActivityCategory,
    links: Vec<GmActivityLink>,
    detail: GmActivityDetail,
    identities: &BTreeMap<String, IdentityRecord>,
) -> GmActivityEntry {
    let ships = scoped_ships(&links, identities);
    GmActivityEntry {
        tick,
        category,
        ships,
        links,
        detail,
    }
}

fn category_rank(category: GmActivityCategory) -> u8 {
    match category {
        GmActivityCategory::Damage => 0,
        GmActivityCategory::Destruction => 1,
        GmActivityCategory::Objective => 2,
        GmActivityCategory::Trigger => 3,
        GmActivityCategory::RedAlert => 4,
        GmActivityCategory::Connection => 5,
        GmActivityCategory::GmAction => 6,
    }
}

fn compare_damage(left: &GmActivityDamage, right: &GmActivityDamage) -> Ordering {
    left.victim_kind
        .as_str()
        .cmp(right.victim_kind.as_str())
        .then_with(|| left.weapon.cmp(&right.weapon))
        .then_with(|| left.amount.total_cmp(&right.amount))
        .then_with(|| left.shield_absorbed.total_cmp(&right.shield_absorbed))
        .then_with(|| left.hull_damage.total_cmp(&right.hull_damage))
        .then_with(|| left.system_hit.cmp(&right.system_hit))
}

fn compare_links(left: &[GmActivityLink], right: &[GmActivityLink]) -> Ordering {
    left.iter()
        .map(|link| (link.role, link.entity.entity_id.as_str()))
        .cmp(
            right
                .iter()
                .map(|link| (link.role, link.entity.entity_id.as_str())),
        )
}

fn link_id(entry: &GmActivityEntry, role: GmActivityLinkRole) -> Option<&str> {
    entry
        .links
        .iter()
        .find(|link| link.role == role)
        .map(|link| link.entity.entity_id.as_str())
}

fn action_key(action: &GmActivityAction) -> (u8, bool, &str) {
    match action {
        GmActivityAction::SetSessionPaused { active } => (0, *active, ""),
        GmActivityAction::ForceStart => (1, false, ""),
        // The event id is the third component so two Fires committed at the
        // same tick and order still sort deterministically by WHAT they fired.
        GmActivityAction::FireGmEvent { event } => (2, false, event.as_str()),
        // Same reason as the Fire above: the entity id disambiguates two
        // effects committed at one tick and order. `heal` rides the boolean
        // slot the pause row already uses, so damage and healing on one target
        // never collapse onto each other.
        GmActivityAction::ApplyDirectEffect { entity, heal, .. } => (3, *heal, entity.as_str()),
        GmActivityAction::SpawnPaletteEntity { palette } => (4, false, palette.as_str()),
        GmActivityAction::SetEventPaused { event, active } => (5, *active, event.as_str()),
        // Its own rank rather than the Fire's, so two rows about one event at
        // one tick and order sort by WHICH lever was pulled — the levers do
        // opposite things and their rows must not collapse onto each other.
        GmActivityAction::ArmGmEventSkip { event } => (6, false, event.as_str()),
    }
}

fn detail_rank(detail: &GmActivityDetail) -> u8 {
    match detail {
        GmActivityDetail::Damage(_) => 0,
        GmActivityDetail::Destruction => 1,
        GmActivityDetail::Objective(_) => 2,
        GmActivityDetail::Trigger(_) => 3,
        GmActivityDetail::RedAlert(_) => 4,
        GmActivityDetail::Connection(_) => 5,
        GmActivityDetail::GmAction(_) => 6,
    }
}

fn compare_details(left: &GmActivityDetail, right: &GmActivityDetail) -> Ordering {
    match (left, right) {
        (GmActivityDetail::Damage(left), GmActivityDetail::Damage(right)) => {
            compare_damage(left, right)
        }
        (GmActivityDetail::Destruction, GmActivityDetail::Destruction) => Ordering::Equal,
        (GmActivityDetail::Objective(left), GmActivityDetail::Objective(right)) => left
            .objective_id
            .cmp(&right.objective_id)
            .then_with(|| (left.status as u8).cmp(&(right.status as u8))),
        (GmActivityDetail::Trigger(left), GmActivityDetail::Trigger(right)) => left
            .trigger_id
            .cmp(&right.trigger_id)
            .then_with(|| left.origin.cmp(&right.origin)),
        (GmActivityDetail::RedAlert(left), GmActivityDetail::RedAlert(right)) => {
            left.active.cmp(&right.active)
        }
        (GmActivityDetail::Connection(left), GmActivityDetail::Connection(right)) => left
            .identity
            .id
            .cmp(&right.identity.id)
            .then_with(|| left.role.cmp(&right.role))
            .then_with(|| left.state.cmp(&right.state))
            .then_with(|| {
                left.ship
                    .as_ref()
                    .map(|ship| ship.entity_id.as_str())
                    .cmp(&right.ship.as_ref().map(|ship| ship.entity_id.as_str()))
            }),
        (GmActivityDetail::GmAction(left), GmActivityDetail::GmAction(right)) => left
            .order
            .cmp(&right.order)
            .then_with(|| left.operator.id.cmp(&right.operator.id))
            .then_with(|| left.correlation.cmp(&right.correlation))
            .then_with(|| action_key(&left.action).cmp(&action_key(&right.action)))
            .then_with(|| left.outcome.cmp(&right.outcome))
            .then_with(|| left.reason.cmp(&right.reason)),
        _ => detail_rank(left).cmp(&detail_rank(right)),
    }
}

fn compare_entries(left: &GmActivityEntry, right: &GmActivityEntry) -> Ordering {
    let category = category_rank(left.category).cmp(&category_rank(right.category));
    if category != Ordering::Equal {
        return category;
    }
    match (left.category, &left.detail, &right.detail) {
        (
            GmActivityCategory::Damage,
            GmActivityDetail::Damage(left_detail),
            GmActivityDetail::Damage(right_detail),
        ) => link_id(left, GmActivityLinkRole::Victim)
            .cmp(&link_id(right, GmActivityLinkRole::Victim))
            .then_with(|| {
                link_id(left, GmActivityLinkRole::Source)
                    .cmp(&link_id(right, GmActivityLinkRole::Source))
            })
            .then_with(|| compare_damage(left_detail, right_detail)),
        (
            GmActivityCategory::Destruction,
            GmActivityDetail::Destruction,
            GmActivityDetail::Destruction,
        ) => link_id(left, GmActivityLinkRole::Victim)
            .cmp(&link_id(right, GmActivityLinkRole::Victim))
            .then_with(|| {
                link_id(left, GmActivityLinkRole::Source)
                    .cmp(&link_id(right, GmActivityLinkRole::Source))
            }),
        (GmActivityCategory::Objective, ..) | (GmActivityCategory::Trigger, ..) => {
            compare_details(&left.detail, &right.detail)
                .then_with(|| compare_links(&left.links, &right.links))
        }
        (GmActivityCategory::RedAlert, ..) => link_id(left, GmActivityLinkRole::Ship)
            .cmp(&link_id(right, GmActivityLinkRole::Ship))
            .then_with(|| compare_details(&left.detail, &right.detail)),
        (GmActivityCategory::Connection, ..) | (GmActivityCategory::GmAction, ..) => {
            compare_details(&left.detail, &right.detail)
                .then_with(|| compare_links(&left.links, &right.links))
        }
        _ => compare_details(&left.detail, &right.detail)
            .then_with(|| compare_links(&left.links, &right.links)),
    }
}

fn compare_timeline_entries(left: &GmActivityEntry, right: &GmActivityEntry) -> Ordering {
    left.tick
        .cmp(&right.tick)
        .then_with(|| compare_entries(left, right))
}

fn objective_status(status: &ObjectiveStatus) -> GmActivityObjectiveStatus {
    match status {
        ObjectiveStatus::Active => GmActivityObjectiveStatus::Active,
        ObjectiveStatus::Completed => GmActivityObjectiveStatus::Completed,
        ObjectiveStatus::Failed => GmActivityObjectiveStatus::Failed,
    }
}

/// Project accepted unconditional source facts and impose one canonical order
/// for the current tick. Stable sorting preserves exact repeated rows.
fn project_balance_events<'a>(
    tick: u64,
    events: impl IntoIterator<Item = &'a BalanceEvent>,
    identities: &BTreeMap<String, IdentityRecord>,
    aliases: &std::collections::HashMap<String, String>,
) -> Vec<GmActivityEntry> {
    let mut projected = Vec::new();
    for event_fact in events {
        let projected_entry = match event_fact {
            BalanceEvent::DamageApplied {
                attacker,
                victim,
                victim_kind,
                weapon,
                amount,
                shield_absorbed,
                hull_damage,
                system_hit,
            } => {
                let mut links = Vec::new();
                if let Some(attacker) = attacker {
                    links.push(linked(attacker, GmActivityLinkRole::Source, identities));
                }
                links.push(linked(victim, GmActivityLinkRole::Victim, identities));
                Some(entry(
                    tick,
                    GmActivityCategory::Damage,
                    links,
                    GmActivityDetail::Damage(GmActivityDamage {
                        victim_kind: *victim_kind,
                        weapon: weapon.clone(),
                        amount: *amount,
                        shield_absorbed: *shield_absorbed,
                        hull_damage: *hull_damage,
                        system_hit: system_hit.clone(),
                    }),
                    identities,
                ))
            }
            BalanceEvent::EntityDestroyed { victim, killer } => {
                let mut links = Vec::new();
                if let Some(killer) = killer {
                    links.push(linked(killer, GmActivityLinkRole::Source, identities));
                }
                links.push(linked(victim, GmActivityLinkRole::Victim, identities));
                Some(entry(
                    tick,
                    GmActivityCategory::Destruction,
                    links,
                    GmActivityDetail::Destruction,
                    identities,
                ))
            }
            BalanceEvent::ObjectiveChanged {
                objective_id,
                status,
                targets,
            } => {
                let links = targets
                    .iter()
                    .map(|target| aliases.get(target).map_or(target.as_str(), String::as_str))
                    .map(|target| linked(target, GmActivityLinkRole::Target, identities))
                    .collect();
                Some(entry(
                    tick,
                    GmActivityCategory::Objective,
                    links,
                    GmActivityDetail::Objective(GmActivityObjective {
                        objective_id: objective_id.clone(),
                        status: objective_status(status),
                    }),
                    identities,
                ))
            }
            BalanceEvent::TriggerFired {
                trigger_id,
                origin,
                entity: involved,
            } => {
                let links = involved
                    .iter()
                    .map(|identity| linked(identity, GmActivityLinkRole::Target, identities))
                    .collect();
                Some(entry(
                    tick,
                    GmActivityCategory::Trigger,
                    links,
                    GmActivityDetail::Trigger(GmActivityTrigger {
                        trigger_id: trigger_id.clone(),
                        origin: origin.clone(),
                    }),
                    identities,
                ))
            }
            BalanceEvent::RedAlertChanged { ship, on } => Some(entry(
                tick,
                GmActivityCategory::RedAlert,
                vec![linked(ship, GmActivityLinkRole::Ship, identities)],
                GmActivityDetail::RedAlert(GmActivityRedAlert { active: *on }),
                identities,
            )),
            _ => None,
        };
        if let Some(projected_entry) = projected_entry {
            projected.push(projected_entry);
        }
    }
    projected.sort_by(compare_entries);
    projected
}

#[derive(Resource)]
pub(crate) struct GmActivityState {
    identities: BTreeMap<String, IdentityRecord>,
    /// Stable public ship identities keyed by the private fleet topology only
    /// inside this projection. The slot never crosses the Host Channel; it is
    /// just the deterministic join from `FleetRoster` presence to the actual
    /// semantic `Ship` entity every simulation peer spawned.
    fleet_ships: BTreeMap<crate::command_admission::HostSlot, GmEntityReference>,
    history: GmActivityHistory,
    dirty: bool,
    presence_seeded: bool,
    presence: BTreeMap<String, GmActivityConnection>,
    action_seeded: bool,
    observed_actions: BTreeSet<(String, String)>,
    observed_start_results: BTreeSet<(String, String)>,
}

impl Default for GmActivityState {
    fn default() -> Self {
        Self {
            identities: BTreeMap::new(),
            fleet_ships: BTreeMap::new(),
            history: GmActivityHistory::new(
                crate::entities::config::GlobalConfig::default().gm_activity_history_depth as usize,
            ),
            dirty: true,
            presence_seeded: false,
            presence: BTreeMap::new(),
            action_seeded: false,
            observed_actions: BTreeSet::new(),
            observed_start_results: BTreeSet::new(),
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
                collect_fixed_activity
                    .before(crate::sim_tick::advance_sim_tick)
                    .run_if(resource_exists::<BrowserGameMaster>),
            )
            .add_systems(
                PostUpdate,
                publish_frame_activity.run_if(resource_exists::<BrowserGameMaster>),
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
        Has<crate::server_app::Ship>,
    ),
    Or<(With<EntityUuid>, With<AsteroidUuid>)>,
>;

type LocalShipIdentityQuery<'w, 's> = Query<
    'w,
    's,
    (&'static EntityUuid, Option<&'static EntityName>),
    (
        With<crate::server_app::Ship>,
        With<crate::server_app::LocalShip>,
    ),
>;

type FleetShipIdentityQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static crate::lockstep::FleetSlotOf,
        &'static EntityUuid,
        Option<&'static EntityName>,
    ),
    With<crate::server_app::Ship>,
>;

fn local_ship_reference(local_ship: &LocalShipIdentityQuery<'_, '_>) -> Option<GmEntityReference> {
    local_ship
        .iter()
        .next()
        .map(|(uuid, name)| GmEntityReference {
            entity_id: uuid.0.clone(),
            name: name
                .map(|name| name.0.clone())
                .unwrap_or_else(|| uuid.0.clone()),
        })
}

fn refresh_fleet_ship_directory(
    state: &mut GmActivityState,
    fleet_ships: &FleetShipIdentityQuery<'_, '_>,
) {
    for (slot, uuid, name) in fleet_ships {
        state.fleet_ships.insert(
            slot.0,
            GmEntityReference {
                entity_id: uuid.0.clone(),
                name: name
                    .map(|name| name.0.clone())
                    .unwrap_or_else(|| uuid.0.clone()),
            },
        );
    }
}

fn refresh_identity_directory(
    state: &mut GmActivityState,
    identities: &StableIdentityQuery<'_, '_>,
) {
    for (entity_uuid, asteroid_uuid, name, is_ship) in identities {
        for identity in [
            entity_uuid.map(|uuid| uuid.0.as_str()),
            asteroid_uuid.map(|uuid| uuid.0.as_str()),
        ]
        .into_iter()
        .flatten()
        {
            state.identities.insert(
                identity.to_owned(),
                IdentityRecord {
                    name: name
                        .map(|name| name.0.clone())
                        .unwrap_or_else(|| identity.to_owned()),
                    is_ship,
                },
            );
        }
    }
}

fn cache_identity_directory(mut state: ResMut<GmActivityState>, identities: StableIdentityQuery) {
    refresh_identity_directory(&mut state, &identities);
}

fn reset_on_lobby(mut state: ResMut<GmActivityState>) {
    state.identities.clear();
    state.fleet_ships.clear();
    state.history.clear();
    state.presence_seeded = false;
    state.presence.clear();
    state.action_seeded = false;
    state.observed_actions.clear();
    state.observed_start_results.clear();
    state.dirty = true;
}

/// A snapshot/join installs an already-observed terminal GM log into a live
/// app. Re-seed the presentation cursor on its next PostUpdate so historical
/// rows do not masquerade as actions taken after restore.
pub(crate) fn rebase_after_restore(world: &mut World) {
    let observed_start_results = world
        .get_resource::<crate::lobby::StartGrantResults>()
        .map(|results| {
            results
                .iter()
                .filter_map(|result| {
                    Some((
                        result.operator_id.as_ref()?.clone(),
                        result.grant_id.as_ref()?.clone(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let Some(mut state) = world.get_resource_mut::<GmActivityState>() else {
        return;
    };
    state.action_seeded = false;
    state.observed_actions.clear();
    state.observed_start_results = observed_start_results;
}

/// Fixed facts and fixed-step presence changes are collected before `SimTick`
/// advances. Publication waits for PostUpdate so every step and frame-driven
/// operational result shares one absolute Host Channel replacement per frame.
fn collect_fixed_activity(
    mut state: ResMut<GmActivityState>,
    mut balance: MessageReader<BalanceEvent>,
    tick: Res<crate::sim_tick::SimTick>,
    world: Option<Res<crate::world::config::WorldConfig>>,
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    identities: StableIdentityQuery,
    sessions: Option<Res<crate::lobby::Sessions>>,
    gm_roster: Option<Res<crate::gm_roster::GmRoster>>,
    fleet_roster: Option<Res<crate::lockstep::FleetRoster>>,
    fleet: Option<Res<crate::lockstep::FleetLockstep>>,
    fleet_ship_identities: FleetShipIdentityQuery,
    local_ship: LocalShipIdentityQuery,
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

    // The pre-Input cache preserves the identity of entities despawned during
    // this tick; this second pass catches entities spawned after that cache so
    // semantic ship scope is still based on their actual `Ship` component.
    refresh_identity_directory(&mut state, &identities);
    refresh_fleet_ship_directory(&mut state, &fleet_ship_identities);
    let events: Vec<&BalanceEvent> = balance.read().collect();
    let empty_aliases = std::collections::HashMap::new();
    let aliases = runtime
        .as_deref()
        .map_or(&empty_aliases, |runtime| &runtime.name_to_uuid);
    let projected = project_balance_events(tick.0, events, &state.identities, aliases);
    // Presence is sampled at every completed fixed step, not reconstructed
    // from PostUpdate's continuation tick. This preserves each source tick
    // when one rendered frame spends multiple fixed steps.
    let presence = current_presence(
        sessions.as_deref(),
        gm_roster.as_deref(),
        local_ship_reference(&local_ship),
        fleet_roster.as_deref(),
        fleet.as_deref(),
        &state.fleet_ships,
    );
    let connections = connection_entries(tick.0, &mut state, presence);
    if state
        .history
        .append(projected.into_iter().chain(connections))
    {
        state.dirty = true;
    }

    let live: BTreeSet<String> = identities
        .iter()
        .flat_map(|(entity_uuid, asteroid_uuid, _, _)| {
            [
                entity_uuid.map(|uuid| uuid.0.clone()),
                asteroid_uuid.map(|uuid| uuid.0.clone()),
            ]
            .into_iter()
            .flatten()
        })
        .collect();
    let GmActivityState {
        identities,
        history,
        ..
    } = &mut *state;
    identities.retain(|identity, _| {
        live.contains(identity) || history.references_identity(identity.as_str())
    });
}

fn current_presence(
    sessions: Option<&crate::lobby::Sessions>,
    gm_roster: Option<&crate::gm_roster::GmRoster>,
    local_ship: Option<GmEntityReference>,
    fleet_roster: Option<&crate::lockstep::FleetRoster>,
    fleet: Option<&crate::lockstep::FleetLockstep>,
    fleet_ships: &BTreeMap<crate::command_admission::HostSlot, GmEntityReference>,
) -> BTreeMap<String, GmActivityConnection> {
    let mut current = BTreeMap::new();
    if let (Some(fleet_roster), Some(_fleet)) = (fleet_roster, fleet) {
        // `Sessions` is deliberately local to one ship host. The frozen roster
        // is the replicated crewing authority every ship peer and rendererless
        // GM shares, while `FleetSlotOf` joins each private topology row to its
        // actual public semantic Ship identity. Host-loss clears that frozen
        // crew at its agreed fixed tick; omission below then becomes one
        // disconnected edge through `connection_entries`. A later restored
        // crew becomes a connected edge by the same fold.
        //
        // One row represents the crew cohort carried by that ship peer;
        // neither the slot, station/rating tuples, nor any transport/session
        // credential enters it.
        for fleet_ship in fleet_roster
            .ships()
            .iter()
            .filter(|ship| !ship.crew.is_empty())
        {
            let Some(ship) = fleet_ships.get(&fleet_ship.host).cloned() else {
                continue;
            };
            let id = format!("crew:{}", ship.entity_id);
            current.insert(
                format!("fleet:{id}"),
                GmActivityConnection {
                    identity: GmActivityPublicIdentity {
                        id,
                        name: ship.name.clone(),
                    },
                    role: GmActivityConnectionRole::Crew,
                    state: GmActivityConnectionState::Connected,
                    ship: Some(ship),
                },
            );
        }
    } else if let Some(sessions) = sessions {
        // Standalone fallback: without a deterministic fleet topology this App
        // can only truthfully project its own crew star.
        for (index, player) in sessions.0.players().iter().enumerate() {
            let id = format!("crew-{}", index + 1);
            let role = if player.spectator {
                GmActivityConnectionRole::Spectator
            } else {
                GmActivityConnectionRole::Crew
            };
            current.insert(
                format!("crew:{id}"),
                GmActivityConnection {
                    identity: GmActivityPublicIdentity {
                        id,
                        name: player.name.clone(),
                    },
                    role,
                    state: if player.connected {
                        GmActivityConnectionState::Connected
                    } else {
                        GmActivityConnectionState::Disconnected
                    },
                    ship: if player.spectator {
                        None
                    } else {
                        local_ship.clone()
                    },
                },
            );
        }
    }
    if let Some(gm_roster) = gm_roster {
        for operator in gm_roster.operators() {
            current.insert(
                format!("gm:{}", operator.id),
                GmActivityConnection {
                    identity: GmActivityPublicIdentity {
                        id: operator.id.clone(),
                        name: operator.name.clone(),
                    },
                    role: GmActivityConnectionRole::GameMaster,
                    state: if operator.connected {
                        GmActivityConnectionState::Connected
                    } else {
                        GmActivityConnectionState::Disconnected
                    },
                    ship: None,
                },
            );
        }
    }
    current
}

fn connection_entries(
    tick: u64,
    state: &mut GmActivityState,
    current: BTreeMap<String, GmActivityConnection>,
) -> Vec<GmActivityEntry> {
    if !state.presence_seeded {
        state.presence_seeded = true;
        state.presence = current;
        return Vec::new();
    }

    let mut changed = Vec::new();
    for (key, connection) in &current {
        if state
            .presence
            .get(key)
            .is_none_or(|previous| previous != connection)
        {
            changed.push(connection.clone());
        }
    }
    for (key, previous) in &state.presence {
        if !current.contains_key(key) && previous.state != GmActivityConnectionState::Disconnected {
            let mut disconnected = previous.clone();
            disconnected.state = GmActivityConnectionState::Disconnected;
            changed.push(disconnected);
        }
    }
    state.presence = current;

    let mut entries: Vec<_> = changed
        .into_iter()
        .map(|connection| {
            let links = connection
                .ship
                .as_ref()
                .map(|ship| {
                    vec![GmActivityLink {
                        role: GmActivityLinkRole::Ship,
                        entity: ship.clone(),
                    }]
                })
                .unwrap_or_default();
            GmActivityEntry {
                tick,
                category: GmActivityCategory::Connection,
                ships: connection.ship.iter().cloned().collect(),
                links,
                detail: GmActivityDetail::Connection(connection),
            }
        })
        .collect();
    entries.sort_by(compare_entries);
    entries
}

fn gm_operator(
    operator_id: &str,
    roster: Option<&crate::gm_roster::GmRoster>,
) -> GmActivityPublicIdentity {
    GmActivityPublicIdentity {
        id: operator_id.to_owned(),
        name: roster
            .and_then(|roster| {
                roster
                    .operators()
                    .iter()
                    .find(|operator| operator.id == operator_id)
            })
            .map(|operator| operator.name.clone())
            .unwrap_or_else(|| operator_id.to_owned()),
    }
}

fn gm_outcome(outcome: crate::gm_action::GmActionOutcome) -> Option<GmActivityActionOutcome> {
    match outcome {
        crate::gm_action::GmActionOutcome::Pending => None,
        crate::gm_action::GmActionOutcome::Applied => Some(GmActivityActionOutcome::Applied),
        crate::gm_action::GmActionOutcome::NoOp => Some(GmActivityActionOutcome::NoOp),
        crate::gm_action::GmActionOutcome::Refused => Some(GmActivityActionOutcome::Refused),
    }
}

fn refusal_reason(reason: crate::gm_action::GmActionRefusalReason) -> &'static str {
    use crate::gm_action::GmActionRefusalReason as Reason;
    match reason {
        Reason::NotInFleet => "not-in-fleet",
        Reason::NotGameMaster => "not-game-master",
        Reason::OperatorMismatch => "operator-mismatch",
        Reason::InvalidOperator => "invalid-operator",
        Reason::InvalidAction => "invalid-action",
        Reason::UnknownStation => "unknown-station",
        Reason::StationNotBackfill => "station-not-backfill",
        Reason::StationNotPuppeted => "station-not-puppeted",
        Reason::SystemOutsideStation => "system-outside-station",
        Reason::SystemUnavailable => "system-unavailable",
        Reason::SystemRefused => "system-refused",
        Reason::OriginMismatch => "origin-mismatch",
        Reason::ConflictingGrant => "conflicting-grant",
        Reason::NonContiguousSequence => "non-contiguous-sequence",
        Reason::JournalFull => "journal-full",
        Reason::WrongPhase => "wrong-phase",
        Reason::UnreadableRequest => "unreadable-request",
        Reason::UnknownGmEvent => "unknown-gm-event",
        Reason::UnknownEntity => "unknown-entity",
        Reason::TargetNotDamageable => "target-not-damageable",
        Reason::UnknownGmPaletteEntry => "unknown-gm-palette-entry",
        Reason::WorldUnavailable => "world-unavailable",
    }
}

fn start_reason(reason: crate::lobby::start_policy::StartGrantReason) -> &'static str {
    use crate::lobby::start_policy::StartGrantReason as Reason;
    match reason {
        Reason::AlreadyStarted => "already-started",
        Reason::ValidationFailed => "validation-failed",
        Reason::ReadinessChanged => "readiness-changed",
        Reason::GmNotConnected => "gm-not-connected",
        Reason::FleetNotManaged => "fleet-not-managed",
        Reason::InvalidGrant => "invalid-grant",
        Reason::UnauthorizedGrant => "unauthorized-grant",
        Reason::UnsafeApplyTick => "unsafe-apply-tick",
        Reason::ConflictingGrant => "conflicting-grant",
        Reason::MissedApplyTick => "missed-apply-tick",
    }
}

fn terminal_action_entries(
    state: &mut GmActivityState,
    log: Option<&crate::gm_action::GmActionLog>,
    refusals: Option<&crate::gm_action::LocalGmActionRefusals>,
    starts: Option<&crate::lobby::StartGrantResults>,
    roster: Option<&crate::gm_roster::GmRoster>,
) -> Vec<GmActivityEntry> {
    let mut durable = Vec::new();
    if let Some(log) = log {
        durable.extend(log.entries().iter().cloned());
    }
    if let Some(refusals) = refusals {
        durable.extend(refusals.entries().iter().cloned());
    }
    // A Station command is only operational history after its authentic System
    // consumer has settled it. Keeping provisional Pending facts out of both
    // the feed and the observed-key set lets the later terminal result surface
    // exactly once under the same correlation.
    durable.retain(|fact| fact.outcome != crate::gm_action::GmActionOutcome::Pending);
    durable.sort_by(|left, right| {
        (
            left.tick,
            left.order,
            &left.operator_id,
            left.correlation.as_str(),
        )
            .cmp(&(
                right.tick,
                right.order,
                &right.operator_id,
                right.correlation.as_str(),
            ))
    });
    let current_keys: BTreeSet<_> = durable
        .iter()
        .map(|fact| {
            (
                fact.operator_id.clone(),
                fact.correlation.as_str().to_owned(),
            )
        })
        .collect();
    if !state.action_seeded {
        // Startup and `rebase_after_restore` both seed from the complete
        // terminal surface. Subsequent bounded local-result rotation is not a
        // restore and therefore cannot suppress a genuinely new result.
        state.action_seeded = true;
        state.observed_actions = current_keys;
        durable.clear();
    } else {
        durable.retain(|fact| {
            state.observed_actions.insert((
                fact.operator_id.clone(),
                fact.correlation.as_str().to_owned(),
            ))
        });
    }

    let mut entries: Vec<GmActivityEntry> = durable
        .into_iter()
        .filter_map(|fact| {
            Some(GmActivityEntry {
                tick: fact.tick,
                category: GmActivityCategory::GmAction,
                ships: Vec::new(),
                links: Vec::new(),
                detail: GmActivityDetail::GmAction(GmActivityGmAction {
                    operator: gm_operator(&fact.operator_id, roster),
                    correlation: fact.correlation.as_str().to_owned(),
                    // The event-control family names WHAT it did to WHICH
                    // event, so it reads the durable fact's VERB and not just
                    // its kind (issue #1303): the kind is the routing family
                    // Fire and Pause share, and folding on it alone would
                    // publish every Pause and Resume as "fired {event}".
                    action: match (fact.action_kind, fact.verb) {
                        // The directed world-effect family names WHAT it hit
                        // and WHAT the hull did with it. A fact whose result
                        // carries no resolved effect (a refusal settled before
                        // any hull was read) still renders, with zeroes, rather
                        // than dropping the operator's row.
                        (crate::gm_action::GmActionKind::DirectEffect, _) => {
                            let effect = fact.effect;
                            GmActivityAction::ApplyDirectEffect {
                                entity: fact.target.clone()?,
                                heal: effect.is_some_and(|effect| {
                                    effect.kind == crate::gm_effect::GmDirectEffectKind::Heal
                                }),
                                applied_milli_hp: effect
                                    .map_or(0, |effect| effect.applied_milli_hp),
                                discarded_milli_hp: effect
                                    .map_or(0, |effect| effect.discarded_milli_hp),
                                destroyed: effect.is_some_and(|effect| effect.destroyed),
                            }
                        }
                        (
                            crate::gm_action::GmActionKind::EventControl,
                            Some(crate::gm_action::GmEventVerb::Fire),
                        ) => GmActivityAction::FireGmEvent {
                            // Every producer of an event-control fact attaches
                            // the qualified id and `validate_fleet_frame`
                            // refuses a replicated refusal without one, so
                            // `None` is unreachable. Dropping that row rather
                            // than publishing an empty id keeps one
                            // hypothetical hole from making the whole absolute
                            // page unparseable for every GM's feed.
                            event: fact.target.clone()?,
                        },
                        (
                            crate::gm_action::GmActionKind::EventControl,
                            Some(crate::gm_action::GmEventVerb::Pause),
                        ) => GmActivityAction::SetEventPaused {
                            event: fact.target.clone()?,
                            active: fact.requested_active,
                        },
                        // An event-control fact with no verb pulled the Skip
                        // lever instead (issue #1304): `verb` covers only Fire
                        // and Pause, so `lever` is the field that names the
                        // rest of the family. A fact naming neither is the
                        // same hypothetical hole as one with no target, and is
                        // dropped rather than rendered under a lever nobody
                        // pulled.
                        (crate::gm_action::GmActionKind::EventControl, None) => {
                            match fact.lever {
                                Some(crate::gm_event::GmEventLever::SkipNext) => {
                                    GmActivityAction::ArmGmEventSkip {
                                        event: fact.target.clone()?,
                                    }
                                }
                                None => return None,
                            }
                        }
                        // Same rule as the event family: every producer of a
                        // world-spawn fact attaches the palette id and
                        // `validate_fleet_frame` refuses a replicated refusal
                        // without one, so `None` drops this row rather than
                        // publishing a placement of the empty id.
                        (crate::gm_action::GmActionKind::WorldSpawn, _) => {
                            GmActivityAction::SpawnPaletteEntity {
                                palette: fact.target.clone()?,
                            }
                        }
                        _ => GmActivityAction::SetSessionPaused {
                            active: fact.requested_active,
                        },
                    },
                    outcome: gm_outcome(fact.outcome)?,
                    reason: fact.reason.map(refusal_reason).map(str::to_owned),
                    order: fact.order,
                }),
            })
        })
        .collect();

    if let Some(starts) = starts {
        for result in starts.iter() {
            let (Some(operator_id), Some(correlation)) =
                (result.operator_id.as_deref(), result.grant_id.as_deref())
            else {
                continue;
            };
            if !state
                .observed_start_results
                .insert((operator_id.to_owned(), correlation.to_owned()))
            {
                continue;
            }
            let outcome = match result.status {
                crate::lobby::start_policy::StartGrantStatus::Applied => {
                    GmActivityActionOutcome::Applied
                }
                crate::lobby::start_policy::StartGrantStatus::NoOp => GmActivityActionOutcome::NoOp,
                crate::lobby::start_policy::StartGrantStatus::Refused => {
                    GmActivityActionOutcome::Refused
                }
            };
            entries.push(GmActivityEntry {
                tick: result.tick,
                category: GmActivityCategory::GmAction,
                ships: Vec::new(),
                links: Vec::new(),
                detail: GmActivityDetail::GmAction(GmActivityGmAction {
                    operator: gm_operator(operator_id, roster),
                    correlation: correlation.to_owned(),
                    action: GmActivityAction::ForceStart,
                    outcome,
                    reason: result.reason.map(start_reason).map(str::to_owned),
                    order: None,
                }),
            });
        }
    }
    entries.sort_by(compare_entries);
    entries
}

/// Frame-driven operational fan-out and the single publication site. It runs
/// before the browser drains `StartGrantResults`, and while virtual time is
/// paused, so Pause/Resume/refusal rows cannot disappear behind FixedUpdate.
pub(crate) fn publish_frame_activity(
    mut state: ResMut<GmActivityState>,
    tick: Option<Res<crate::sim_tick::SimTick>>,
    sessions: Option<Res<crate::lobby::Sessions>>,
    gm_roster: Option<Res<crate::gm_roster::GmRoster>>,
    fleet_roster: Option<Res<crate::lockstep::FleetRoster>>,
    fleet: Option<Res<crate::lockstep::FleetLockstep>>,
    log: Option<Res<crate::gm_action::GmActionLog>>,
    refusals: Option<Res<crate::gm_action::LocalGmActionRefusals>>,
    starts: Option<Res<crate::lobby::StartGrantResults>>,
    fleet_ship_identities: FleetShipIdentityQuery,
    local_ship: LocalShipIdentityQuery,
    mut changed: MessageWriter<GmActivityFeedChanged>,
) {
    let now = tick.as_deref().map_or(0, |tick| tick.0);
    refresh_fleet_ship_directory(&mut state, &fleet_ship_identities);
    let local_ship = local_ship_reference(&local_ship);
    let presence = current_presence(
        sessions.as_deref(),
        gm_roster.as_deref(),
        local_ship,
        fleet_roster.as_deref(),
        fleet.as_deref(),
        &state.fleet_ships,
    );
    let connection_entries = connection_entries(now, &mut state, presence);
    let action_entries = terminal_action_entries(
        &mut state,
        log.as_deref(),
        refusals.as_deref(),
        starts.as_deref(),
        gm_roster.as_deref(),
    );
    if state
        .history
        .append(connection_entries.into_iter().chain(action_entries))
    {
        state.dirty = true;
    }

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
