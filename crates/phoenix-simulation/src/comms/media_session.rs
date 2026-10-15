//! Ephemeral local Session media leases. No serialization, game clocks or power mutation.
//! The adapter supplies the actual current operator and current paid call on
//! EVERY operation. Neither a packet nor a claimed remote token supplies them.
use base64::Engine;
use bevy::prelude::Resource;
use phoenix_model::comms_media::{MediaMode, MediaPacket, MediaPacketKind};
use std::collections::{BTreeMap, VecDeque};

const MAX_ACTIVE_LEASES: usize = 64;
const MAX_FRAMES_PER_SECOND: usize = 16;
const MAX_ENCODED_BYTES_PER_SECOND: usize = 80 * 1024;

/// Private local authority fingerprint, derived by the existing Session owner.
/// `connection` distinguishes authenticated incarnations of the same token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaOperator {
    pub token: String,
    pub connection: u64,
    pub ship: String,
    pub system: phoenix_model::identity::SystemId,
    pub console: phoenix_model::identity::ConsoleId,
    pub station: phoenix_model::identity::StationId,
}

/// Current accepted lineage/actual carrier, supplied by the fixed Comms owner.
/// This contains no peer token and must never be constructed from a wire claim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EligibleMediaCall {
    pub origin: String,
    pub local_ship: String,
    pub peer_ship: String,
    pub mode: MediaMode,
}

#[derive(Debug)]
struct Lease {
    operator: MediaOperator,
    call: EligibleMediaCall,
    generation: u32,
    sequence: u32,
    capture_ms: BTreeMap<u8, u32>,
    arrivals: VecDeque<(u64, usize)>,
    last_video: Option<u64>,
    last_clock: u64,
}

