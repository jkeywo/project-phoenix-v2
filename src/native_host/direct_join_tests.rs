//! The in-process rendezvous's protocol half, driven with no sockets at all
//! (issue #1353).
//!
//! The same division `relay_transport` draws and for the same reason: what is
//! worth testing here is the SERVICE's answers — the code lookup, the frame
//! ferry, the class contract, the bounds — and none of that needs a WebSocket.
//! What does need one is that a real client's handshake, framing and close all
//! interoperate, and that is `tests/native_direct_join.rs`, which binds a real
//! host and drives a real `tungstenite` client through it. Unlike
//! `tests/native_relay_live.rs`, that test needs no service to be running, so
//! it is not `#[ignore]`d.

use super::*;

/// A record with no sockets, plus the host's end of its frame queue.
fn service() -> (Arc<Record>, Receiver<String>) {
    budgeted_service(AdmissionBudgets::default())
}

/// The same, with the transport-plane budgets named — the seam the rate-limit
/// tests drive so that a bucket empties in one call instead of in a minute.
fn budgeted_service(budgets: AdmissionBudgets) -> (Arc<Record>, Receiver<String>) {
    let table = JoinCodeTable::read(std::path::Path::new("assets/join/join-codes.toml"))
        .expect("the authored table is checked in");
    let code = table
        .mint_client_code(crate::native_host::join_codes::os_draw)
        .expect("a code is mintable");
    let (tx, rx) = mpsc::channel::<String>();
    let record = Arc::new(Record {
        table,
        code,
        to_host: Mutex::new(tx),
        joined: AtomicUsize::new(0),
        admissions: Admissions::new(budgets),
        peers: Mutex::new(HashMap::new()),
        next_peer: AtomicU64::new(1),
        open: AtomicBool::new(true),
    });
    (record, rx)
}

fn joiner(record: &Arc<Record>) -> Joiner {
    from_source(record, None)
}

/// A joiner that arrived from `source`, for the budgets a reconnect cannot
/// shed. `None` is a connection with no socket under it, which is what every
/// protocol test here is.
fn from_source(record: &Arc<Record>, source: Option<IpAddr>) -> Joiner {
    Joiner {
        id: record.mint_peer_id(),
        joined: false,
        outbox: None,
        lookups: 0,
        source,
        throttle: Duration::ZERO,
        pending: Vec::new(),
        cut: false,
    }
}

fn ip(text: &str) -> Option<IpAddr> {
    Some(text.parse().expect("a test address"))
}

/// One wrong guess from `source`, as a phone's `join` frame would carry it.
fn guess(record: &Arc<Record>, j: &mut Joiner) {
    send_client(
        record,
        j,
        &RendezvousFrame {
            code: Some(CodeField::Typed("XYZABCDE".to_string())),
            ..client("join")
        },
    );
}

/// Everything the joiner has been told since the last look.
fn told(joiner: &mut Joiner) -> Vec<RendezvousFrame> {
    let mut frames: Vec<String> = std::mem::take(&mut joiner.pending);
    if let Some(outbox) = &joiner.outbox {
        frames.extend(outbox.drain());
    }
    frames
        .iter()
        .map(|t| decode_rendezvous_frame(t).expect("the service emits decodable frames"))
        .collect()
}

/// Everything the host half has been told since the last look.
fn host_saw(rx: &Receiver<String>) -> Vec<RendezvousFrame> {
    let mut out = Vec::new();
    while let Ok(text) = rx.try_recv() {
        out.push(decode_rendezvous_frame(&text).expect("the service emits decodable frames"));
    }
    out
}

fn client(kind: &str) -> RendezvousFrame {
    RendezvousFrame::new(kind)
}

fn send_client(record: &Arc<Record>, j: &mut Joiner, frame: &RendezvousFrame) {
    on_client_frame(
        record,
        j,
        &encode_rendezvous_frame(frame).expect("a test frame encodes"),
    );
}

