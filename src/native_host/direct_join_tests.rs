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
        sockets: AtomicUsize::new(0),
        peers: Mutex::new(HashMap::new()),
        next_peer: AtomicU64::new(1),
        open: AtomicBool::new(true),
    });
    (record, rx)
}

fn joiner(record: &Arc<Record>) -> Joiner {
    Joiner {
        id: record.mint_peer_id(),
        joined: false,
        outbox: None,
        lookups: 0,
        pending: Vec::new(),
        cut: false,
    }
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
    assert_eq!(code.suffix.chars().count(), 5);
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
fn a_bare_five_letter_suffix_joins_exactly_as_a_scanned_code_does() {
    // Two routes into one record: a QR sends the structured code, a guest
    // reading the viewscreen types five letters.
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
            code: Some(CodeField::Typed("XYZAB".to_string())),
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
    // The suffix space is 25^5 and codes are private, so an unbounded socket
    // could try every one of them. The authored per-connection cap is what
    // stops it, and it keeps refusing afterwards.
    let (record, _rx) = service();
    let mut j = joiner(&record);
    for _ in 0..record.table.limits.max_lookups_per_connection {
        send_client(
            &record,
            &mut j,
            &RendezvousFrame {
                code: Some(CodeField::Typed("XYZAB".to_string())),
                ..client("join")
            },
        );
    }
    told(&mut j);
    send_client(
        &record,
        &mut j,
        &RendezvousFrame {
            code: Some(CodeField::Typed("XYZAB".to_string())),
            ..client("join")
        },
    );
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