/// High-water survives lease invalidation and restore on a live process.
/// Never deserialize, replace with Default, or reset this Resource on restore.
#[derive(Resource, Default, Debug)]
pub struct MediaLeases {
    high_water: u32,
    leases: BTreeMap<String, Lease>,
}
impl MediaLeases {
    pub fn start(
        &mut self,
        operator: MediaOperator,
        call: EligibleMediaCall,
        now_ms: u64,
    ) -> Result<u32, &'static str> {
        if operator.ship != call.local_ship
            || call.local_ship == call.peer_ship
            || !phoenix_model::comms_channel::identity(&call.origin)
            || (!self.leases.contains_key(&operator.token)
                && self.leases.len() >= MAX_ACTIVE_LEASES)
        {
            return Err("media start lacks current local call/operator ownership");
        }
        let generation = self
            .high_water
            .checked_add(1)
            .ok_or("media lease generations exhausted")?;
        self.high_water = generation;
        self.leases.insert(
            operator.token.clone(),
            Lease {
                operator,
                call,
                generation,
                sequence: 0,
                capture_ms: BTreeMap::new(),
                arrivals: VecDeque::new(),
                last_video: None,
                last_clock: now_ms,
            },
        );
        Ok(generation)
    }
    pub fn stop(&mut self, operator: &MediaOperator, generation: u32) -> bool {
        if !self
            .leases
            .get(&operator.token)
            .is_some_and(|lease| lease.operator == *operator && lease.generation == generation)
        {
            return false;
        }
        self.leases.remove(&operator.token);
        true
    }
    /// Identify/reconnect, departure or authority change invalidates that start.
    pub fn forget_operator(&mut self, token: &str) {
        self.leases.remove(token);
    }
    /// Restore/continuation clears starts, not their generation namespace.
    pub fn invalidate_all(&mut self) {
        self.leases.clear();
    }
    /// Call after current paid carrier/authority derivation even when nobody
    /// sends a frame. Otherwise a silent loss/recovery could revive an old start.
    pub fn reconcile(
        &mut self,
        mut current: impl FnMut(&MediaOperator, &EligibleMediaCall) -> bool,
    ) {
        self.leases
            .retain(|_, lease| current(&lease.operator, &lease.call));
    }
    pub fn generation(&self, operator: &MediaOperator, call: &EligibleMediaCall) -> Option<u32> {
        let lease = self.leases.get(&operator.token)?;
        (lease.operator == *operator && lease.call == *call).then_some(lease.generation)
    }
    /// Supplied only by the actual authenticated current paid-call heartbeat.
    /// This cannot start capture; it can only retire stale consent and reserve
    /// a generation beyond a surviving receiver's retained stale-frame floor.
    pub(super) fn raise_floor(
        &mut self,
        operator: &MediaOperator,
        call: &EligibleMediaCall,
        floor: u32,
    ) {
        if floor == 0 || operator.ship != call.local_ship {
            return;
        }
        if self
            .leases
            .get(&operator.token)
            .is_some_and(|lease| lease.operator != *operator || lease.call != *call)
        {
            return;
        }
        self.high_water = self.high_water.max(floor);
        if self.leases.get(&operator.token).is_some_and(|lease| {
            lease.operator == *operator && lease.call == *call && lease.generation <= floor
        }) {
            self.leases.remove(&operator.token);
        }
    }
    /// `call` is Some ONLY for a genuinely CURRENT paid physical route and
    /// accepted selected lineage. No route/receipt is inferred from this cache.
    /// `now_ms` is a trusted local monotonic adapter clock, never packet input.
    pub fn admit(
        &mut self,
        operator: &MediaOperator,
        call: Option<&EligibleMediaCall>,
        packet: &MediaPacket,
        now_ms: u64,
    ) -> Result<(), &'static str> {
        packet.validate()?;
        let Some(call) = call else {
            self.forget_operator(&operator.token);
            return Err("media carrier is not currently paid/available");
        };
        let lease = self
            .leases
            .get_mut(&operator.token)
            .ok_or("media requires a fresh explicit start")?;
        if lease.operator != *operator || lease.call != *call {
            self.forget_operator(&operator.token);
            return Err("media operator/call changed; fresh local start required");
        }
        if packet.call_origin_id != call.origin
            || packet.lease_generation != lease.generation
            || !call.mode.permits(packet.kind)
            || packet.sequence <= lease.sequence
            || now_ms < lease.last_clock
        {
            return Err("stale or mismatched media call/lease/sequence");
        }
        let kind = match packet.kind {
            MediaPacketKind::Pcm16Mono16k => 0,
            MediaPacketKind::Jpeg => 1,
        };
        if lease
            .capture_ms
            .get(&kind)
            .is_some_and(|prior| packet.capture_ms < *prior)
        {
            return Err("media capture offsets rewound within one supported stream");
        }
        // Expire by an actual local receipt clock; a forged capture offset
        // cannot buy rate or memory. Invalid packets don't advance the lease.
        while lease
            .arrivals
            .front()
            .is_some_and(|(tick, _)| now_ms.saturating_sub(*tick) >= 1000)
        {
            lease.arrivals.pop_front();
        }
        if lease.arrivals.len() >= MAX_FRAMES_PER_SECOND
            || lease.arrivals.iter().map(|(_, bytes)| bytes).sum::<usize>() + packet.data.len()
                > MAX_ENCODED_BYTES_PER_SECOND
            || (kind == 1
                && lease
                    .last_video
                    .is_some_and(|tick| now_ms.saturating_sub(tick) < 500))
        {
            return Err("media exceeds bounded real-time packet admission");
        }
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&packet.data)
            .map_err(|_| "invalid bounded media encoding")?;
        packet.validate_decoded(&decoded)?;
        lease.sequence = packet.sequence;
        lease.capture_ms.insert(kind, packet.capture_ms);
        lease.last_clock = now_ms;
        lease.arrivals.push_back((now_ms, packet.data.len()));
        if kind == 1 {
            lease.last_video = Some(now_ms);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn authority() -> (MediaOperator, EligibleMediaCall) {
        (
            MediaOperator {
                token: "actual-local-token".into(),
                connection: 1,
                ship: "own".into(),
                system: phoenix_model::identity::SystemId("comms".into()),
                console: phoenix_model::identity::ConsoleId("comms".into()),
                station: phoenix_model::identity::StationId("comms".into()),
            },
            EligibleMediaCall {
                origin: "actual-call".into(),
                local_ship: "own".into(),
                peer_ship: "peer".into(),
                mode: MediaMode::Audio,
            },
        )
    }
    fn packet(generation: u32, sequence: u32) -> MediaPacket {
        MediaPacket {
            call_origin_id: "actual-call".into(),
            lease_generation: generation,
            sequence,
            capture_ms: sequence * 80,
            kind: MediaPacketKind::Pcm16Mono16k,
            data: format!("{}AA==", "A".repeat(3412)),
        }
    }
    #[test]
    fn restore_reconnect_and_operator_reassignment_never_reuse_a_live_generation() {
        let (mut operator, call) = authority();
        let mut leases = MediaLeases::default();
        let first = leases.start(operator.clone(), call.clone(), 0).unwrap();
        let queued = packet(first, 100);
        leases.invalidate_all();
        assert!(leases.admit(&operator, Some(&call), &queued, 1).is_err());
        let second = leases.start(operator.clone(), call.clone(), 2).unwrap();
        assert!(second > first);
        assert!(leases.admit(&operator, Some(&call), &queued, 3).is_err());
        assert!(leases
            .admit(&operator, Some(&call), &packet(second, 1), 3)
            .is_ok());
        operator.connection += 1;
        assert!(leases
            .admit(&operator, Some(&call), &packet(second, 2), 4)
            .is_err());
        let third = leases.start(operator.clone(), call.clone(), 5).unwrap();
        assert!(third > second);
        operator.station.0 = "former-host".into();
        assert!(leases
            .admit(&operator, Some(&call), &packet(third, 1), 6)
            .is_err());
        leases.high_water = u32::MAX;
        assert!(leases.start(operator, call, 7).is_err());
    }
    #[test]
    fn loss_requires_new_start_and_packet_offsets_cannot_bypass_actual_rate() {
        let (operator, call) = authority();
        let mut leases = MediaLeases::default();
        let generation = leases.start(operator.clone(), call.clone(), 0).unwrap();
        for sequence in 1..=16 {
            assert!(leases
                .admit(&operator, Some(&call), &packet(generation, sequence), 0)
                .is_ok());
        }
        assert!(leases
            .admit(&operator, Some(&call), &packet(generation, 17), 0)
            .is_err());
        assert!(leases
            .admit(&operator, Some(&call), &packet(generation, 17), 1000)
            .is_ok());
        assert!(leases
            .admit(&operator, None, &packet(generation, 18), 1001)
            .is_err());
        assert!(leases
            .admit(&operator, Some(&call), &packet(generation, 18), 1002)
            .is_err());
    }
    #[test]
    fn surviving_receiver_floor_requires_new_consent_without_reusing_or_wrapping_generations() {
        let (operator, call) = authority();
        let mut replacement = MediaLeases::default();
        let first = replacement
            .start(operator.clone(), call.clone(), 0)
            .unwrap();
        replacement.raise_floor(&operator, &call, first);
        assert_eq!(replacement.generation(&operator, &call), None);
        assert!(replacement
            .admit(&operator, Some(&call), &packet(first, 1), 1)
            .is_err());
        let fresh = replacement
            .start(operator.clone(), call.clone(), 2)
            .unwrap();
        assert!(fresh > first);
        // An older queued floor cannot retire the newer explicit start.
        replacement.raise_floor(&operator, &call, first);
        assert_eq!(replacement.generation(&operator, &call), Some(fresh));
        // Only the exact current operator fingerprint and selected lineage can
        // invalidate its consent; process supplies these after actual routing.
        let mut stale = operator.clone();
        stale.connection += 1;
        replacement.raise_floor(&stale, &call, fresh);
        assert_eq!(replacement.generation(&operator, &call), Some(fresh));
        let mut other = call.clone();
        other.origin = "another-call".into();
        replacement.raise_floor(&operator, &other, fresh);
        assert_eq!(replacement.generation(&operator, &call), Some(fresh));
        replacement.raise_floor(&operator, &call, u32::MAX);
        assert_eq!(replacement.generation(&operator, &call), None);
        assert!(replacement.start(operator, call, 3).is_err());
    }
}
