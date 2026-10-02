use super::*;
use crate::core::messages::{
    ActionCorrelationId, ActionFeedbackOutcome, DeliveryClass, ServerMessage,
};
use crate::lobby::handler::Target;

fn identity(n: u8) -> PaneIdentity {
    PaneIdentity::adopt(
        format!("3f1a6c2e-0a11-4b3c-9d55-00000000000{n}"),
        format!("crew-{n}"),
    )
    .unwrap()
}

#[test]
fn what_a_page_says_reaches_the_seam_as_a_typed_message_on_the_panes_own_token() {
    let bus = PaneBus::default();
    let id = bus.open(identity(1));
    let token = bus.token_of(id).unwrap();
    bus.submit_json(
        id,
        &format!(r#"{{"type":"Identify","data":{{"token":"{token}","name":"Ada"}}}}"#),
    )
    .unwrap();
    let events = bus.transport().poll();
    assert_eq!(
        events,
        vec![TransportEvent::Received {
            token,
            msg: ClientMessage::Identify {
                token: bus.token_of(id).unwrap(),
                name: "Ada".to_string()
            }
        }]
    );
}

#[test]
fn a_pane_cannot_identify_as_the_host_operator() {
    // The third gate. The seam refuses `__local_console__` in the ENVELOPE
    // and `handle_identify` refuses it in the BODY — but a pane that
    // presented it would still have got as far as the seam, and the seam
    // sees the pane's own token there. Refusing at the bus means the
    // message never leaves the pane at all.
    let bus = PaneBus::default();
    let id = bus.open(identity(1));
    let err = bus
        .submit(
            id,
            ClientMessage::Identify {
                token: crate::console_bridge::LOCAL_CONSOLE_TOKEN.to_string(),
                name: "impostor".to_string(),
            },
        )
        .expect_err("a pane may only identify as itself");
    assert!(matches!(err, PaneInputRefusal::Impersonation { .. }));
    assert!(bus.transport().poll().is_empty());
}

#[test]
fn a_pane_cannot_identify_as_another_pane() {
    // The hole the other two gates do not cover: another participant's
    // ordinary, entirely un-reserved token.
    let bus = PaneBus::default();
    let mine = bus.open(identity(1));
    let theirs = bus.open(identity(2));
    let their_token = bus.token_of(theirs).unwrap();
    assert_eq!(
        bus.submit(
            mine,
            ClientMessage::Identify {
                token: their_token.clone(),
                name: "impostor".to_string()
            }
        ),
        Err(PaneInputRefusal::Impersonation {
            presented: their_token
        })
    );
}

#[test]
fn a_targeted_projection_reaches_only_the_pane_it_names() {
    // The acceptance criterion, at the transport: a pane cannot read
    // another pane's audience projection. `Audience::Holding(station)` has
    // already resolved to this `Target::Token` by the time it arrives.
    let bus = PaneBus::default();
    let helm = bus.open(identity(1));
    identify_test_pane(&bus, helm);
    let comms = bus.open(identity(2));
    identify_test_pane(&bus, comms);
    bus.mark_live(helm);
    bus.mark_live(comms);
    let helm_token = bus.token_of(helm).unwrap();

    bus.transport().dispatch(TransportDispatch {
        target: &Target::Token(helm_token),
        msg: &ServerMessage::GameStarted,
        delivery: DeliveryClass::Reliable,
    });

    assert_eq!(bus.take_outbound(helm).len(), 1);
    assert!(
        bus.take_outbound(comms).is_empty(),
        "a pane receives its own audience's projections and no others"
    );
}

#[test]
fn correlated_action_feedback_matches_the_phone_transport_audience_and_codec() {
    let bus = PaneBus::default();
    let captain = bus.open(identity(1));
    identify_test_pane(&bus, captain);
    let other = bus.open(identity(2));
    identify_test_pane(&bus, other);
    bus.mark_live(captain);
    bus.mark_live(other);
    let captain_token = bus.token_of(captain).unwrap();
    let expected = ServerMessage::ActionFeedback {
        correlation: ActionCorrelationId::new("native-pane-red-alert")
            .expect("valid test correlation"),
        outcome: ActionFeedbackOutcome::Applied,
    };

    bus.transport().dispatch(TransportDispatch {
        target: &Target::Token(captain_token),
        msg: &expected,
        delivery: DeliveryClass::Reliable,
    });

    let queued = bus.take_outbound(captain);
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].delivery, DeliveryClass::Reliable);
    assert_eq!(JsonCodec.decode_server(&queued[0].json).unwrap(), expected);
    assert!(
        bus.take_outbound(other).is_empty(),
        "feedback is token-targeted, never broadcast to another pane"
    );
}

