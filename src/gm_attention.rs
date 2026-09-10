//! Peer-local Game Master attention queue (issue #1433 / PRD #1419 M4).
//!
//! One advisory projection answers "what is waiting for a Game Master right
//! now". It reads facts the simulation already owns — today the ordinary Comms
//! inbox and the authored `[[gm_comms_route]]` table — and publishes them onto
//! the page-local `gm_attention` Host Channel.
//!
//! # What this is NOT
//!
//! It is not a `ServerMessage`, a mesh frame, a snapshot field, a digest fold,
//! a replay record, or a second GM event bus. Nothing here reaches
//! [`crate::gm_action::GmActionJournal`], and nothing here mutates the world:
//! an occurrence appears because a condition holds and disappears because it
//! stopped holding. Reading, filtering, holding and snoozing all happen in the
//! operator's own browser (`gui/gm-attention-panel.js`) and never cross back.
//!
//! # Occurrence identity
//!
//! An occurrence's id is derived from the durable identity of the thing that is
//! waiting — for pending Comms, the `CommsMessage` id the world minted. That is
//! what makes the two lifecycle rules fall out for free: while the condition
//! holds the row keeps one identity (so a browser can hold a snooze, a focus
//! ring or a reading position against it), and a *recurrence* — a second hail
//! after the first was answered — is a different message and therefore a fresh
//! occurrence that no stale snooze can hide.
//!
//! # Bands
//!
//! Three bands, `Urgent`/`Attention`/`Background`. Pending Comms default to
//! `Attention`; a scenario author may say otherwise on the route the sender
//! speaks through ([`GmCommsRoute::attention_band`]). The band is GM-facing
//! triage only: it does not touch `CommsPriority`, delivery, routing or
//! anything a crew console renders.

use std::collections::BTreeMap;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::comms::server::CommsInboxRes;
use crate::console_bridge::GmAttentionChanged;
use crate::entities::spawner::{EntityName, EntityUuid};
use crate::gm_projection::GmEntityReference;
use crate::world::config::WorldConfig;

/// Presentation bound on one published queue. A GM cannot read past this many
/// rows in a live session, and an unbounded projection is an unbounded page.
pub const MAX_GM_ATTENTION_OCCURRENCES: usize = 128;

/// The complete authored band vocabulary, in queue order.
///
/// Deliberately closed: an author names one of these three or the world fails
/// to load. There is no numerical score to reverse-engineer and no fourth band
/// a mod pack can invent.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum GmAttentionBand {
    Urgent,
    Attention,
    Background,
}

impl GmAttentionBand {
    /// The authored spelling, which is also the wire spelling.
    pub fn as_authored(self) -> &'static str {
        match self {
            Self::Urgent => "urgent",
            Self::Attention => "attention",
            Self::Background => "background",
        }
    }

    /// Parse one authored override. Exact, lower-case, no aliases: a world that
    /// says `Urgent` or `critical` is a world whose author believed something
    /// this build does not do, and guessing on their behalf is how a scenario
    /// ships with a priority nobody chose.
    pub fn from_authored(value: &str) -> Option<Self> {
        match value {
            "urgent" => Some(Self::Urgent),
            "attention" => Some(Self::Attention),
            "background" => Some(Self::Background),
            _ => None,
        }
    }

    /// Every band an author may write, for an error message that tells them
    /// what to write instead.
    pub fn authored_vocabulary() -> String {
        [Self::Urgent, Self::Attention, Self::Background]
            .iter()
            .map(|band| format!("'{}'", band.as_authored()))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Why a row is in the queue. One variant today; `#1437`'s technical banners
/// are deliberately NOT a category — see the module note on the banner seam in
/// `gui/gm-attention-panel.js`.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum GmAttentionCategory {
    PendingComms,
}

/// The short human reason, as a String Table id plus its runtime parameters.
///
/// Never prose: the projection has no locale, and the values are themselves
/// authored ids (entity names) the page resolves at its own presentation
/// boundary.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmAttentionReason {
    pub id: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, String>,
}