#[test]
fn the_service_opens_with_the_frame_that_makes_a_host_register() {
    // `RelayTransport` sends `host-open` on `ready` and nothing before it, so a
    // service that never says `ready` is a host that never registers and a
    // viewscreen with no code on it.
    let (mut service, code) = DirectJoinService::open(
        JoinCodeTable::read(std::path::Path::new("assets/join/join-codes.toml")).unwrap(),
    )
    .expect("the service opens");
    let first = service.poll();
    assert_eq!(first.len(), 1);
    let frame = decode_rendezvous_frame(&first[0]).unwrap();
    assert_eq!(frame.kind, "ready");
    assert_eq!(frame.v, RENDEZVOUS_PROTOCOL);
    assert_eq!(code.suffix.chars().count(), 8);
    assert!(service.is_open());
}

#[test]
fn the_host_is_answered_with_the_code_this_process_minted() {
    // The one place the two halves meet: the host registers, and the service —
    // which is this same process — hands back the code it minted at bind rather
    // than waiting for a network round trip that has no wire to cross.
    let (mut service, code) = DirectJoinService::open(
        JoinCodeTable::read(std::path::Path::new("assets/join/join-codes.toml")).unwrap(),
    )
    .unwrap();
    service.poll();
    service.send(encode_rendezvous_frame(&RendezvousFrame::host_open("client", None)).unwrap());
    let frames = service.poll();
    let hosted = decode_rendezvous_frame(&frames[0]).unwrap();
    assert_eq!(hosted.kind, "hosted");
    let issued = hosted.code.as_ref().and_then(|c| c.issued()).unwrap();
    assert_eq!(issued.full, code.full);
    assert_eq!(issued.suffix, code.suffix);
}

#[test]
fn a_joiner_with_the_right_code_is_told_the_host_answers_only_on_the_relay() {
    let (record, _rx) = service();
    let mut j = joiner(&record);
    send_client(
        &record,
        &mut j,
        &RendezvousFrame {
            code: Some(CodeField::Typed(record.code.full.clone())),
            ..client("join")
        },
    );
    let frames = told(&mut j);
    assert_eq!(frames[0].kind, "joined");
    assert_eq!(frames[0].peer.as_deref(), Some(j.id.as_str()));
    // Without this a joiner spends the whole 8/16/30 s ladder, four times over,
    // discovering that a host with no WebRTC has no WebRTC.
    assert_eq!(frames[0].transports, vec![TRANSPORT_WS_RELAY.to_string()]);
    assert_eq!(frames[0].admission.as_deref(), Some("open"));
}

#[test]
fn a_bare_suffix_joins_exactly_as_a_scanned_code_does() {
    // Two routes into one record: a QR sends the structured code, a guest
    // reading the viewscreen types the letters.
    let (record, _rx) = service();
    let mut j = joiner(&record);
    send_client(
        &record,
        &mut j,
        &RendezvousFrame {
            code: Some(CodeField::Typed(record.code.suffix.to_lowercase())),
            ..client("join")
        },
    );
    assert_eq!(told(&mut j)[0].kind, "joined");
}

#[test]
fn the_refusals_a_phone_gets_are_the_ones_the_worker_would_have_given() {
    // Worker parity, which is the whole acceptance criterion: a phone must not
    // be able to tell which service answered it. Each reason maps to its own
    // sentence through gui/join-code.js's reasonStringId.
    let (record, _rx) = service();
    let mut j = joiner(&record);
    send_client(
        &record,
        &mut j,
        &RendezvousFrame {
            code: Some(CodeField::Typed("XYZABCDE".to_string())),
            ..client("join")
        },
    );
    let refusal = &told(&mut j)[0];
    assert_eq!(refusal.kind, "error");
    assert_eq!(refusal.request.as_deref(), Some("join"));
    assert_eq!(refusal.reason.as_deref(), Some("unknown"));

    // A frame from another vocabulary revision is refused, not guessed at —
    // and it ends the connection, because every later frame gets the same
    // answer.
    let mut j = joiner(&record);
    let mut ahead = client("join");
    ahead.v = RENDEZVOUS_PROTOCOL + 1;
    ahead.code = Some(CodeField::Typed(record.code.full.clone()));
    send_client(&record, &mut j, &ahead);
    let refusal = &told(&mut j)[0];
    assert_eq!(refusal.reason.as_deref(), Some("unsupported-protocol"));
    assert!(j.cut, "the socket is closed after the refusal is sent");
}