#[test]
fn a_broadcast_reaches_every_open_pane_and_no_closed_one() {
    let bus = PaneBus::default();
    let a = bus.open(identity(1));
    identify_test_pane(&bus, a);
    let b = bus.open(identity(2));
    identify_test_pane(&bus, b);
    bus.mark_live(a);
    bus.mark_live(b);
    bus.close(b);
    bus.transport().dispatch(TransportDispatch {
        target: &Target::All,
        msg: &ServerMessage::GameStarted,
        delivery: DeliveryClass::Reliable,
    });
    assert_eq!(bus.take_outbound(a).len(), 1);
    assert!(bus.take_outbound(b).is_empty());
}

#[test]
fn closing_a_pane_owes_the_lobby_exactly_one_disconnect() {
    let bus = PaneBus::default();
    let id = bus.open(identity(1));
    identify_test_pane(&bus, id);
    let token = bus.token_of(id).unwrap();
    bus.close(id);
    bus.close(id);
    assert_eq!(
        bus.transport().poll(),
        vec![TransportEvent::Disconnected { token }]
    );
    assert!(bus.transport().poll().is_empty(), "and only once");
}

#[test]
fn rebuilt_console_keeps_controller_while_its_replacement_page_loads() {
    use super::super::recovery::{service_faults, PaneFault};
    const PADS: &str = "window.__phoenixSetGamepads([{\"index\":0,\"id\":\"pad\",\"buttons\":[{\"pressed\":true,\"value\":1}],\"axes\":[1]}])";
    const SELECT: &str = r#"{"type":"NativeOperator","operation":"select","index":0}"#;
    for crash in [false, true] {
        let bus = PaneBus::default();
        let original = bus.open(identity(1));
        let competitor = bus.open(identity(2));
        bus.observe_gamepads(PADS);
        assert!(bus.submit_operator_record(original, SELECT));
        let token = bus.token_of(original).unwrap();
        let replacement = if crash {
            bus.fault(original, PaneFault::ViewCrashed);
            service_faults(&bus).pop().unwrap().recreated.unwrap().0
        } else {
            bus.rebuild(original).unwrap().0
        };
        assert_eq!(bus.token_of(replacement).as_deref(), Some(token.as_str()));
        assert!(bus.submit_operator_record(competitor, SELECT));
        assert!(bus.take_operator_replies(competitor)[0].contains("refused"));
        assert!(bus
            .gamepads_for_pane(replacement, PADS)
            .contains("\"nativeOwned\":true"));
        assert!(bus
            .gamepads_for_pane(competitor, PADS)
            .contains("\"available\":false"));
        // The new page has not selected anything. Its controller belongs
        // to it already, and closing the obsolete view cannot free it.
        bus.close(original);
        assert!(bus
            .gamepads_for_pane(replacement, PADS)
            .contains("\"nativeOwned\":true"));
        bus.close(replacement);
        assert!(bus.submit_operator_record(competitor, SELECT));
        assert!(bus
            .gamepads_for_pane(competitor, PADS)
            .contains("\"nativeOwned\":true"));
        bus.observe_gamepads("window.__phoenixSetGamepads([])");
        bus.observe_gamepads(PADS);
        assert!(bus
            .gamepads_for_pane(competitor, PADS)
            .contains("\"nativeOwned\":false"));
    }
}

#[test]
fn queued_fault_cannot_rebuild_a_pane_already_replaced_by_a_move() {
    use super::super::recovery::{service_faults, PaneFault};
    const PADS: &str =
        "window.__phoenixSetGamepads([{\"index\":0,\"id\":\"pad\",\"buttons\":[],\"axes\":[1]}])";
    const SELECT: &str = r#"{"type":"NativeOperator","operation":"select","index":0}"#;
    let bus = PaneBus::default();
    let original = bus.open(identity(1));
    bus.observe_gamepads(PADS);
    assert!(bus.submit_operator_record(original, SELECT));
    let token = bus.token_of(original).unwrap();
    bus.fault(original, PaneFault::ViewCrashed);
    let replacement = bus.rebuild(original).unwrap().0;
    let outcomes = service_faults(&bus);
    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].recreated.is_none());
    assert_eq!(bus.open_count(), 1);
    assert_eq!(bus.token_of(replacement).as_deref(), Some(token.as_str()));
    assert!(bus.rebuild(original).is_none());
    assert!(bus
        .gamepads_for_pane(replacement, PADS)
        .contains("\"nativeOwned\":true"));
    let competitor = bus.open(identity(2));
    assert!(bus.submit_operator_record(competitor, SELECT));
    assert!(bus.take_operator_replies(competitor)[0].contains("refused"));
}