/// Where activating a row takes the operator. Every field names something that
/// already exists on the GM desk — an authored Comms route, a ship, a sender —
/// so "open" is a navigation, never a new action surface.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmAttentionTarget {
    /// The authored `[[gm_comms_route]]` id that already speaks as this sender.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
    /// The ship whose console is waiting. `None` for legacy fleet-wide traffic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ship: Option<GmEntityReference>,
    /// The fictional speaker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender: Option<GmEntityReference>,
    /// The conversation thread, so a later surface can open the exact exchange.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<String>,
}

/// One thing waiting for a Game Master.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmAttentionOccurrence {
    /// Stable while the condition holds; a recurrence mints a different one.
    pub id: String,
    pub category: GmAttentionCategory,
    pub band: GmAttentionBand,
    /// The simulation tick this peer first observed the condition.
    pub first_seen_tick: u64,
    /// Real milliseconds elapsed since that first observation, sampled as this
    /// payload was built. Real time, not simulation time: it keeps running
    /// while the world is paused, which is exactly the basis a GM ages a queue
    /// on and the basis the personal snooze expires on.
    pub age_ms: u64,
    pub reason: GmAttentionReason,
    #[serde(default)]
    pub target: GmAttentionTarget,
}

/// The absolute queue, ordered band-agnostically by age then id. The page
/// groups it into bands; publishing one ordered list keeps the tie-break in one
/// place rather than three.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmAttentionProjection {
    pub occurrences: Vec<GmAttentionOccurrence>,
}

/// When this peer first saw one occurrence, in both bases.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FirstSeen {
    tick: u64,
    real_ms: u64,
}

/// This peer's own attention bookkeeping. `Presentation`: derived entirely from
/// the Comms inbox and the authored route table, both already classified, and
/// read by nothing authoritative.
#[derive(Resource, Default)]
pub struct GmAttentionState {
    first_seen: BTreeMap<String, FirstSeen>,
    last: Option<GmAttentionProjection>,
}

impl GmAttentionState {
    /// The projection this peer last published, for tests and for a late
    /// mount that needs the current queue without waiting for a change.
    pub fn last(&self) -> Option<&GmAttentionProjection> {
        self.last.as_ref()
    }
}

/// Is this inbox message actually waiting on a human decision?
///
/// Three conditions, and each one is a lifecycle exit:
/// - `is_orphaned` — the sender left; the conversation was withdrawn.
/// - `selected_response` — somebody answered; it resolved.
/// - no `responses` — nothing to answer yet. An empty response list is the
///   `…` follow-up placeholder while the other side is still speaking, not a
///   demand on the crew, and a literal GM transmission never grows one.
///
/// A message leaving the inbox entirely (a Comms officer's `ClearComms`, a
/// world-layer unload) removes it from the walk below and so also removes the
/// occurrence, without needing a fourth rule here.
fn pending(message: &crate::core::messages::CommsMessage) -> bool {
    !message.is_orphaned && message.selected_response.is_none() && !message.responses.is_empty()
}

/// The authored route this sender already speaks through, if any.
///
/// First authored match wins, so a world that lists one speaker on several
/// routes gets a stable answer rather than an order-of-iteration one. The
/// world's own author order is the tie-break, which is the one a scenario
/// writer can see and change.
fn route_for<'a>(
    world: Option<&'a WorldConfig>,
    sender_name: Option<&str>,
) -> Option<&'a crate::gm_comms::GmCommsRoute> {
    let name = sender_name?;
    world?
        .gm_comms_routes
        .iter()
        .find(|route| route.senders.iter().any(|s| s == name))
}

/// Band for one pending Comms occurrence: the author's choice on the route that
/// speaks as this sender, else the system default.
fn band_for(route: Option<&crate::gm_comms::GmCommsRoute>) -> GmAttentionBand {
    route
        .and_then(|route| route.attention_band.as_deref())
        .and_then(GmAttentionBand::from_authored)
        .unwrap_or(GmAttentionBand::Attention)
}

