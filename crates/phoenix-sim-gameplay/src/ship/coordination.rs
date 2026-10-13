pub use crate::core::messages::{CoordinationAddress, CoordinationPayload};
use crate::core::messages::{StationId, SystemId};
use crate::ship::control_source::{ControlSource, ControlTickPolicy};

// ── Coordination sender-label ids (issue #975) ────────────────────────────────
//
// The sender label rides the wire as one of these string ids, never as English
// prose. `localiseTree` resolves it on the client, so the host viewscreen
// chatter and the phone popup render the same origin name from the same CSV row
// (`assets/strings/strings.csv`). An emitter stamps the id for the system it
// speaks for; nothing downstream composes the word. Rule 11: no player-visible
// English in Rust.
pub const CHATTER_SENDER_AI: &str = "chatter.sender.ai";
pub const CHATTER_SENDER_SENSORS: &str = "chatter.sender.sensors";
pub const CHATTER_SENDER_WEAPONS: &str = "chatter.sender.weapons";
pub const CHATTER_SENDER_NAVIGATION: &str = "chatter.sender.navigation";
pub const CHATTER_SENDER_POWER: &str = "chatter.sender.power";
pub const CHATTER_SENDER_SHIELDS: &str = "chatter.sender.shields";

/// Sender label id for a channel-3 message emitted by a ship's Core — a System
/// that belongs to no Station. Coordination destinations themselves are always
/// an explicit Station or the whole Ship.
pub const CHATTER_ADDRESSEE_CORE: &str = "chatter.addressee.core";
pub const CHATTER_ADDRESSEE_SHIP: &str = "chatter.addressee.ship";

/// Client-resolvable display id naming an explicit Coordination destination.
///
/// Channel-3 (the AI-coordination bus) addresses crew by STATION, not by the
/// fine system underneath it: John's playtest note is that "Navigation → Helm
/// Navigation" should read "Helm" on both sides — the station alone, never the
/// station+system pair, and never the raw system. Both the sender identity
/// (resolved at enqueue from the emitting system) and the destination label
/// (read directly from the typed address) run through the same vocabulary, so
/// the viewscreen chatter bubble and phone popup cannot disagree about the
/// route.
///
/// The returned value is a `station.<id>.name` or whole-ship string id,
/// resolved to words by `localiseTree` on the client exactly as the
/// `chatter.sender.*` ids are — nothing downstream composes the word.
pub fn coordination_addressee_label(address: &CoordinationAddress) -> String {
    match address {
        CoordinationAddress::Station(station) => format!("station.{}.name", station.0),
        CoordinationAddress::Console(console) => console.0.clone(),
        CoordinationAddress::Ship => CHATTER_ADDRESSEE_SHIP.to_string(),
    }
}

/// Resolve an owning Console at the producer boundary. Fixed home Consoles
/// retain Station destinations; detached or movable Consoles retain Console
/// identity so the current host is selected only when the message is delivered.
///
/// Returning `None` for an ownerless or unknown System is deliberate. A
/// producer that means "the whole ship" must author [`CoordinationAddress::Ship`]
/// explicitly; a failed System lookup must never turn into a broadcast.
pub fn address_for_system(
    config: &crate::ship::config::ShipConfig,
    system: &SystemId,
) -> Option<CoordinationAddress> {
    let console = config.console_for_system(system)?;
    if console.human_seeking || console.home_station.is_none() {
        Some(CoordinationAddress::Console(console.id.clone()))
    } else {
        console
            .home_station
            .clone()
            .map(CoordinationAddress::Station)
    }
}

/// Resolve the single destination shared by every authored System of a kind.
///
/// This is for multi-instance receiver families such as `shield_arc`, where no
/// coarse System exists to stand in for the Station. A missing kind, an
/// ownerless instance, or instances split across destinations is ambiguous and
/// returns `None`; callers must never widen any of those cases to `Ship`.
pub fn address_for_system_kind(
    config: &crate::ship::config::ShipConfig,
    kind: &str,
) -> Option<CoordinationAddress> {
    let mut systems = config.systems.iter().filter(|system| system.kind == kind);
    let first = address_for_system(config, &systems.next()?.id)?;
    systems
        .all(|system| address_for_system(config, &system.id).as_ref() == Some(&first))
        .then_some(first)
}

