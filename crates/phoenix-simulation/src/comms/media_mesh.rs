//! Ephemeral selected-call packets on the existing authenticated host mesh.
//! Never a MeshFrame/continuation stream: no commands, tokens or retained bytes.
use super::media_session::EligibleMediaCall;
use bevy::prelude::Resource;
use phoenix_model::comms_media::{MediaPacket, MediaPacketKind, MAX_MEDIA_JSON_BYTES};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaHostFrame {
    pub m: u32,
    pub t: String,
    pub tick: Option<u64>,
    pub d: MediaHostBody,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaHostBody {
    pub from: u32,
    pub to: u32,
    pub call_origin_id: String,
    pub generation: Option<u32>,
    /// Only a current authenticated heartbeat may raise a sender's floor.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub generation_floor: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub packet: Option<MediaPacket>,
}
fn is_zero(value: &u32) -> bool {
    *value == 0
}
impl MediaHostFrame {
    pub fn decode(raw: &str) -> Option<Self> {
        if raw.len() > MAX_MEDIA_JSON_BYTES {
            return None;
        }
        let frame: Self = crate::core::codec::from_json(raw).ok()?;
        (frame.m == crate::lockstep::HOST_MESH_PROTOCOL
            && frame.t == "media"
            && frame.tick.is_none()
            && frame.d.from > 0
            && frame.d.to > 0
            && frame.d.from != frame.d.to
            && phoenix_model::comms_channel::identity(&frame.d.call_origin_id)
            && frame.d.generation.is_none_or(|generation| generation > 0)
            && frame.d.packet.as_ref().is_none_or(|packet| {
                frame.d.generation_floor == 0
                    && packet.validate().is_ok()
                    && Some(packet.lease_generation) == frame.d.generation
                    && packet.call_origin_id == frame.d.call_origin_id
            }))
        .then_some(frame)
    }
    pub fn encode(body: MediaHostBody) -> Option<String> {
        let frame = Self {
            m: crate::lockstep::HOST_MESH_PROTOCOL,
            t: "media".into(),
            tick: None,
            d: body,
        };
        let raw = crate::core::codec::to_json(&frame).ok()?;
        (raw.len() <= MAX_MEDIA_JSON_BYTES).then_some(raw)
    }
}
struct RemoteLease {
    generation: u32,
    call: EligibleMediaCall,
    heard_ms: u64,
    sequence: u32,
    arrivals: VecDeque<(u64, usize)>,
    video_ms: Option<u64>,
}
#[derive(Resource, Default)]
pub struct MediaMesh {
    pending: VecDeque<(u32, MediaHostFrame)>,
    outgoing: Vec<String>,
    leases: BTreeMap<String, RemoteLease>,
    high_water: BTreeMap<String, u32>,
    announced_ms: Option<u64>,
    supported: bool,
    heard_routes: BTreeMap<String, (EligibleMediaCall, u64)>,
}
impl MediaMesh {
    pub fn set_supported(&mut self, supported: bool) {
        self.supported = supported;
        if !supported {
            self.invalidate();
        }
    }
    pub(super) fn route_available(&self, call: &EligibleMediaCall, now: u64) -> bool {
        self.supported
            && self
                .heard_routes
                .get(&call.peer_ship)
                .is_some_and(|(heard, time)| heard == call && now.saturating_sub(*time) <= 750)
    }
    pub fn offer(&mut self, authenticated_slot: u32, raw: &str) -> bool {
        if authenticated_slot == 0 || self.pending.len() >= 64 {
            return false;
        }
        let Some(frame) = MediaHostFrame::decode(raw) else {
            return false;
        };
        self.pending.push_back((authenticated_slot, frame));
        true
    }
    pub fn invalidate(&mut self) {
        self.pending.clear();
        self.outgoing.clear();
        self.leases.clear();
        self.heard_routes.clear();
        self.announced_ms = None;
    }
    pub fn take_outgoing(&mut self) -> Vec<String> {
        std::mem::take(&mut self.outgoing)
    }
    pub(super) fn drain(&mut self) -> Vec<(u32, MediaHostFrame)> {
        self.pending.drain(..).collect()
    }
    pub(super) fn push(&mut self, body: MediaHostBody) {
        if self.outgoing.len() < 64 {
            if let Some(raw) = MediaHostFrame::encode(body) {
                self.outgoing.push(raw);
            }
        }
    }
    pub(super) fn heartbeat(&mut self, now: u64) -> bool {
        if !self.supported {
            return false;
        }
        if self
            .announced_ms
            .is_some_and(|last| now.saturating_sub(last) < 250)
        {
            false
        } else {
            self.announced_ms = Some(now);
            true
        }
    }
    pub(super) fn reconcile(
        &mut self,
        now: u64,
        mut permitted: impl FnMut(&EligibleMediaCall) -> bool,
    ) {
        self.heard_routes
            .retain(|_, (call, heard)| now.saturating_sub(*heard) <= 750 && permitted(call));
        self.leases
            .retain(|_, lease| now.saturating_sub(lease.heard_ms) <= 750 && permitted(&lease.call));
    }
    pub(super) fn generation(&self, call: &EligibleMediaCall) -> Option<u32> {
        let lease = self.leases.get(&call.peer_ship)?;
        (lease.call == *call).then_some(lease.generation)
    }
    pub(super) fn required_floor(&self, call: &EligibleMediaCall) -> u32 {
        if self.generation(call).is_some() {
            0
        } else {
            self.high_water.get(&call.peer_ship).copied().unwrap_or(0)
        }
    }
    pub(super) fn announce(&mut self, call: &EligibleMediaCall, generation: Option<u32>, now: u64) {
        self.heard_routes
            .insert(call.peer_ship.clone(), (call.clone(), now));
        let Some(generation) = generation else {
            self.leases.remove(&call.peer_ship);
            return;
        };
        if let Some(active) = self.leases.get_mut(&call.peer_ship) {
            if active.generation == generation && active.call == *call {
                active.heard_ms = now;
                return;
            }
        }
        if self
            .high_water
            .get(&call.peer_ship)
            .is_some_and(|prior| generation <= *prior)
        {
            return;
        }
        self.high_water.insert(call.peer_ship.clone(), generation);
        self.leases.insert(
            call.peer_ship.clone(),
            RemoteLease {
                generation,
                call: call.clone(),
                heard_ms: now,
                sequence: 0,
                arrivals: VecDeque::new(),
                video_ms: None,
            },
        );
    }
    pub(super) fn admit(
        &mut self,
        call: &EligibleMediaCall,
        packet: &MediaPacket,
        now: u64,
    ) -> bool {
        use base64::Engine;
        let Some(lease) = self.leases.get_mut(&call.peer_ship) else {
            return false;
        };
        if lease.call != *call
            || lease.generation != packet.lease_generation
            || packet.sequence <= lease.sequence
            || packet.call_origin_id != call.origin
            || !call.mode.permits(packet.kind)
            || packet.validate().is_err()
        {
            return false;
        }
        while lease
            .arrivals
            .front()
            .is_some_and(|(time, _)| now.saturating_sub(*time) >= 1000)
        {
            lease.arrivals.pop_front();
        }
        if lease.arrivals.len() >= 16
            || lease.arrivals.iter().map(|(_, bytes)| bytes).sum::<usize>() + packet.data.len()
                > 80 * 1024
            || (packet.kind == MediaPacketKind::Jpeg
                && lease
                    .video_ms
                    .is_some_and(|time| now.saturating_sub(time) < 500))
        {
            return false;
        }
        let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(&packet.data) else {
            return false;
        };
        if packet.validate_decoded(&decoded).is_err() {
            return false;
        }
        lease.sequence = packet.sequence;
        lease.arrivals.push_back((now, packet.data.len()));
        if packet.kind == MediaPacketKind::Jpeg {
            lease.video_ms = Some(now);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use phoenix_model::comms_media::MediaMode;
    fn call() -> EligibleMediaCall {
        EligibleMediaCall {
            origin: "accepted-call".into(),
            local_ship: "own".into(),
            peer_ship: "peer".into(),
            mode: MediaMode::Audio,
        }
    }
    fn packet(generation: u32, sequence: u32) -> MediaPacket {
        MediaPacket {
            call_origin_id: "accepted-call".into(),
            lease_generation: generation,
            sequence,
            capture_ms: sequence * 80,
            kind: MediaPacketKind::Pcm16Mono16k,
            data: format!("{}AA==", "A".repeat(3412)),
        }
    }
    #[test]
    fn restore_keeps_remote_floor_and_queued_frames_cannot_revive_a_lease() {
        let mut mesh = MediaMesh::default();
        let call = call();
        mesh.set_supported(true);
        mesh.announce(&call, Some(7), 0);
        assert_eq!(mesh.required_floor(&call), 0);
        assert!(mesh.admit(&call, &packet(7, 1), 0));
        mesh.invalidate();
        assert_eq!(mesh.required_floor(&call), 7);
        mesh.announce(&call, Some(7), 1);
        assert_eq!(mesh.generation(&call), None);
        assert!(!mesh.admit(&call, &packet(7, 2), 1));
        mesh.announce(&call, Some(8), 2);
        assert_eq!(mesh.required_floor(&call), 0);
        assert!(!mesh.admit(&call, &packet(7, 2), 2));
        assert!(mesh.admit(&call, &packet(8, 1), 2));
        assert!(!mesh.admit(&call, &packet(8, 1), 3));
        mesh.reconcile(753, |_| true);
        assert!(!mesh.route_available(&call, 753));
        assert_eq!(mesh.required_floor(&call), 8);
    }
    #[test]
    fn only_bounded_heartbeat_can_report_a_floor_not_a_frame_packet() {
        let mut body = MediaHostBody {
            from: 1,
            to: 2,
            call_origin_id: "accepted-call".into(),
            generation: Some(7),
            generation_floor: 9,
            packet: None,
        };
        let raw = MediaHostFrame::encode(body.clone()).unwrap();
        assert_eq!(MediaHostFrame::decode(&raw).unwrap().d.generation_floor, 9);
        body.packet = Some(packet(7, 1));
        assert!(MediaHostFrame::decode(&MediaHostFrame::encode(body.clone()).unwrap()).is_none());
        body.generation_floor = 0;
        assert!(MediaHostFrame::decode(&MediaHostFrame::encode(body).unwrap()).is_some());
        let mut mesh = MediaMesh::default();
        assert!(!mesh.offer(0, &raw));
        for _ in 0..64 {
            assert!(mesh.offer(1, &raw));
        }
        assert!(!mesh.offer(1, &raw));
        assert_eq!(mesh.drain().len(), 64);
        assert!(mesh.offer(1, &raw));
    }
}