/// The String Table id a pending Comms row addressed at a single ship explains
/// itself with. Takes `{sender}` and `{ship}`.
pub const PENDING_COMMS_REASON: &str = "server.gm.attention.reason.pending_comms";

/// The String Table id a pending Comms row with no single recipient ship
/// explains itself with. Takes `{sender}` alone.
///
/// Most real traffic lands here: only a GM's own `TransmitComms` addresses a
/// dialogue at named hulls, while a world-authored `open_comms` and the crew's
/// own hails carry no `recipient_ship` at all. Reusing the addressed sentence
/// for those would interpolate an empty `{ship}` and put "Cordon Control is
/// waiting on ." on the GM desk, so the fleet-wide case gets its own sentence
/// and its own parameter set rather than a blank.
pub const PENDING_COMMS_FLEET_REASON: &str = "server.gm.attention.reason.pending_comms_fleet";

fn reference(uuid: &str, names: &BTreeMap<String, String>) -> GmEntityReference {
    GmEntityReference {
        entity_id: uuid.to_string(),
        name: names.get(uuid).cloned().unwrap_or_else(|| uuid.to_string()),
    }
}

/// One occurrence before this peer's own first-seen bookkeeping is attached.
#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingRow {
    id: String,
    category: GmAttentionCategory,
    band: GmAttentionBand,
    reason: GmAttentionReason,
    target: GmAttentionTarget,
}

/// Build the queue from the live inbox. Pure over its inputs so the ordering
/// and lifecycle rules are testable without a `World`.
fn collect(
    world: Option<&WorldConfig>,
    inbox: &crate::console::comms::inbox::CommsInbox,
    names: &BTreeMap<String, String>,
) -> Vec<PendingRow> {
    let mut rows = Vec::new();
    for message in inbox.iter() {
        if !pending(message) {
            continue;
        }
        let sender_name = names.get(&message.sender_uuid).map(String::as_str);
        let route = route_for(world, sender_name);
        let sender = reference(&message.sender_uuid, names);
        let ship = message
            .recipient_ship
            .as_ref()
            .map(|ship| reference(&ship.0, names));
        let mut params = BTreeMap::new();
        params.insert("sender".to_string(), sender.name.clone());
        // Two sentences, not one sentence with an optional blank: a hail with
        // no recipient ship is waiting on the fleet, and saying so is not the
        // same as saying it is waiting on nobody.
        let reason_id = match ship.as_ref() {
            Some(ship) => {
                params.insert("ship".to_string(), ship.name.clone());
                PENDING_COMMS_REASON
            }
            None => PENDING_COMMS_FLEET_REASON,
        };
        rows.push(PendingRow {
            id: format!("comms:{}", message.id),
            category: GmAttentionCategory::PendingComms,
            band: band_for(route),
            reason: GmAttentionReason {
                id: reason_id.to_string(),
                params,
            },
            target: GmAttentionTarget {
                route: route.map(|route| route.id.clone()),
                ship,
                sender: Some(sender),
                conversation: (!message.thread_id.is_empty()).then(|| message.thread_id.clone()),
            },
        });
    }
    rows
}

