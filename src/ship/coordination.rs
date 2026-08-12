pub use crate::messages::CoordinationPayload;
use crate::messages::{StationId, SystemId};
#[cfg(test)]
use crate::ship::control_source::ControlSourceResolver;
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
        (ControlSource::Ai, _) | (ControlSource::Offline, _) => DeliverAction::Consume,
        (ControlSource::Human, ControlSource::Ai) => DeliverAction::Popup,
        (ControlSource::Human, ControlSource::Human)
        | (ControlSource::Human, ControlSource::Offline) => DeliverAction::Suppress,
    }
}

/// Resolves a channel-3 target to the station that should receive a popup.
///
/// Coordination uses both fine system ids and console-level keys. Unlike
/// command admission, the latter are valid delivery addresses: Helm and
/// Tactical are not `[[system]]` ids, while legacy aggregate targets can name
/// an authored station directly. An unresolved target remains ownerless so the
/// existing fallback delivery policy can decide what to do with it.
pub fn station_for_target(
    config: &crate::ship::config::ShipConfig,
    target: &SystemId,
) -> Option<StationId> {
    if let Some(system) = config.system(target) {
        return system.station.clone();
    }

    if *target == crate::system_registry::helm_station_key() {
        return config
            .system(&crate::system_registry::helm_steering_system_id())
            .and_then(|system| system.station.clone());
    }

    if *target == crate::system_registry::tactical_station_key() {
        return config.weapons_station();
    }

    let station = StationId(target.0.clone());
    config.station(&station).map(|_| station)
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
    if policies.iter().any(|p| p.accept_human_input) {
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

/// A coordination message queued for lagged delivery.
#[derive(Clone, Debug, PartialEq)]
pub struct QueuedCoordination {
    /// Sender control origin captured at enqueue time.
    pub sender_origin: ControlSource,
    /// Target system instance — resolved to live control source at delivery time.
    pub target: SystemId,
    /// Typed coordination payload.
    pub payload: CoordinationPayload,
    /// Human-readable label for the sender (e.g. "AI Tactical", "Captain").
    pub sender_label: String,
    /// Simulation time (seconds) at which this message is due for delivery.
    pub due_time: f32,
}

/// Lag scheduler for channel-3 coordination messages.
///
/// Every coordination message is queued with `due_time = sent_at + lag_secs`.
/// At delivery time the target's live control source is resolved by the caller.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CoordinationLagQueue(Vec<QueuedCoordination>);

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

    /// Enqueue a coordination message with its due time already computed.
    pub fn enqueue(&mut self, msg: QueuedCoordination) {
        self.0.push(msg);
    }

    /// Drain all messages whose due_time has passed, returning them in
    /// enqueue order. `now` is the current simulation time in seconds.
    pub fn due_messages(&mut self, now: f32) -> Vec<QueuedCoordination> {
        let mut due = Vec::new();
        self.0.retain(|msg| {
            if msg.due_time <= now {
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
mod tests {
    use super::*;

    // ── Routing matrix ────────────────────────────────────────────────────

    #[test]
    fn target_ai_always_consumes_regardless_of_sender() {
        assert_eq!(
            route_coordination(ControlSource::Ai, ControlSource::Ai),
            DeliverAction::Consume
        );
        assert_eq!(
            route_coordination(ControlSource::Human, ControlSource::Ai),
            DeliverAction::Consume
        );
    }

    #[test]
    fn target_human_sender_human_suppresses() {
        assert_eq!(
            route_coordination(ControlSource::Human, ControlSource::Human),
            DeliverAction::Suppress
        );
    }

    #[test]
    fn station_for_target_resolves_console_keys_and_ownerless_targets() {
        let config = crate::ship::components::ShipConfigComponent::default().0;

        assert_eq!(
            station_for_target(&config, &crate::system_registry::helm_station_key()),
            config
                .system(&crate::system_registry::helm_steering_system_id())
                .and_then(|system| system.station.clone())
        );
        assert_eq!(
            station_for_target(&config, &crate::system_registry::tactical_station_key()),
            config.weapons_station()
        );
        assert_eq!(
            station_for_target(&config, &SystemId("not-a-real-target".into())),
            None
        );
    }

    #[test]
    fn target_human_sender_ai_shows_popup() {
        assert_eq!(
            route_coordination(ControlSource::Ai, ControlSource::Human),
            DeliverAction::Popup
        );
    }

    /// #673: a Channel-3 message targeting a damage-disabled human console
    /// must resolve via `policy_for` (which honours `offline_systems`), not
    /// the raw `source_for`. A damage-disabled console can neither operate
    /// AI nor accept human input, so delivery must be treated as Consume,
    /// never Popup — mirroring the resolution logic in
    /// `process_coordination_lag` (src/ship_plugin.rs).
    #[test]
    fn target_damage_disabled_human_console_is_not_a_popup_candidate() {
        let mut resolver = ControlSourceResolver::new();
        let helm = SystemId("helm".into());

        // Station rating says Human, but damage has taken the console offline.
        resolver.set(helm.clone(), ControlSource::Human);
        resolver.set_offline(helm.clone(), true);

        let target_policy = resolver.policy_for(&helm);
        assert!(!target_policy.operate_ai && !target_policy.accept_human_input);

        // The caller must treat this as Consume rather than routing through
        // route_coordination with the raw (stale) ControlSource::Human, which
        // would incorrectly produce a Popup.
        let action = if !target_policy.operate_ai && !target_policy.accept_human_input {
            DeliverAction::Consume
        } else {
            route_coordination(ControlSource::Ai, resolver.source_for(&helm))
        };
        assert_eq!(action, DeliverAction::Consume);
    }

    // ── Ship-wide broadcast (issue #879) ──────────────────────────────────

    fn seat(station: &str, control: ControlSource, holder: Option<&str>) -> ShipSeat {
        ShipSeat {
            station: StationId(station.into()),
            control,
            holder: holder.map(|h| h.to_string()),
        }
    }

    fn human_policy() -> ControlTickPolicy {
        crate::ship::control_source::control_tick_policy(ControlSource::Human)
    }

    fn ai_policy() -> ControlTickPolicy {
        crate::ship::control_source::control_tick_policy(ControlSource::Ai)
    }

    fn offline_policy() -> ControlTickPolicy {
        crate::ship::control_source::control_tick_policy(ControlSource::Offline)
    }

    #[test]
    fn a_station_with_no_systems_is_an_offline_seat() {
        assert_eq!(seat_control_source(&[]), ControlSource::Offline);
    }

    #[test]
    fn a_fully_backfilled_station_is_an_ai_seat() {
        assert_eq!(
            seat_control_source(&[ai_policy(), ai_policy(), ai_policy()]),
            ControlSource::Ai
        );
    }

    /// One live human console on the station is enough: the question the router
    /// asks is "can a person read this".
    #[test]
    fn one_human_system_makes_the_whole_seat_human() {
        assert_eq!(
            seat_control_source(&[ai_policy(), human_policy(), offline_policy()]),
            ControlSource::Human
        );
    }

    #[test]
    fn a_damage_disabled_station_is_an_offline_seat() {
        assert_eq!(
            seat_control_source(&[offline_policy(), offline_policy()]),
            ControlSource::Offline
        );
    }

    /// AC: a backfilled seat's advisory reaches EVERY human seat on the source
    /// ship, and no AI or offline seat.
    #[test]
    fn a_backfilled_senders_advisory_reaches_every_human_seat_and_no_other() {
        let seats = vec![
            seat("captain", ControlSource::Human, Some("alice")),
            seat("helm", ControlSource::Human, Some("hikaru")),
            seat("tactical", ControlSource::Ai, None),
            seat("shields", ControlSource::Ai, Some("stale-token")),
            seat("power", ControlSource::Offline, Some("scotty")),
        ];

        let recipients = broadcast_to_ship(ControlSource::Ai, &seats);

        let stations: Vec<&str> = recipients.iter().map(|s| s.station.0.as_str()).collect();
        assert_eq!(
            stations,
            vec!["captain", "helm"],
            "every human seat, in authored order — and neither the backfilled \
             Tactical/Shields nor the offline Power, whatever token they carry"
        );
        let tokens: Vec<&str> = recipients
            .iter()
            .map(|s| s.holder.as_deref().unwrap())
            .collect();
        assert_eq!(tokens, vec!["alice", "hikaru"]);
    }

    /// The seat semantics are about the CONTROL SOURCE, not about who happens
    /// to be logged in: a human-held station whose holder has dropped is still
    /// a human seat, it simply has nobody to deliver to.
    #[test]
    fn a_human_seat_with_no_connected_holder_receives_nothing() {
        let seats = vec![seat("helm", ControlSource::Human, None)];
        assert!(broadcast_to_ship(ControlSource::Ai, &seats).is_empty());
    }

    /// The broadcast is `route_coordination` applied per seat, so it inherits
    /// the human→human `Suppress`: two officers on the same bridge do not need
    /// popups about each other.
    #[test]
    fn a_human_senders_broadcast_is_suppressed_at_every_seat() {
        let seats = vec![
            seat("captain", ControlSource::Human, Some("alice")),
            seat("helm", ControlSource::Human, Some("hikaru")),
        ];
        assert!(
            broadcast_to_ship(ControlSource::Human, &seats).is_empty(),
            "human sender + human seats is Suppress, as it has always been"
        );
    }

    #[test]
    fn a_ship_with_no_human_seats_broadcasts_to_nobody() {
        let seats = vec![
            seat("captain", ControlSource::Ai, None),
            seat("helm", ControlSource::Ai, None),
        ];
        assert!(broadcast_to_ship(ControlSource::Ai, &seats).is_empty());
    }

    // ── Lag queue ─────────────────────────────────────────────────────────

    #[test]
    fn queue_starts_empty() {
        let queue = CoordinationLagQueue::new();
        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn enqueue_adds_message() {
        let mut queue = CoordinationLagQueue::new();
        queue.enqueue(QueuedCoordination {
            sender_origin: ControlSource::Ai,
            target: SystemId("helm".into()),
            payload: CoordinationPayload::Advisory {
                message: "test".into(),
            },
            sender_label: String::new(),
            due_time: 10.0,
        });
        assert_eq!(queue.len(), 1);
        assert!(!queue.is_empty());
    }

    #[test]
    fn due_messages_returns_nothing_before_deadline() {
        let mut queue = CoordinationLagQueue::new();
        queue.enqueue(QueuedCoordination {
            sender_origin: ControlSource::Ai,
            target: SystemId("helm".into()),
            payload: CoordinationPayload::Advisory {
                message: "test".into(),
            },
            sender_label: String::new(),
            due_time: 10.0,
        });
        let due = queue.due_messages(5.0);
        assert!(due.is_empty());
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn due_messages_returns_ready_at_deadline() {
        let mut queue = CoordinationLagQueue::new();
        queue.enqueue(QueuedCoordination {
            sender_origin: ControlSource::Ai,
            target: SystemId("helm".into()),
            payload: CoordinationPayload::Advisory {
                message: "test".into(),
            },
            sender_label: String::new(),
            due_time: 10.0,
        });
        let due = queue.due_messages(10.0);
        assert_eq!(due.len(), 1);
        assert!(queue.is_empty());
    }

    #[test]
    fn due_messages_returns_ready_after_deadline() {
        let mut queue = CoordinationLagQueue::new();
        queue.enqueue(QueuedCoordination {
            sender_origin: ControlSource::Ai,
            target: SystemId("helm".into()),
            payload: CoordinationPayload::Advisory {
                message: "test".into(),
            },
            sender_label: String::new(),
            due_time: 10.0,
        });
        let due = queue.due_messages(15.0);
        assert_eq!(due.len(), 1);
        assert!(queue.is_empty());
    }

    #[test]
    fn due_messages_respects_per_message_deadlines() {
        let mut queue = CoordinationLagQueue::new();
        queue.enqueue(QueuedCoordination {
            sender_origin: ControlSource::Ai,
            target: SystemId("helm".into()),
            payload: CoordinationPayload::Advisory {
                message: "first".into(),
            },
            sender_label: String::new(),
            due_time: 1.0,
        });
        queue.enqueue(QueuedCoordination {
            sender_origin: ControlSource::Ai,
            target: SystemId("helm".into()),
            payload: CoordinationPayload::Advisory {
                message: "early".into(),
            },
            sender_label: String::new(),
            due_time: 5.0,
        });
        queue.enqueue(QueuedCoordination {
            sender_origin: ControlSource::Ai,
            target: SystemId("helm".into()),
            payload: CoordinationPayload::Advisory {
                message: "late".into(),
            },
            sender_label: String::new(),
            due_time: 15.0,
        });

        let due = queue.due_messages(10.0);
        assert_eq!(due.len(), 2);
        assert_eq!(
            due[0].payload,
            CoordinationPayload::Advisory {
                message: "first".into()
            }
        );
        assert_eq!(
            due[1].payload,
            CoordinationPayload::Advisory {
                message: "early".into()
            }
        );
        assert_eq!(queue.len(), 1);
        assert_eq!(
            queue.0[0].payload,
            CoordinationPayload::Advisory {
                message: "late".into()
            }
        );
    }

    #[test]
    fn sender_origin_is_captured_at_enqueue_time() {
        let mut queue = CoordinationLagQueue::new();
        queue.enqueue(QueuedCoordination {
            sender_origin: ControlSource::Ai,
            target: SystemId("helm".into()),
            payload: CoordinationPayload::Alert {
                title: "AI Alert".into(),
                body: "Incoming threat detected.".into(),
            },
            sender_label: "AI Tactical".into(),
            due_time: 5.0,
        });
        let due = queue.due_messages(10.0);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].sender_origin, ControlSource::Ai);
        assert_eq!(due[0].sender_label, "AI Tactical");
    }

    #[test]
    fn target_is_resolved_live_at_delivery_time() {
        let mut queue = CoordinationLagQueue::new();
        let target = SystemId("red-alert".into());
        queue.enqueue(QueuedCoordination {
            sender_origin: ControlSource::Ai,
            target: target.clone(),
            payload: CoordinationPayload::Alert {
                title: "test".into(),
                body: "body".into(),
            },
            sender_label: String::new(),
            due_time: 1.0,
        });
        let due = queue.due_messages(2.0);
        assert_eq!(due[0].target, target);
    }
}
