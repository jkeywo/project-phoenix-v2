//! Retained planned Contact delivery on the SAME canonical source inbox record.
//! No retry queue, extra work charge, acknowledgement oracle or decision loop.
use super::{copy_message, Endpoint};
use crate::comms::server::{CommsInboxRes, CommsRuntime};
use phoenix_model::{comms_channel::CommsAudience, comms_request::CommsRadioContent};

pub(super) fn settle(
    endpoints: &[Endpoint],
    inbox: &mut CommsInboxRes,
    runtime: &mut CommsRuntime,
    paths: &crate::regions::signal_path::RegionSignalQuery<'_, '_>,
    now: u64,
) {
    let retained: Vec<_> = inbox
        .0
        .iter()
        .filter(|message| {
            message.transmission.as_ref().is_some_and(|wire| {
                wire.sustained
                    && wire.received_tick.is_none()
                    && matches!(wire.content, CommsRadioContent::Request { .. })
            })
        })
        .cloned()
        .collect();
    for sent in retained {
        let wire = sent.transmission.as_ref().expect("selected transmission");
        let CommsAudience::Contact { target_uuid } = &wire.audience else {
            continue;
        };
        let copy_id = format!("radio-copy/{}/{}", wire.origin_id, target_uuid);
        if inbox.0.iter().any(|message| message.id == copy_id) {
            continue;
        }
        let Some(source) = endpoints
            .iter()
            .find(|endpoint| endpoint.uuid == sent.sender_uuid)
        else {
            continue;
        };
        let Some(receiver) = endpoints
            .iter()
            .find(|endpoint| &endpoint.uuid == target_uuid)
        else {
            continue;
        };
        if !super::route_available(endpoints, source, receiver, &wire.channel, paths) {
            continue;
        }
        let mut heard = wire.clone();
        heard.received_tick = Some(now);
        // Original attribution is immutable even if the live hull is renamed.
        let original_source = Endpoint {
            name: sent.sender_name.clone(),
            ..source.clone()
        };
        inbox.0.inject(copy_message(
            copy_id,
            &original_source,
            &receiver.uuid,
            &sent.body,
            heard,
        ));
        runtime.needs_broadcast = true;
    }
}