#[test]
fn a_socket_may_not_walk_the_suffix_space() {
    // The authored per-connection cap, which ends ONE abusive connection. It
    // is charged to a socket, so it is not by itself a rate limit — the
    // budgets that survive a reconnect are asserted below.
    let (record, _rx) = budgeted_service(AdmissionBudgets {
        // Big enough that this test measures the per-CONNECTION cap and only
        // that: the per-source bucket has its own test.
        guess_burst: u32::MAX,
        ..AdmissionBudgets::default()
    });
    let mut j = joiner(&record);
    for _ in 0..record.table.limits.max_lookups_per_connection {
        guess(&record, &mut j);
    }
    told(&mut j);
    guess(&record, &mut j);
    assert_eq!(told(&mut j)[0].reason.as_deref(), Some("too-many-attempts"));
    assert!(j.cut);
}

#[test]
fn the_relay_is_gated_on_having_joined_first() {
    // The relay is the fallback for a direct link that could not be built, not
    // a way to reach a host without ever resolving its code.
    let (record, _rx) = service();
    let mut j = joiner(&record);
    send_client(&record, &mut j, &client("relay-open"));
    assert_eq!(told(&mut j)[0].reason.as_deref(), Some("not-joined"));
    send_client(
        &record,
        &mut j,
        &RendezvousFrame {
            class: Some(CLASS_RELIABLE.to_string()),
            payload: Some("{}".to_string()),
            ..client("relay")
        },
    );
    assert_eq!(told(&mut j)[0].reason.as_deref(), Some("not-relaying"));
}

#[test]
fn attaching_tells_the_host_before_it_tells_the_joiner() {
    // Load-bearing order: the joiner's very next act is to put its
    // compatibility handshake on the relay, and a host that had not yet built
    // its side of the pair would drop it.
    let (record, rx) = service();
    let mut j = joiner(&record);
    send_client(
        &record,
        &mut j,
        &RendezvousFrame {
            code: Some(CodeField::Typed(record.code.full.clone())),
            ..client("join")
        },
    );
    told(&mut j);
    send_client(&record, &mut j, &client("relay-open"));
    let to_host = host_saw(&rx);
    assert_eq!(to_host[0].kind, "relay-peer");
    assert_eq!(to_host[0].peer.as_deref(), Some(j.id.as_str()));
    assert!(
        to_host[0].limits.is_some(),
        "both ends are told one ceiling"
    );
    let to_joiner = told(&mut j);
    assert_eq!(to_joiner[0].kind, "relay-ready");
    assert_eq!(to_joiner[0].limits, to_host[0].limits);
}