/// Publish the absolute attention queue onto the page-local Host Channel when
/// it changes. Never emits an unchanged payload: a GM desk that repainted a
/// held list sixty times a second would defeat the reading stability this whole
/// feature exists for.
pub fn publish_attention_projection(
    world: Option<Res<WorldConfig>>,
    inbox: Option<Res<CommsInboxRes>>,
    entities: Query<(&EntityUuid, Option<&EntityName>)>,
    tick: Res<crate::sim_tick::SimTick>,
    real: Option<Res<Time<Real>>>,
    mut state: ResMut<GmAttentionState>,
    mut writer: MessageWriter<GmAttentionChanged>,
) {
    let names: BTreeMap<String, String> = entities
        .iter()
        .map(|(uuid, name)| {
            (
                uuid.0.clone(),
                name.map_or_else(|| uuid.0.clone(), |name| name.0.clone()),
            )
        })
        .collect();
    let now_ms = real.map_or(0, |real| {
        real.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
    });
    let rows = collect(
        world.as_deref(),
        inbox.as_deref().map_or(&EMPTY_INBOX, |inbox| &inbox.0),
        &names,
    );

    // Retire the bookkeeping for anything that stopped holding, so a recurrence
    // that somehow reused an identity still gets a fresh age rather than
    // inheriting the resolved row's.
    let live: std::collections::BTreeSet<&String> = rows.iter().map(|row| &row.id).collect();
    state.first_seen.retain(|id, _| live.contains(id));

    let mut occurrences: Vec<GmAttentionOccurrence> = rows
        .into_iter()
        .map(|row| {
            let seen = *state.first_seen.entry(row.id.clone()).or_insert(FirstSeen {
                tick: tick.0,
                real_ms: now_ms,
            });
            GmAttentionOccurrence {
                id: row.id,
                category: row.category,
                band: row.band,
                first_seen_tick: seen.tick,
                age_ms: now_ms.saturating_sub(seen.real_ms),
                reason: row.reason,
                target: row.target,
            }
        })
        .collect();
    // Oldest first, stable-id tie-break. Bands are a grouping the page applies
    // on top; the age order inside each one is decided exactly once, here.
    occurrences.sort_by(|a, b| {
        a.first_seen_tick
            .cmp(&b.first_seen_tick)
            .then_with(|| a.id.cmp(&b.id))
    });
    occurrences.truncate(MAX_GM_ATTENTION_OCCURRENCES);

    let next = GmAttentionProjection { occurrences };
    // Age alone is not a change: it advances every frame by construction, and
    // republishing on it would make "held" meaningless. Compare the queue's
    // membership, order, band, reason and target instead — the things a GM
    // reads — and let the page age its own rows from the last honest sample.
    let changed = state.last.as_ref().is_none_or(|last| {
        last.occurrences.len() != next.occurrences.len()
            || last
                .occurrences
                .iter()
                .zip(&next.occurrences)
                .any(|(a, b)| {
                    a.id != b.id
                        || a.category != b.category
                        || a.band != b.band
                        || a.first_seen_tick != b.first_seen_tick
                        || a.reason != b.reason
                        || a.target != b.target
                })
    });
    if changed {
        state.last = Some(next.clone());
        writer.write(GmAttentionChanged { payload: next });
    }
}

/// A world with no Comms runtime at all still publishes an empty queue rather
/// than nothing, so a GM desk shows "nothing is waiting" instead of a stale
/// list from the previous scenario.
static EMPTY_INBOX: std::sync::LazyLock<crate::console::comms::inbox::CommsInbox> =
    std::sync::LazyLock::new(crate::console::comms::inbox::CommsInbox::new);

/// Registers the attention projection on a GM-presenting peer, exactly as
/// [`crate::gm_activity::GmActivityPlugin`] registers the activity feed.
pub struct GmAttentionPlugin;

