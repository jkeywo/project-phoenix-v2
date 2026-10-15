use super::*;

#[test]
fn it_upgrades_a_service_base_to_the_host_endpoint() {
    // The same upgrade gui/rendezvous-transport.js's socketUrl() performs.
    // Asked of the operator instead, it would be one more thing to get
    // wrong on a checklist that already has the origin allowlist on it.
    assert_eq!(
        host_socket_url("https://phoenix-rendezvous.project-phoenix.workers.dev").unwrap(),
        "wss://phoenix-rendezvous.project-phoenix.workers.dev/v1/host"
    );
    // A local `wrangler dev` is plain HTTP and must stay plain.
    assert_eq!(
        host_socket_url("http://localhost:8787/").unwrap(),
        "ws://localhost:8787/v1/host"
    );
    // Already a socket scheme: left alone rather than upgraded twice.
    assert_eq!(
        host_socket_url("wss://example.test").unwrap(),
        "wss://example.test/v1/host"
    );
    assert_eq!(
        join_socket_url("https://phoenix-rendezvous.project-phoenix.workers.dev/").unwrap(),
        "wss://phoenix-rendezvous.project-phoenix.workers.dev/v1/join"
    );
    assert_eq!(
        join_socket_url("http://localhost:8787").unwrap(),
        "ws://localhost:8787/v1/join"
    );
}

#[test]
fn the_redial_ladder_doubles_and_then_holds_at_a_minute() {
    // The same shape gui/connection-manager.js's nextBackoffDelay gives the
    // browser host for the identical event. A service that is down for an
    // hour is still asked once a minute; a blip is retried in a second.
    assert_eq!(redial_delay_ms(0), 1_000);
    assert_eq!(redial_delay_ms(1), 2_000);
    assert_eq!(redial_delay_ms(4), 16_000);
    assert_eq!(redial_delay_ms(6), 60_000);
    // And it never runs away: an attempt count that keeps climbing for the
    // rest of the mission must not overflow or stop retrying.
    assert_eq!(redial_delay_ms(40), 60_000);
    assert_eq!(redial_delay_ms(u32::MAX), 60_000);
}

#[test]
fn it_refuses_something_that_is_not_a_service_url() {
    // A typo'd `--rendezvous` must fail at parse with a stated reason, not
    // as a connection attempt to a hostname made of the whole argument.
    for bad in [
        "",
        "   ",
        "phoenix-rendezvous.workers.dev",
        "ftp://x",
        "https://",
    ] {
        assert!(
            host_socket_url(bad).is_err(),
            "{bad:?} is not a rendezvous base"
        );
    }
}

#[test]
fn lifecycle_edges_survive_a_redial_that_finishes_before_poll() {
    let (inbound, receive) = mpsc::channel();
    let (outbound, _send_queue) = mpsc::channel();
    let mut socket = WsRelaySocket {
        inbound: std::sync::Mutex::new(receive),
        ingress: Arc::new(IngressGate::default()),
        outbound,
        open: Arc::new(AtomicBool::new(true)),
        queued: Arc::new(AtomicUsize::new(0)),
        shutdown: Arc::new(AtomicBool::new(false)),
    };
    socket.ingress.emit(&inbound, RelaySocketEvent::Closed);
    socket.ingress.emit(&inbound, RelaySocketEvent::Opened);
    socket
        .ingress
        .emit(&inbound, RelaySocketEvent::Text("ready".into()));
    assert!(socket.is_open());
    assert_eq!(
        socket.poll_events(),
        [
            RelaySocketEvent::Closed,
            RelaySocketEvent::Opened,
            RelaySocketEvent::Text("ready".into())
        ]
    );
    assert!(socket.poll_events().is_empty());
}

#[test]
fn ordinary_work_arriving_between_optional_sends_is_next_eligible_packet() {
    let (send, receive) = mpsc::channel();
    let mut pending = std::collections::VecDeque::new();
    send.send(QueuedText {
        text: "optional-1".into(),
        optional: true,
    })
    .unwrap();
    send.send(QueuedText {
        text: "optional-2".into(),
        optional: true,
    })
    .unwrap();
    assert_eq!(
        outbound_next(&receive, &mut pending),
        (Some("optional-1".into()), false)
    );
    send.send(QueuedText {
        text: "late-command".into(),
        optional: false,
    })
    .unwrap();
    send.send(QueuedText {
        text: "late-game-snapshot".into(),
        optional: false,
    })
    .unwrap();
    assert_eq!(
        outbound_next(&receive, &mut pending),
        (Some("late-command".into()), false)
    );
    assert_eq!(
        outbound_next(&receive, &mut pending),
        (Some("late-game-snapshot".into()), false)
    );
    assert_eq!(
        outbound_next(&receive, &mut pending),
        (Some("optional-2".into()), false)
    );
}
#[test]
fn optional_socket_send_uses_actual_open_state_and_queued_bytes() {
    let (_in_tx, in_rx) = mpsc::channel();
    let (out_tx, out_rx) = mpsc::channel();
    let mut socket = WsRelaySocket {
        inbound: std::sync::Mutex::new(in_rx),
        ingress: Arc::new(IngressGate::default()),
        outbound: out_tx,
        open: Arc::new(AtomicBool::new(true)),
        queued: Arc::new(AtomicUsize::new(4)),
        shutdown: Arc::new(AtomicBool::new(false)),
    };
    assert!(!socket.try_send_snapshot("12345".into(), 8));
    assert!(socket.try_send_snapshot("1234".into(), 8));
    assert_eq!(socket.buffered_bytes(), 8);
    assert_eq!(
        out_rx.try_recv().unwrap(),
        QueuedText {
            text: "1234".into(),
            optional: true
        }
    );
    socket.open.store(false, Ordering::Relaxed);
    assert!(!socket.snapshot_only_supported());
    assert!(!socket.try_send_snapshot("x".into(), 100));
}

#[test]
fn genuine_scoped_flood_is_bounded_before_inbound_channel_and_ordinary_work_survives() {
    fn classify(raw: &str) -> IngressClass {
        match raw {
            "media" => IngressClass::Scoped,
            "invalid-media" => IngressClass::Refuse,
            _ => IngressClass::Ordinary,
        }
    }
    let gate = IngressGate::default();
    *gate.classifier.lock().unwrap() = Some(classify);
    let (send, receive) = mpsc::channel();
    for _ in 0..1000 {
        assert!(gate.emit(&send, RelaySocketEvent::Text("media".into())));
    }
    assert_eq!(gate.scoped.load(Ordering::Relaxed), 64);
    gate.emit(&send, RelaySocketEvent::Text("invalid-media".into()));
    gate.emit(&send, RelaySocketEvent::Closed);
    gate.emit(&send, RelaySocketEvent::Opened);
    gate.emit(&send, RelaySocketEvent::Text("Identify".into()));
    gate.emit(&send, RelaySocketEvent::Text("command".into()));
    let mut events = Vec::new();
    for item in receive.try_iter() {
        gate.release(&item);
        events.push(item.event);
    }
    assert_eq!(events.len(), 68);
    assert_eq!(gate.scoped.load(Ordering::Relaxed), 0);
    assert!(matches!(&events[66],RelaySocketEvent::Text(text) if text=="Identify"));
    assert!(matches!(&events[67],RelaySocketEvent::Text(text) if text=="command"));
    drop(receive);
    assert!(!gate.emit(&send, RelaySocketEvent::Text("media".into())));
    assert_eq!(gate.scoped.load(Ordering::Relaxed), 0);
}