#[test]
fn a_game_frame_crosses_in_both_directions() {
    let (record, rx) = service();
    let mut j = joiner(&record);
    send_client(
        &record,
        &mut j,
        &RendezvousFrame {
            code: Some(CodeField::Typed(record.code.full.clone())),
            ..client("join")
        },
    );
    send_client(&record, &mut j, &client("relay-open"));
    told(&mut j);
    host_saw(&rx);

    send_client(
        &record,
        &mut j,
        &RendezvousFrame {
            class: Some(CLASS_RELIABLE.to_string()),
            payload: Some(r#"{"type":"Identify"}"#.to_string()),
            ..client("relay")
        },
    );
    let up = host_saw(&rx);
    assert_eq!(up[0].kind, "relay");
    assert_eq!(up[0].from.as_deref(), Some(j.id.as_str()));
    assert_eq!(up[0].payload.as_deref(), Some(r#"{"type":"Identify"}"#));

    // …and back down, through the same `RelaySocket::send` the host half uses.
    let outbox = record
        .peers
        .lock()
        .unwrap()
        .get(&j.id)
        .cloned()
        .expect("attached");
    let down = RendezvousFrame::relay(
        &j.id,
        CLASS_RELIABLE,
        r#"{"type":"GameStarted"}"#.to_string(),
    );
    let carried = encode_rendezvous_frame(&RendezvousFrame {
        from: Some(HOST_PEER.to_string()),
        class: down.class.clone(),
        payload: down.payload.clone(),
        ..RendezvousFrame::new("relay")
    })
    .unwrap();
    let _ = outbox.push(CLASS_RELIABLE, carried);
    let seen = told(&mut j);
    assert_eq!(seen[0].kind, "relay");
    assert_eq!(
        seen[0].payload.as_deref(),
        Some(r#"{"type":"GameStarted"}"#)
    );
}

#[test]
fn the_snapshot_class_stays_lossy_and_the_reliable_class_stays_ordered() {
    // The class contract, kept on the SERVICE side: a WebSocket is reliable and
    // ordered, so a mailbox that queued snapshots behind a backlog would
    // silently upgrade the lossy class into a reliable one and reintroduce the
    // head-of-line blocking it exists to avoid.
    let (record, _rx) = service();
    let outbox = PeerOutbox::new(&record.table.limits);
    let depth = record.table.limits.max_relay_queue_snapshot;
    for i in 0..depth + 5 {
        outbox.push(CLASS_SNAPSHOT, format!("snap-{i}"));
    }
    let drained = outbox.drain();
    assert_eq!(drained.len(), depth, "the queue holds its authored depth");
    assert_eq!(
        drained[0], "snap-5",
        "the OLDEST snapshots go: a late one is worthless, the next tick supersedes it"
    );
    assert_eq!(outbox.dropped.load(Ordering::Relaxed), 5);

    // Reliable frames keep their order and are never shed…
    let outbox = PeerOutbox::new(&record.table.limits);
    for i in 0..10 {
        outbox.push(CLASS_RELIABLE, format!("cmd-{i}"));
    }
    let drained = outbox.drain();
    assert_eq!(drained.len(), 10);
    assert_eq!(drained[0], "cmd-0");
    assert_eq!(drained[9], "cmd-9");

    // …until the queue passes its bound, which ends the session rather than
    // becoming a quietly lossy reliable channel.
    let outbox = PeerOutbox::new(&record.table.limits);
    let mut overflowed = false;
    for i in 0..record.table.limits.max_relay_queue_reliable + 2 {
        if matches!(
            outbox.push(CLASS_RELIABLE, format!("cmd-{i}")),
            Enqueued::Overflowed
        ) {
            overflowed = true;
        }
    }
    assert!(
        overflowed,
        "a full reliable queue is a dead session, said so"
    );
}

#[test]
fn reliable_frames_leave_before_snapshots_so_a_command_never_trails_its_effect() {
    let (record, _rx) = service();
    let outbox = PeerOutbox::new(&record.table.limits);
    outbox.push(CLASS_SNAPSHOT, "snap".to_string());
    outbox.push(CLASS_RELIABLE, "cmd".to_string());
    assert_eq!(outbox.drain(), vec!["cmd".to_string(), "snap".to_string()]);
}

#[test]
fn a_frame_for_a_peer_that_has_gone_is_one_refusal_and_not_a_lost_crew() {
    // The defect this shape exists to avoid: a host broadcasting to
    // `Target::All` addresses a peer that detached a tick ago on every single
    // departure. `RelayTransport` treats `no-peer` as a notice; treating it as
    // relay loss disconnected everybody else.
    let (mut service, _code) = DirectJoinService::open(
        JoinCodeTable::read(std::path::Path::new("assets/join/join-codes.toml")).unwrap(),
    )
    .unwrap();
    service.poll();
    service.send(
        encode_rendezvous_frame(&RendezvousFrame::relay(
            "lan-404",
            CLASS_RELIABLE,
            "{}".to_string(),
        ))
        .unwrap(),
    );
    let frames: Vec<RendezvousFrame> = service
        .poll()
        .iter()
        .map(|t| decode_rendezvous_frame(t).unwrap())
        .collect();
    assert_eq!(frames[0].kind, "error");
    assert_eq!(frames[0].reason.as_deref(), Some("no-peer"));
    assert_eq!(frames[0].request.as_deref(), Some("relay"));
}

#[test]
fn the_host_evicting_a_peer_says_so_in_band_before_detaching_it() {
    // The reserved-token refusal and the duplicate-token sever both arrive as
    // `relay-close`. Left one-sided, the evicted phone would keep a status line
    // reading "connected" for a link this host had already walked away from.
    let (record, rx) = service();
    let mut j = joiner(&record);
    send_client(
        &record,
        &mut j,
        &RendezvousFrame {
            code: Some(CodeField::Typed(record.code.full.clone())),
            ..client("join")
        },
    );
    send_client(&record, &mut j, &client("relay-open"));
    told(&mut j);
    host_saw(&rx);

    DirectJoinService::close_peer(&record, &j.id, "host-closed");
    let seen = told(&mut j);
    assert_eq!(seen[0].kind, "relay-closed");
    assert_eq!(seen[0].reason.as_deref(), Some("host-closed"));
    assert!(
        record.peers.lock().unwrap().is_empty(),
        "and the record stops carrying it"
    );
    let to_host = host_saw(&rx);
    assert_eq!(to_host[0].kind, "relay-peer-left");
}

#[test]
fn a_relay_frame_larger_than_the_authored_ceiling_is_refused_not_carried() {
    let (record, rx) = service();
    let mut j = joiner(&record);
    send_client(
        &record,
        &mut j,
        &RendezvousFrame {
            code: Some(CodeField::Typed(record.code.full.clone())),
            ..client("join")
        },
    );
    send_client(&record, &mut j, &client("relay-open"));
    told(&mut j);
    host_saw(&rx);
    send_client(
        &record,
        &mut j,
        &RendezvousFrame {
            class: Some(CLASS_RELIABLE.to_string()),
            payload: Some("x".repeat(record.table.limits.max_relay_frame_bytes + 1)),
            ..client("relay")
        },
    );
    assert_eq!(told(&mut j)[0].reason.as_deref(), Some("relay-too-large"));
    assert!(host_saw(&rx).is_empty(), "nothing reached the simulation");
}

#[test]
fn a_verb_this_service_does_not_have_is_refused_rather_than_ignored() {
    let (record, _rx) = service();
    let mut j = joiner(&record);
    // A native host has no WebRTC and says so on every `joined`; a signal frame
    // means a joiner offered anyway, and there is nobody to carry it to.
    send_client(&record, &mut j, &client("signal"));
    assert_eq!(told(&mut j)[0].reason.as_deref(), Some("no-peer"));
    send_client(&record, &mut j, &client("rotate"));
    let seen = told(&mut j);
    assert_eq!(seen[0].reason.as_deref(), Some("malformed"));
    assert_eq!(seen[0].request.as_deref(), Some("rotate"));
}

#[test]
fn every_peer_id_this_leg_mints_is_marked_as_this_legs_own() {
    // Namespace separation for a host running BOTH legs: a worker-minted UUID
    // and a directly-accepted joiner can never be read as the same peer.
    let (record, _rx) = service();
    let first = record.mint_peer_id();
    let second = record.mint_peer_id();
    assert!(first.starts_with(PEER_PREFIX));
    assert_ne!(first, second);
}

#[test]
fn closing_the_service_shuts_every_joiner_and_reports_the_link_down() {
    let (mut service, _code) = DirectJoinService::open(
        JoinCodeTable::read(std::path::Path::new("assets/join/join-codes.toml")).unwrap(),
    )
    .unwrap();
    let record = Arc::clone(&service.record);
    let outbox = Arc::new(PeerOutbox::new(&record.table.limits));
    record
        .peers
        .lock()
        .unwrap()
        .insert("lan-1".to_string(), Arc::clone(&outbox));
    service.close();
    assert!(!service.is_open());
    assert!(
        outbox.is_closed(),
        "each joiner's thread notices and closes"
    );
}

// ── The budgets a reconnect cannot shed ─────────────────────────────────────

#[test]
fn a_wrong_guess_budget_outlives_the_connection_that_spent_it() {
    // THE regression. `max_lookups_per_connection` is charged to a `Joiner`,
    // and a `Joiner` is exactly what a guesser discards: sixty guesses, close,
    // dial again — measured at ~2,340 wrong guesses a second. The per-source
    // bucket lives on the RECORD, so the guess past the budget is refused
    // however many sockets the ones before it arrived on.
    let (record, _rx) = budgeted_service(AdmissionBudgets {
        guess_burst: 3,
        // Long enough that nothing refills during this test: this one asserts
        // the bucket empties, the next asserts it fills again.
        guess_refill: Duration::from_secs(3600),
        ..AdmissionBudgets::default()
    });
    let hostile = ip("198.51.100.7");
    for attempt in 0..3 {
        // A FRESH connection every time, which is the whole point.
        let mut j = from_source(&record, hostile);
        guess(&record, &mut j);
        assert_eq!(
            told(&mut j)[0].reason.as_deref(),
            Some("unknown"),
            "guess {attempt} is inside the budget"
        );
        assert!(!j.cut, "and the connection is not yet cut");
    }
    let mut j = from_source(&record, hostile);
    guess(&record, &mut j);
    assert_eq!(
        told(&mut j)[0].reason.as_deref(),
        Some("too-many-attempts"),
        "past the budget, on a connection that had itself guessed only once"
    );
    assert!(j.cut);
}

#[test]
fn a_starved_source_is_refused_softly_and_is_guessing_again_a_moment_later() {
    // Never a ban. A whole crew can share one address — a phone hotspot, a
    // venue router, a port-forward — so a lockout that did not lift would be a
    // self-inflicted outage waiting for one clumsy typist.
    let (record, _rx) = budgeted_service(AdmissionBudgets {
        guess_burst: 1,
        guess_refill: Duration::from_millis(20),
        ..AdmissionBudgets::default()
    });
    let crew = ip("192.168.1.44");
    for _ in 0..2 {
        let mut j = from_source(&record, crew);
        guess(&record, &mut j);
        told(&mut j);
    }
    let mut j = from_source(&record, crew);
    guess(&record, &mut j);
    assert_eq!(told(&mut j)[0].reason.as_deref(), Some("too-many-attempts"));

    std::thread::sleep(Duration::from_millis(120));
    let mut j = from_source(&record, crew);
    guess(&record, &mut j);
    assert_eq!(
        told(&mut j)[0].reason.as_deref(),
        Some("unknown"),
        "the bucket refilled on a clock, with nobody having to ask"
    );
}

#[test]
fn a_right_code_costs_nothing_so_the_crew_joins_through_a_guessers_noise() {
    // The property the soft limits exist for, at the sharpest angle: the
    // guesser and the crew member are the SAME address — one NAT, or a hostile
    // page open in a crew member's own browser, which is the case the
    // no-Origin decision leaves standing. Only FAILED lookups are charged, so
    // a guest who types the code correctly never touches the emptied budget.
    let (record, _rx) = budgeted_service(AdmissionBudgets {
        guess_burst: 2,
        guess_refill: Duration::from_secs(3600),
        ..AdmissionBudgets::default()
    });
    let shared = ip("203.0.113.9");
    for _ in 0..6 {
        let mut j = from_source(&record, shared);
        guess(&record, &mut j);
        told(&mut j);
    }
    let mut crew = from_source(&record, shared);
    send_client(
        &record,
        &mut crew,
        &RendezvousFrame {
            code: Some(CodeField::Typed(record.code.full.clone())),
            ..client("join")
        },
    );
    let seen = told(&mut crew);
    assert_eq!(seen[0].kind, "joined", "got {seen:?}");
    assert!(!crew.cut);
}

#[test]
fn one_sources_exhausted_budget_is_not_another_sources_problem() {
    let (record, _rx) = budgeted_service(AdmissionBudgets {
        guess_burst: 1,
        guess_refill: Duration::from_secs(3600),
        ..AdmissionBudgets::default()
    });
    for _ in 0..3 {
        let mut j = from_source(&record, ip("198.51.100.7"));
        guess(&record, &mut j);
        told(&mut j);
    }
    let mut neighbour = from_source(&record, ip("192.168.1.50"));
    guess(&record, &mut neighbour);
    assert_eq!(
        told(&mut neighbour)[0].reason.as_deref(),
        Some("unknown"),
        "a budget is per source, not a service-wide door"
    );
}

#[test]
fn the_breaker_slows_every_lookup_answer_once_the_service_is_being_guessed_at() {
    // The layer that covers what a per-source budget cannot: a guesser spread
    // across addresses, or spoofing them. The delay is charged to the ANSWER,
    // which is the guesser's own thread, and it is capped low enough that a
    // legitimate guest caught in the middle of one waits once and gets in.
    let budgets = AdmissionBudgets {
        guess_burst: u32::MAX,
        breaker_free: 2,
        breaker_ramp: 2,
        breaker_step: Duration::from_millis(50),
        breaker_max: Duration::from_millis(150),
        ..AdmissionBudgets::default()
    };
    let (record, _rx) = budgeted_service(budgets.clone());
    let mut seen: Vec<Duration> = Vec::new();
    for n in 0..12 {
        // Every guess from its own address, so it is not the per-source bucket
        // being measured here.
        let mut j = from_source(&record, ip(&format!("198.51.100.{n}")));
        guess(&record, &mut j);
        told(&mut j);
        seen.push(j.throttle);
    }
    assert_eq!(
        seen[0],
        Duration::ZERO,
        "nobody waits for the first guesses"
    );
    assert_eq!(seen[1], Duration::ZERO);
    assert!(seen[2] > Duration::ZERO, "the ramp starts: {seen:?}");
    assert!(seen[3] >= seen[2], "and it ramps: {seen:?}");
    assert_eq!(
        *seen.last().unwrap(),
        budgets.breaker_max,
        "…to a stated ceiling, not to a hang: {seen:?}"
    );

    // …and a CORRECT code is answered on the same clock. Answering a right
    // code faster than a wrong one during an attack would be a timing oracle
    // over the very keyspace the delay is protecting.
    let mut crew = from_source(&record, ip("192.168.1.60"));
    send_client(
        &record,
        &mut crew,
        &RendezvousFrame {
            code: Some(CodeField::Typed(record.code.full.clone())),
            ..client("join")
        },
    );
    assert_eq!(told(&mut crew)[0].kind, "joined");
    assert_eq!(crew.throttle, budgets.breaker_max);
}

#[test]
fn silence_has_its_own_budget_and_cannot_spend_the_rooms_places() {
    // The secondary finding: un-joined sockets were counted against the
    // authored `max_peers_per_record`, so one device holding thirty-two silent
    // sockets refused the crew standing in the room with `join-sockets-full`.
    let (record, _rx) = budgeted_service(AdmissionBudgets {
        unjoined_per_source: 2,
        unjoined_total: 3,
        ..AdmissionBudgets::default()
    });
    let hoarder = ip("198.51.100.7");
    assert!(record.admissions.take_socket(hoarder).is_ok());
    assert!(record.admissions.take_socket(hoarder).is_ok());
    assert_eq!(
        record.admissions.take_socket(hoarder),
        Err("join-sockets-busy"),
        "one source cannot hold every silent slot"
    );
    // …and the crew, on another address, still gets in.
    assert!(record.admissions.take_socket(ip("192.168.1.70")).is_ok());
    assert_eq!(
        record.admissions.take_socket(ip("192.168.1.71")),
        Err("join-sockets-full"),
        "the global bound on silence is its own number, not the crew's"
    );
    // A slot frees the instant its socket closes: nothing here is a ban.
    record.admissions.release_socket(hoarder);
    assert!(record.admissions.take_socket(ip("192.168.1.71")).is_ok());

    // And the record's own crew cap is untouched by any of it, which is the
    // whole point of the split.
    assert_eq!(record.joined.load(Ordering::Relaxed), 0);
    assert!(record.table.limits.max_peers_per_record >= 32);
}

#[test]
fn the_source_table_is_bounded_and_the_breaker_carries_what_it_cannot_track() {
    // A per-source table is memory a stranger can spend, so it is swept of
    // settled sources rather than grown. What a flood of addresses meets is
    // the global breaker, which is one of the reasons that layer exists.
    let (record, _rx) = budgeted_service(AdmissionBudgets {
        guess_burst: 0,
        guess_refill: Duration::from_millis(1),
        tracked_sources: 8,
        breaker_free: 4,
        breaker_ramp: 4,
        breaker_step: Duration::from_millis(1),
        breaker_max: Duration::from_millis(4),
        ..AdmissionBudgets::default()
    });
    for n in 0..64 {
        let mut j = from_source(&record, ip(&format!("198.51.100.{n}")));
        guess(&record, &mut j);
        told(&mut j);
    }
    assert!(
        record.admissions.sources.lock().unwrap().len() <= 8,
        "the table stays at its bound however many addresses arrive"
    );
    let mut j = from_source(&record, ip("192.168.1.80"));
    guess(&record, &mut j);
    assert!(
        j.throttle > Duration::ZERO,
        "and the flood is answered by the breaker instead"
    );
}
