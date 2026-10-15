//! Bounded ephemeral forwarding on the existing native fleet bridge.
use super::*;

/// Classify the actual typed host envelope before the generic/native queues.
/// Caller flags cannot promote media into the ordinary reliable path.
pub(super) fn classify(raw: &str) -> phoenix_transport::socket::IngressClass {
    use phoenix_transport::socket::IngressClass;
    let Ok(outer) = crate::core::codec::from_json::<serde_json::Value>(raw) else {
        return IngressClass::Ordinary;
    };
    let Some(payload) = outer.get("payload") else {
        return IngressClass::Ordinary;
    };
    let header = payload
        .as_str()
        .and_then(|text| crate::core::codec::from_json::<serde_json::Value>(text).ok());
    let media = header
        .as_ref()
        .or_else(|| payload.is_object().then_some(payload))
        .is_some_and(|header| header.get("t").and_then(|t| t.as_str()) == Some("media"));
    if !media {
        return if outer.get("drop_only").and_then(|flag| flag.as_bool()) == Some(true) {
            IngressClass::Refuse
        } else {
            IngressClass::Ordinary
        };
    }
    if raw.len() <= 32768
        && outer.get("type").and_then(|t| t.as_str()) == Some("relay")
        && outer.get("class").and_then(|t| t.as_str()) == Some("snapshot")
        && payload
            .as_str()
            .is_some_and(|text| crate::comms::media_mesh::MediaHostFrame::decode(text).is_some())
    {
        IngressClass::Scoped
    } else {
        IngressClass::Refuse
    }
}

pub(super) fn is_drop_only(raw: &str) -> bool {
    #[derive(serde::Deserialize)]
    struct Header {
        #[serde(rename = "type")]
        kind: String,
        class: String,
        #[serde(default)]
        drop_only: bool,
    }
    crate::core::codec::from_json::<Header>(raw).is_ok_and(|header| {
        header.kind == "relay" && header.class == "snapshot" && header.drop_only
    })
}

fn supported(world: &World, generation: u64) -> bool {
    let lanes = world.resource::<NativeFleetRoleWires>();
    if lanes.terminal
        || lanes
            .snapshot_sequence
            .get(&generation)
            .is_some_and(|sequence| *sequence >= 9_007_199_254_740_991)
    {
        return false;
    }
    if generation == 0 {
        lanes.adopted
            && !lanes.primary_closed
            && world
                .get_resource::<NativeFleetWire>()
                .is_some_and(|wire| wire.0.is_open() && wire.0.snapshot_only_supported())
    } else {
        lanes
            .sockets
            .get(&generation)
            .is_some_and(|wire| wire.is_open() && wire.snapshot_only_supported())
    }
}

fn receipt(world: &World, generation: u64, sequence: Option<u64>, accepted: bool) {
    if let Some(bridge) = world.get_resource::<HostLobbyBridgeResource>() {
        let raw = serde_json::json!({"native_wire":{
            "generation":generation,
            "event":if sequence.is_some() {"snapshot-receipt"} else {"snapshot-ready"},
            "sequence":sequence,"accepted":accepted,"supported":supported(world,generation)
            ,"next_sequence":world.resource::<NativeFleetRoleWires>().snapshot_sequence.get(&generation)
                .copied().unwrap_or(0).checked_add(1).filter(|next|*next<=9_007_199_254_740_991)
        }})
        .to_string();
        // At most one receipt per admitted generation is outstanding. Tiny
        // control evidence may use the ordered lane; no media bytes do.
        bridge.0.push_fleet_wire(raw);
    }
}

pub(super) fn ready(world: &World, generation: u64) {
    receipt(world, generation, None, false);
}

pub(super) fn send(
    world: &mut World,
    generation: u64,
    sequence: u64,
    frame: String,
    max_buffered_bytes: usize,
) {
    let fresh = sequence > 0
        && sequence <= 9_007_199_254_740_991
        && world
            .resource::<NativeFleetRoleWires>()
            .snapshot_sequence
            .get(&generation)
            .is_none_or(|previous| sequence > *previous);
    let mut accepted = false;
    if fresh
        && supported(world, generation)
        && frame.len() <= 32768
        && max_buffered_bytes > 0
        && max_buffered_bytes <= 65536
        && is_drop_only(&frame)
        && classify(&frame) == phoenix_transport::socket::IngressClass::Scoped
    {
        world
            .resource_mut::<NativeFleetRoleWires>()
            .snapshot_sequence
            .insert(generation, sequence);
        if generation == 0 {
            if let Some(mut wire) = world.get_resource_mut::<NativeFleetWire>() {
                accepted = wire.0.try_send_snapshot(frame, max_buffered_bytes);
            }
        } else if let Some(wire) = world
            .resource_mut::<NativeFleetRoleWires>()
            .sockets
            .get_mut(&generation)
        {
            accepted = wire.try_send_snapshot(frame, max_buffered_bytes);
        }
    }
    // One actual attempt; no implicit retry or promotion to the ordinary send.
    receipt(world, generation, Some(sequence), accepted);
}

pub(super) fn support(world: &mut World) {
    let available = world
        .get_resource::<HostLobbyBridgeResource>()
        .is_some_and(|bridge| !bridge.0.fleet_faulted())
        && (supported(world, 0)
            || world
                .resource::<NativeFleetRoleWires>()
                .sockets
                .keys()
                .any(|generation| supported(world, *generation)));
    if let Some(mut mesh) = world.get_resource_mut::<crate::comms::media_mesh::MediaMesh>() {
        mesh.set_supported(available);
    }
}

pub(super) fn publish(world: &mut World) {
    let frames = world
        .get_resource_mut::<crate::comms::media_mesh::MediaMesh>()
        .map(|mut mesh| mesh.take_outgoing())
        .unwrap_or_default();
    let Some(bridge) = world.get_resource::<HostLobbyBridgeResource>() else {
        return;
    };
    for frame in frames {
        if frame.len() <= 20480 {
            if let Ok(json) = serde_json::to_string(&frame) {
                bridge
                    .0
                    .push_fleet_ephemeral(format!("window.__phoenixHostFleetMedia?.({json});"));
            }
        }
    }
}
