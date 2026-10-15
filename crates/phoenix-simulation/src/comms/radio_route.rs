//! Current physical radio routes. Derived within the existing paid radio tick;
//! no forwarding inbox, route cache, resource claim or private route publication.
use super::Endpoint;
use phoenix_model::comms_channel::CommsChannelConfig;
use std::collections::{BTreeSet, VecDeque};

fn channel<'a>(endpoint: &'a Endpoint, id: &str) -> Option<&'a CommsChannelConfig> {
    let selected = endpoint.radio.channels.iter().find(|row| row.id == id)?;
    (endpoint.listening.is_finite()
        && endpoint.listening > 0.0
        && selected
            .key
            .as_ref()
            .is_none_or(|key| endpoint.radio.keys.contains(key)))
    .then_some(selected)
}

/// The shortest viable path, direct first, then lexicographic endpoint identity.
/// Kept private: neither the source nor recipient learns relay membership.
fn select<'a>(
    endpoints: &'a [Endpoint],
    source: &'a Endpoint,
    receiver: &'a Endpoint,
    id: &str,
    mut reachable: impl FnMut(&Endpoint, &CommsChannelConfig, &Endpoint, &CommsChannelConfig) -> bool,
) -> Option<Vec<&'a Endpoint>> {
    let transmitting = channel(source, id)?;
    let receiving = channel(receiver, id)?;
    if source.uuid == receiver.uuid || transmitting.key != receiving.key {
        return None;
    }
    let mut candidates: Vec<_> = endpoints
        .iter()
        .filter(|endpoint| endpoint.uuid != source.uuid && endpoint.uuid != receiver.uuid)
        .filter_map(|endpoint| {
            channel(endpoint, id)
                .filter(|profile| profile.relay && profile.key == transmitting.key)
                .map(|profile| (endpoint, profile))
        })
        .collect();
    candidates.sort_by(|a, b| a.0.uuid.cmp(&b.0.uuid));
    let mut visited = BTreeSet::from([source.uuid.as_str()]);
    let mut pending = VecDeque::from([vec![source]]);
    while let Some(route) = pending.pop_front() {
        let current = *route.last().expect("nonempty route");
        let current_channel = channel(current, id).expect("queued compatible endpoint");
        if reachable(current, current_channel, receiver, receiving) {
            let mut completed = route;
            completed.push(receiver);
            return Some(completed);
        }
        for &(endpoint, profile) in &candidates {
            if !visited.contains(endpoint.uuid.as_str())
                && reachable(current, current_channel, endpoint, profile)
            {
                visited.insert(endpoint.uuid.as_str());
                let mut next = route.clone();
                next.push(endpoint);
                pending.push_back(next);
            }
        }
    }
    None
}

pub(crate) fn available(
    endpoints: &[Endpoint],
    source: &Endpoint,
    receiver: &Endpoint,
    id: &str,
    paths: &crate::regions::signal_path::RegionSignalQuery<'_, '_>,
) -> bool {
    select(endpoints, source, receiver, id, |a, ac, b, bc| {
        super::super::server::signal_comms_in_range(
            paths,
            super::super::server::SignalEndpoint {
                point: a.point,
                range: ac.range * a.listening,
                receiver: Some((&a.uuid, &a.radio.system)),
            },
            super::super::server::SignalEndpoint {
                point: b.point,
                range: bc.range * b.listening,
                receiver: Some((&b.uuid, &b.radio.system)),
            },
        )
    })
    .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::*;
    use phoenix_model::{comms_channel::CommsRadioConfig, identity::SystemId};
    fn endpoint(id: &str, relay: bool) -> Endpoint {
        Endpoint {
            entity: Entity::PLACEHOLDER,
            uuid: id.into(),
            name: id.into(),
            point: Vec3::ZERO,
            radio: CommsRadioConfig {
                system: SystemId("comms".into()),
                request_reading_ticks: 30,
                keys: vec!["fleet-key".into()],
                channels: vec![CommsChannelConfig {
                    id: "fleet".into(),
                    range: 100.0,
                    work_units: 1,
                    key: Some("fleet-key".into()),
                    relay,
                }],
            },
            listening: 1.0,
            fleet: false,
            faction: None,
        }
    }
    #[test]
    fn routes_are_direct_first_then_shortest_stable_identity_without_cycles() {
        let source = endpoint("source", false);
        let receiver = endpoint("receiver", false);
        // Deliberately reverse insertion order; a/b can reach each other.
        let endpoints = vec![endpoint("b", true), endpoint("a", true)];
        let route = select(&endpoints, &source, &receiver, "fleet", |a, _, b, _| {
            !(a.uuid == "source" && b.uuid == "receiver")
        })
        .unwrap();
        assert_eq!(
            route
                .iter()
                .map(|row| row.uuid.as_str())
                .collect::<Vec<_>>(),
            vec!["source", "a", "receiver"]
        );
        let direct = select(&endpoints, &source, &receiver, "fleet", |_, _, _, _| true).unwrap();
        assert_eq!(direct.len(), 2);
        assert!(
            select(&endpoints, &source, &receiver, "fleet", |_, _, b, _| b.uuid
                != "receiver")
            .is_none()
        );
    }
    #[test]
    fn actual_channel_capability_paid_operation_and_held_keys_gate_forwarding() {
        let source = endpoint("source", false);
        let receiver = endpoint("receiver", false);
        let hop = |a: &Endpoint, _: &CommsChannelConfig, b: &Endpoint, _: &CommsChannelConfig| {
            a.uuid != "source" || b.uuid != "receiver"
        };
        for fault in 0..4 {
            let mut relay = endpoint("relay", true);
            match fault {
                0 => relay.listening = 0.0,
                1 => relay.radio.channels[0].relay = false,
                2 => relay.radio.keys.clear(),
                _ => relay.radio.channels[0].key = Some("other".into()),
            }
            assert!(
                select(&[relay], &source, &receiver, "fleet", hop).is_none(),
                "fault{fault}"
            );
        }
    }
}
