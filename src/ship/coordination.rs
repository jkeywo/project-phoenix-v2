use crate::messages::SystemId;
use crate::ship::control_source::ControlSource;
pub use crate::messages::CoordinationPayload;

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
///   target AI         → Consume
///   target Human + sender AI     → Popup
///   target Human + sender Human  → Suppress
pub fn route_coordination(
    sender_origin: ControlSource,
    target_control: ControlSource,
) -> DeliverAction {
    match (target_control, sender_origin) {
        (ControlSource::Ai, _) => DeliverAction::Consume,
        (ControlSource::Human, ControlSource::Ai) => DeliverAction::Popup,
        (ControlSource::Human, ControlSource::Human) => DeliverAction::Suppress,
    }
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
    fn target_human_sender_ai_shows_popup() {
        assert_eq!(
            route_coordination(ControlSource::Ai, ControlSource::Human),
            DeliverAction::Popup
        );
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

    #[test]
    fn suggestion_payload_serde_round_trip() {
        let payload = CoordinationPayload::SuggestTarget {
            uuid: "abc-123".into(),
            reason: "High priority target".into(),
        };
        let json = serde_json::to_string(&payload).unwrap();
        let decoded: CoordinationPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, payload);
    }
}
