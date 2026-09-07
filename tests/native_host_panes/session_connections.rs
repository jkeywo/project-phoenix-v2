//! Real native App, Session policy and production adapters; only sockets are fake.

use super::*;
use std::sync::{Arc, Mutex};

use project_phoenix::core::codec::{
    encode_handshake_frame, encode_rendezvous_frame, JsonCodec, MessageCodec,
};
use project_phoenix::core::rendezvous::{
    HandshakeData, HandshakeFrame, RendezvousFrame, JOIN_HANDSHAKE,
};
use project_phoenix::delivery::stamp::DeliveryStamp;
use project_phoenix::native_host::relay_transport::{RelayHostConfig, RelaySocket, RelayTransport};

#[derive(Clone, Default)]
struct Socket {
    incoming: Arc<Mutex<Vec<String>>>,
    outgoing: Arc<Mutex<Vec<String>>>,
}

impl RelaySocket for Socket {
    fn poll(&mut self) -> Vec<String> {
        std::mem::take(&mut *self.incoming.lock().unwrap())
    }
    fn send(&mut self, text: String) {
        self.outgoing.lock().unwrap().push(text);
    }
    fn is_open(&self) -> bool {
        true
    }
    fn close(&mut self) {}
}

impl Socket {
    fn arrive(&self, frame: RendezvousFrame) {
        self.incoming
            .lock()
            .unwrap()
            .push(encode_rendezvous_frame(&frame).unwrap());
    }

    fn payload(&self, peer: &str, payload: String) {
        self.arrive(RendezvousFrame {
            from: Some(peer.into()),
            payload: Some(payload),
            class: Some("reliable".into()),
            ..RendezvousFrame::new("relay")
        });
    }

    fn command(&self, peer: &str, msg: ClientMessage) {
        self.payload(peer, JsonCodec.encode_client(&msg).unwrap());
    }

    fn join(&self, peer: &str, token: &str) {
        self.arrive(RendezvousFrame {
            peer: Some(peer.into()),
            ..RendezvousFrame::new("relay-peer")
        });
        self.payload(
            peer,
            encode_handshake_frame(&HandshakeFrame {
                kind: JOIN_HANDSHAKE.into(),
                data: HandshakeData {
                    stamp: Some(format!(
                        "{}/phoenix-base/1",
                        project_phoenix::core::messages::PROTOCOL_VERSION
                    )),
                    ..Default::default()
                },
            })
            .unwrap(),
        );
        self.command(
            peer,
            ClientMessage::Identify {
                token: token.into(),
                name: "Ada".into(),
            },
        );
    }

    fn leave(&self, peer: &str) {
        self.arrive(RendezvousFrame {
            peer: Some(peer.into()),
            ..RendezvousFrame::new("relay-peer-left")
        });
    }

    fn transport(&self) -> RelayTransport {
        RelayTransport::new(
            self.clone(),
            RelayHostConfig {
                namespace: "client".into(),
                version: None,
                stamp: DeliveryStamp {
                    protocol: project_phoenix::core::messages::PROTOCOL_VERSION,
                    content_id: "phoenix-base".into(),
                    content_epoch: 1,
                },
            },
        )
    }
}

#[test]
fn cross_leg_reconnect_retains_real_station_control_after_stale_commands_and_departure() {
    let mut cfg = crewed_config();
    cfg.solo = true;
    let mut app = build_native_host_app(&cfg, &preload()).unwrap();
    let (station, system, _, _) = two_stations_with_systems(&ship_config(&app));
    let lan = Socket::default();
    let cloud = Socket::default();
    app.insert_resource(NativeTransportLink::new(PairedTransport::new(
        lan.transport(),
        cloud.transport(),
    )));
    pump(&mut app, 8);
    lan.join("peer-A", "crew-A");
    pump(&mut app, 4);
    lan.command(
        "peer-A",
        ClientMessage::SelectStation {
            station: station.clone(),
        },
    );
    pump(&mut app, 4);
    lan.command("peer-A", ClientMessage::SetReady { ready: true });
    pump(&mut app, 4);
    assert!(admits(&mut app, "crew-A", &system));
    let held_rating = station_rating(&mut app, &station);

    cloud.join("peer-A", "crew-A");
    pump(&mut app, 4);
    // These used to reach Session policy through the other relay's private map.
    lan.command("peer-A", ClientMessage::ReleaseStation);
    lan.command("peer-A", ClientMessage::SetReady { ready: false });
    lan.leave("peer-A");
    pump(&mut app, 4);
    assert!(admits(&mut app, "crew-A", &system));
    assert_eq!(station_rating(&mut app, &station), held_rating);
    let sessions = app.world().resource::<project_phoenix::lobby::Sessions>();
    let player = sessions
        .0
        .players()
        .iter()
        .find(|p| p.token == "crew-A")
        .unwrap();
    assert!(player.connected && player.ready);
    assert_eq!(
        player.station.as_ref().map(|s| s.0.as_str()),
        Some(station.as_str())
    );
    cloud.command("peer-A", ClientMessage::ReleaseStation);
    pump(&mut app, 4);
    assert!(
        !admits(&mut app, "crew-A", &system),
        "the current owner's command still reaches admission"
    );
}

#[test]
fn pane_handoff_to_cloud_preserves_control_without_crash_recreation() {
    let mut cfg = crewed_config();
    cfg.solo = true;
    let panes = LocalPanes::open(&["Ada".into()], "127.0.0.1:0");
    let bus = panes.bus.clone();
    let pane = panes.opened[0].id;
    let token = bus.token_of(pane).unwrap();
    cfg.panes = Some(panes);
    let mut app = build_native_host_app(&cfg, &preload()).unwrap();
    let (station, system, _, _) = two_stations_with_systems(&ship_config(&app));
    let cloud = Socket::default();
    app.insert_resource(NativeTransportLink::new(PairedTransport::new(
        bus.transport(),
        cloud.transport(),
    )));
    pump(&mut app, 8);
    bus.mark_live(pane);
    join_claim_ready(
        &mut app,
        &mut |msg| bus.submit(pane, msg).unwrap(),
        &token,
        "Ada",
        &station,
    );
    assert!(admits(&mut app, &token, &system));
    cloud.join("phone", &token);
    pump(&mut app, 4);
    assert!(bus.is_superseded(pane));
    for _ in 0..5 {
        bus.fault(pane, PaneFault::ViewCrashed);
        assert!(service_faults(&bus).is_empty());
        pump(&mut app, 2);
        assert!(admits(&mut app, &token, &system));
    }
    assert!(bus.take_pending_views().is_empty());
    bus.close(pane);
    pump(&mut app, 4);
    assert!(admits(&mut app, &token, &system));
    cloud.leave("phone");
    pump(&mut app, 4);
    assert_eq!(
        station_rating(&mut app, &station).as_deref(),
        Some(project_phoenix::ship::rating::BACKFILL_RATING)
    );
}