#[test]
fn recreate_refuses_a_pane_that_is_not_closed() {
    // The same-token invariant at the seam: an OPEN pane still owns its token,
    // so recreation must never clone it into a second live pane. `recreate`
    // only ever follows `close` in production; this proves it refuses the
    // misuse rather than minting the duplicate.
    let bus = PaneBus::default();
    let open = bus.open(identity(1));
    assert!(
        bus.recreate(open).is_none(),
        "an open pane cannot be recreated — that would duplicate its token"
    );
    assert_eq!(bus.open_count(), 1, "and nothing new was opened");

    // After a close it is allowed, on the same identity.
    let token = bus.token_of(open).unwrap();
    bus.close(open);
    let (recreated, _) = bus.recreate(open).expect("a closed pane recreates");
    assert_eq!(bus.token_of(recreated).as_deref(), Some(token.as_str()));
}

#[test]
fn is_open_answers_true_for_live_panes_and_false_for_closed_ones() {
    let bus = PaneBus::default();
    let id = bus.open(identity(1));
    assert!(bus.is_open(id));
    bus.close(id);
    assert!(
        !bus.is_open(id),
        "a closed pane's lingering record is not open"
    );
    assert!(!bus.is_open(PaneId(999)), "an unknown handle is not open");
}

#[test]
fn a_pane_that_speaks_and_then_closes_in_one_frame_is_heard_before_it_disconnects() {
    // Order matters to the lobby: a `ReleaseStation` followed by a
    // disconnect vacates the seat; the reverse order re-seats a participant
    // who has gone.
    let bus = PaneBus::default();
    let id = bus.open(identity(1));
    identify_test_pane(&bus, id);
    bus.submit(id, ClientMessage::ReleaseStation).unwrap();
    bus.close(id);
    let events = bus.transport().poll();
    assert!(matches!(events[0], TransportEvent::Received { .. }));
    assert!(matches!(events[1], TransportEvent::Disconnected { .. }));
}

#[test]
fn a_page_that_produces_nonsense_is_refused_with_a_snippet_rather_than_panicking() {
    let bus = PaneBus::default();
    let id = bus.open(identity(1));
    let err = bus.submit_json(id, "not json at all").unwrap_err();
    assert!(matches!(err, PaneInputRefusal::Undecodable { .. }));
    assert!(bus.transport().poll().is_empty());
}

#[test]
fn closing_a_pane_stops_serving_its_document() {
    // `HostedDocuments::withdraw` had no production caller at all: a pane
    // closed by a fault left its page published for the rest of the
    // process's life, and a host that ran a long session accumulated one
    // dead console per closed pane.
    let bus = PaneBus::default();
    let documents = HostedDocuments::default();
    let a = bus.open(identity(1));
    let b = bus.open(identity(2));
    bus.attach_documents(documents.clone());
    bus.publish_document(a, "/client/pane-0-aaaa.html".to_string(), "a".to_string());
    bus.publish_document(b, "/client/pane-1-bbbb.html".to_string(), "b".to_string());
    assert_eq!(documents.len(), 2);

    bus.close(a);
    assert_eq!(documents.get("/client/pane-0-aaaa.html"), None);
    assert_eq!(bus.document_path(a), None);
    assert_eq!(
        documents.get("/client/pane-1-bbbb.html"),
        Some("b".to_string()),
        "and only the closed pane's"
    );

    // Host shutdown takes the rest, before the delivery thread is joined.
    bus.withdraw_all();
    assert!(documents.is_empty());
}

#[test]
fn a_bus_with_no_documents_attached_closes_panes_exactly_as_before() {
    // Every test that drives panes without an HTTP server, which is most of
    // them: publishing is a no-op and closing must not care.
    let bus = PaneBus::default();
    let id = bus.open(identity(1));
    bus.publish_document(id, "/client/pane-0-aaaa.html".to_string(), "a".to_string());
    assert_eq!(bus.document_path(id), None);
    bus.close(id);
    assert_eq!(bus.open_count(), 0);
}