/// Sender identity remains attached to its owning Console across handoff.
/// Real home Stations retain their existing labels; detached Consoles use
/// authored display names (which may be localization keys), then their id.
/// Only genuinely unowned or unknown Systems fall back to Core.
pub fn system_addressee_label(
    config: &crate::ship::config::ShipConfig,
    system: &SystemId,
) -> String {
    if let Some(address) = address_for_system(config, system) {
        return coordination_addressee_label_for_config(config, &address);
    }
    config
        .console_for_system(system)
        .map(|console| {
            if console.name.trim().is_empty() {
                console.id.0.clone()
            } else {
                console.name.clone()
            }
        })
        .unwrap_or_else(|| CHATTER_ADDRESSEE_CORE.to_string())
}

/// What to do with a delivered coordination message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeliverAction {
    /// Target is AI-controlled — the AI consumes it silently.
    Consume,
    /// Target is human-controlled, sender was AI — show a popup.
    Popup,
    /// Both sender and target are human — suppress (they can coordinate IRL).
    Suppress,
}

/// Route a coordination message based on sender origin and target control source.
///
/// Delivery-time matrix:
///   target AI         → Consume (AI drains silently)
///   target Offline    → Consume (no one to receive it)
///   target Human + sender AI     → Popup
///   target Human + sender Human  → Suppress
pub fn route_coordination(
    sender_origin: ControlSource,
    target_control: ControlSource,
) -> DeliverAction {
    match (target_control, sender_origin) {
        (ControlSource::Ai | ControlSource::Simplified, _) | (ControlSource::Offline, _) => {
            DeliverAction::Consume
        }
        (ControlSource::Human, ControlSource::Ai | ControlSource::Simplified) => {
            DeliverAction::Popup
        }
        (ControlSource::Human, ControlSource::Human)
        | (ControlSource::Human, ControlSource::Offline) => DeliverAction::Suppress,
    }
}

// ── Ship-wide broadcast (issue #879) ──────────────────────────────────────────

/// One crew seat on the source ship, as the ship-broadcast router sees it.
///
/// A *seat* is a station, not a system: a coordination popup lands on a
/// console, and a console belongs to whoever is holding the station. The
/// adapter reduces the station's fine systems to one [`ControlSource`] with
/// [`seat_control_source`] before building this.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShipSeat {
    /// The station this seat is.
    pub station: StationId,
    /// Who is operating it, reduced from its fine systems.
    pub control: ControlSource,
    /// The session token of the connected holder, or `None` when nobody is
    /// browser-connected to it.
    pub holder: Option<String>,
}

/// Reduce a station's fine-system tick policies to the seat's control source.
///
/// A station owns several systems and a coordination popup is addressed to the
/// station, so the router needs one answer for the seat as a whole:
///
/// * any system still accepting human input → the seat is **Human**; someone is
///   sitting there and can read a popup.
/// * otherwise, any system operating AI → the seat is **Ai**; it is backfilled,
///   and there is nobody to show anything to.
/// * otherwise → **Offline**: damage-disabled, explicitly offline, or a station
///   with no systems at all.
///
/// The precedence is human-first for the same reason `process_coordination_lag`
/// treats a damage-disabled console as `Consume`: the question the router is
/// asking is "can a person read this", and one live console on the station is
/// enough for the answer to be yes.
pub fn seat_control_source(policies: &[ControlTickPolicy]) -> ControlSource {
    if policies
        .iter()
        .any(|p| p.accept_human_input || p.accept_summary_input)
    {
        ControlSource::Human
    } else if policies.iter().any(|p| p.operate_ai) {
        ControlSource::Ai
    } else {
        ControlSource::Offline
    }
}

/// Fan one ship-wide advisory out to every seat on the SOURCE ship that the
/// existing delivery matrix resolves to a popup.
///
/// This is [`route_coordination`] applied per seat rather than to one addressed
/// target — the extension issue #879 needed and the reason it is not a second,
/// parallel rule. A backfilled seat's advisory therefore reaches every human
/// seat on the ship (target Human + sender Ai → `Popup`) and no AI or offline
/// seat (either → `Consume`), and an advisory whose sender is itself human is
/// suppressed at every seat, exactly as a human-to-human channel-3 message
/// already is: two people on the same bridge talk to each other.
///
/// Seats with no connected holder are dropped last, not first, so the
/// `Consume`/`Suppress` reasoning above is about the seat's control source and
/// not about whether anyone happens to be logged in.
///
/// Returns the recipients in the order `seats` was given, which the adapter
/// derives from the authored station list — a deterministic order, not a hash
/// order, so two lockstep peers emit the same popups in the same sequence.
pub fn broadcast_to_ship(sender_origin: ControlSource, seats: &[ShipSeat]) -> Vec<&ShipSeat> {
    seats
        .iter()
        .filter(|seat| route_coordination(sender_origin, seat.control) == DeliverAction::Popup)
        .filter(|seat| seat.holder.is_some())
        .collect()
}

