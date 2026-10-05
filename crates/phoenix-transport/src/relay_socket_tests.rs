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
        outbound,
        open: Arc::new(AtomicBool::new(true)),
        queued: Arc::new(AtomicUsize::new(0)),
        shutdown: Arc::new(AtomicBool::new(false)),
    };
    inbound.send(RelaySocketEvent::Closed).unwrap();
    inbound.send(RelaySocketEvent::Opened).unwrap();
    inbound
        .send(RelaySocketEvent::Text("ready".into()))
        .unwrap();
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