#[test]
fn a_pane_whose_page_stopped_draining_is_reported_as_faulted() {
    // Every queued message is reliable, so nothing may be dropped. The
    // caller closes the pane rather than letting the page drift out of
    // agreement with the simulation behind a clean log.
    let bus = PaneBus::with_capacity(1);
    let id = bus.open(identity(1));
    identify_test_pane(&bus, id);
    bus.mark_live(id);
    let mut transport = bus.transport();
    for _ in 0..3 {
        transport.dispatch(TransportDispatch {
            target: &Target::All,
            msg: &ServerMessage::GameStarted,
            delivery: DeliveryClass::Reliable,
        });
    }
    assert_eq!(bus.take_faulted(), vec![(id, PaneFault::ReliableOverflow)]);
    assert!(bus.take_faulted().is_empty(), "reported once per overflow");
}

// ── a console opened after init (issue #1331) ───────────────────────────

#[test]
fn a_console_opened_at_runtime_is_an_ordinary_participant_with_a_fresh_token() {
    // The crew-symmetry criterion at the seam: a console the lobby's screen
    // row opened joins on a minted, ordinary session token — not the host
    // operator's, and not one shared with anything else — so nothing
    // downstream of admission can tell it from a phone.
    let bus = PaneBus::default();
    let (helm, _) = bus.open_console("helm");
    let (weapons, _) = bus.open_console("weapons");

    let helm_token = bus.token_of(helm).expect("a console holds a token");
    let weapons_token = bus.token_of(weapons).unwrap();
    assert_ne!(helm_token, weapons_token);
    assert!(!crate::lobby::handler::is_reserved_token(&helm_token));
    assert_eq!(bus.name_of(helm).as_deref(), Some("helm"));
    assert_eq!(
        bus.open_pane_for_name("helm"),
        Some(helm),
        "the pane and the layout share one namespace, so the station id resolves it"
    );
    assert_eq!(bus.open_count(), 2);
}

#[test]
fn a_console_queues_its_view_on_the_same_queue_a_recreated_pane_does() {
    // One queue for both, because what the pane host has to do is identical.
    let bus = PaneBus::default();
    let (id, _) = bus.open_console("helm");
    assert_eq!(
        bus.take_pending_views()
            .into_iter()
            .map(|(p, _)| p)
            .collect::<Vec<_>>(),
        vec![id]
    );
    assert!(bus.take_pending_views().is_empty(), "drained once");
}

#[test]
fn closing_a_console_owes_the_lobby_the_disconnect_a_dropped_phone_would() {
    // Unassigning is the ordinary participant-left path — the station keeps
    // its holder and flips to Backfill — rather than a native special case.
    let bus = PaneBus::default();
    let (id, _) = bus.open_console("helm");
    identify_test_pane(&bus, id);
    let token = bus.token_of(id).unwrap();
    bus.mark_live(id);

    bus.close(id);

    assert_eq!(bus.open_count(), 0);
    assert_eq!(
        bus.transport().poll(),
        vec![TransportEvent::Disconnected { token }]
    );
    assert!(
        bus.open_pane_for_name("helm").is_none(),
        "and the station id resolves to nothing, so a re-open is a new console"
    );
}

#[test]
fn a_console_gets_its_own_document_from_the_armed_template() {
    // The same arming a recreated pane rebuilds from (issue #1125): a host
    // with a client bundle arms it whether or not it was given a `--pane`,
    // because a screen row can open a console at any moment.
    let bus = PaneBus::default();
    let documents = HostedDocuments::default();
    bus.attach_documents(documents.clone());
    bus.arm_recreation(
        "127.0.0.1:8080".to_string(),
        "<html>console</html>".to_string(),
    );

    let (id, url) = bus.open_console("helm");
    let path = bus.document_path(id).expect("the console has a document");
    assert_eq!(
        documents.get(&path).as_deref(),
        Some("<html>console</html>")
    );
    assert!(
        url.starts_with("http://127.0.0.1:8080") && url.contains(&path),
        "the view is sent to this host's own address, at the nonce'd path the \
             document was published under: {url}"
    );
    assert!(
        url.contains(&bus.token_of(id).unwrap()),
        "…carrying the console's own session token, which is how its page \
             identifies as an ordinary participant: {url}"
    );

    // …and closing it takes the document down, exactly as it does for a
    // `--pane`: an unguessable path is no reason to keep serving a console
    // for a participant that has gone.
    bus.close(id);
    assert!(documents.get(&path).is_none());
}