/// Resolve world-authored scenario detail-floor vocabulary onto one hull.
///
/// A scenario cannot name per-hull System ids because the crew chooses its hull
/// in the lobby. Each selector therefore names either a console family (the
/// authored Station id) or a System kind. Matching both namespaces and taking
/// their union preserves that hull independence; the result is the concrete,
/// deterministic System-id set consumed by the rating resolver.
pub fn resolve_scenario_detail_floor(
    config: &crate::ship::config::ShipConfig,
    selectors: &[String],
) -> std::collections::BTreeSet<SystemId> {
    config
        .systems
        .iter()
        .filter(|system| {
            selectors.iter().any(|selector| {
                system.kind == *selector
                    || system.console.as_ref().is_some_and(|id| id.0 == *selector)
                    || config
                        .system_home_station(system)
                        .is_some_and(|station| station.0 == *selector)
            })
        })
        .map(|system| system.id.clone())
        .collect()
}

/// A coordination message queued for lagged delivery.
#[derive(Clone, Debug, PartialEq)]
pub struct QueuedCoordination {
    /// Sender control origin captured at enqueue time.
    pub sender_origin: ControlSource,
    /// Explicit Station or whole-ship destination, resolved live at delivery.
    pub address: CoordinationAddress,
    /// Typed coordination payload.
    pub payload: CoordinationPayload,
    /// Producer-owned display envelope, carried unchanged through the delay.
    pub presentation: crate::core::messages::CoordinationPresentation,
    /// Human-readable label for the sender (e.g. "AI Tactical", "Captain").
    pub sender_label: String,
    /// Logical simulation tick at which this message is due for delivery.
    pub due_tick: u64,
}

/// Lag scheduler for channel-3 coordination messages.
///
/// Every coordination message converts its authored seconds to logical ticks
/// once, at enqueue. At delivery time the target's live control source is
/// resolved by the caller.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CoordinationLagQueue(Vec<QueuedCoordination>);

/// Convert an authored Coordination delay to whole logical simulation ticks.
///
/// Multiplication intentionally stays in `f32`: both inputs are authored/stored
/// as `f32`, and widening `0.6_f32` before multiplying by 60 turns its exact
/// intended 36 ticks into `36.000001...` and therefore 37 after `ceil`. World
/// and Ship config validation keep the production inputs finite and
/// non-negative; the guards retain deterministic no-delay behaviour in small
/// pure fixtures that call this helper directly.
pub fn coordination_lag_ticks(lag_secs: f32, sim_tick_hz: f32) -> u64 {
    if !lag_secs.is_finite() || lag_secs <= 0.0 || !sim_tick_hz.is_finite() || sim_tick_hz <= 0.0 {
        return 0;
    }
    (lag_secs * sim_tick_hz).ceil() as u64
}

impl CoordinationLagQueue {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Pending messages in enqueue order, for authoritative snapshot
    /// projection. The queue's runtime representation stays private; callers
    /// can observe the values but cannot reorder them in place.
    pub fn pending(&self) -> &[QueuedCoordination] {
        &self.0
    }

    /// Replace bootstrap state with a restored enqueue-ordered queue.
    pub fn replace(&mut self, pending: Vec<QueuedCoordination>) {
        self.0 = pending;
    }

    /// Enqueue a coordination message with its due time already computed.
    pub fn enqueue(&mut self, msg: QueuedCoordination) {
        self.0.push(msg);
    }

    /// Drain all messages whose due tick has passed, returning them in enqueue
    /// order. Equal-tick entries retain their original FIFO order.
    pub fn due_messages(&mut self, now_tick: u64) -> Vec<QueuedCoordination> {
        let mut due = Vec::new();
        self.0.retain(|msg| {
            if msg.due_tick <= now_tick {
                due.push(msg.clone());
                false
            } else {
                true
            }
        });
        due
    }
}

#[cfg(test)]
#[path = "coordination_tests.rs"]
mod tests;

/// Console destinations retain their authored identity while their host changes.
pub fn coordination_addressee_label_for_config(
    config: &crate::ship::config::ShipConfig,
    address: &CoordinationAddress,
) -> String {
    if let CoordinationAddress::Console(id) = address {
        if let Some(console) = config.consoles.iter().find(|console| &console.id == id) {
            if !console.name.trim().is_empty() {
                return console.name.clone();
            }
        }
    }
    coordination_addressee_label(address)
}