impl Plugin for GmAttentionPlugin {
    fn build(&self, app: &mut App) {
        use crate::authoritative::{DeclareState, StateClass};

        app.init_resource::<GmAttentionState>()
            .declare_state::<GmAttentionState>(StateClass::Presentation, "gm-t3-attention-queue")
            .add_message::<GmAttentionChanged>()
            .add_systems(
                PostUpdate,
                publish_attention_projection.run_if(crate::gm_projection::gm_presentation_active),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::messages::{CommsMessage, CommsPriority, CommsResponseView};
    use crate::gm_comms::{GmCommsRoute, GmCommsVisibility};

    fn message(id: &str, sender: &str) -> CommsMessage {
        CommsMessage::injected(
            id.into(),
            sender.into(),
            "sender".into(),
            "body".into(),
            Default::default(),
            vec![CommsResponseView {
                text: "reply".into(),
                important: false,
                available: true,
            }],
            format!("thread-{id}"),
            true,
            CommsPriority::Routine,
        )
    }

    fn route(id: &str, band: Option<&str>) -> GmCommsRoute {
        GmCommsRoute {
            id: id.into(),
            label: "label".into(),
            visibility: GmCommsVisibility::SelectedShips,
            senders: vec!["speaker".into()],
            hails: Vec::new(),
            attention_band: band.map(str::to_string),
        }
    }

    #[test]
    fn authored_band_overrides_the_default_and_an_unknown_word_is_ignored_by_the_reader() {
        assert_eq!(band_for(None), GmAttentionBand::Attention);
        assert_eq!(
            band_for(Some(&route("a", None))),
            GmAttentionBand::Attention
        );
        assert_eq!(
            band_for(Some(&route("a", Some("urgent")))),
            GmAttentionBand::Urgent
        );
        assert_eq!(
            band_for(Some(&route("a", Some("background")))),
            GmAttentionBand::Background
        );
        // The loader refuses this spelling outright (see `gm_comms::validate_routes`);
        // the reader still refuses to invent a band from it.
        assert_eq!(
            band_for(Some(&route("a", Some("Urgent")))),
            GmAttentionBand::Attention
        );
        assert_eq!(GmAttentionBand::from_authored("critical"), None);
    }

    #[test]
    fn only_a_live_unanswered_message_with_options_is_pending() {
        let live = message("m1", "speaker");
        assert!(pending(&live));
        let mut answered = live.clone();
        answered.selected_response = Some(0);
        assert!(!pending(&answered));
        let mut orphaned = live.clone();
        orphaned.is_orphaned = true;
        assert!(!pending(&orphaned));
        let mut placeholder = live;
        placeholder.responses.clear();
        assert!(!pending(&placeholder));
    }

    #[test]
    fn occurrence_identity_follows_the_message_and_carries_the_authored_route() {
        let world = WorldConfig {
            gm_comms_routes: vec![route("private", Some("background"))],
            ..Default::default()
        };
        let mut inbox = crate::console::comms::inbox::CommsInbox::new();
        let mut addressed = message("m1", "speaker-uuid");
        addressed.recipient_ship = Some(crate::command_admission::log::ShipKey(
            "ship-uuid".to_string(),
        ));
        inbox.inject(addressed);
        let names = BTreeMap::from([
            ("speaker-uuid".to_string(), "speaker".to_string()),
            ("ship-uuid".to_string(), "Valiant".to_string()),
        ]);
        let rows = collect(Some(&world), &inbox, &names);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "comms:m1");
        assert_eq!(rows[0].band, GmAttentionBand::Background);
        assert_eq!(rows[0].reason.id, PENDING_COMMS_REASON);
        assert_eq!(rows[0].reason.params.get("sender").unwrap(), "speaker");
        assert_eq!(rows[0].reason.params.get("ship").unwrap(), "Valiant");
        assert_eq!(rows[0].target.route.as_deref(), Some("private"));
        assert_eq!(rows[0].target.conversation.as_deref(), Some("thread-m1"));
    }

    /// A hail nobody addressed at one hull says so, rather than rendering the
    /// addressed sentence around an empty `{ship}`.
    #[test]
    fn a_conversation_with_no_recipient_ship_reads_as_fleet_wide() {
        let mut inbox = crate::console::comms::inbox::CommsInbox::new();
        let fleet_wide = message("m1", "speaker-uuid");
        assert!(fleet_wide.recipient_ship.is_none());
        inbox.inject(fleet_wide);
        let names = BTreeMap::from([("speaker-uuid".to_string(), "speaker".to_string())]);
        let rows = collect(None, &inbox, &names);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].reason.id, PENDING_COMMS_FLEET_REASON);
        assert_eq!(rows[0].reason.params.get("sender").unwrap(), "speaker");
        // No blank parameter at all: the sentence that reads this id does not
        // mention a ship, so carrying an empty one would be a lie in waiting.
        assert!(rows[0].reason.params.get("ship").is_none());
        assert!(rows[0].target.ship.is_none());
    }
}
