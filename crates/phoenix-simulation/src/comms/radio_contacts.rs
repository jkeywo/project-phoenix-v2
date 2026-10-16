//! The existing own Comms publication learns only actual paid public endpoints.
use super::{Endpoint, Sources};
use crate::core::messages::{CommsBlackboard, CommsContact, SystemBlackboard};
use bevy::prelude::*;

pub(super) fn publish(
    endpoints: &[Endpoint],
    sources: &mut Sources,
    hailable: &Query<&crate::comms::CommsHailable>,
    paths: &crate::regions::signal_path::RegionSignalQuery,
) {
    // Crew publications retain their established legacy roster/plot boundary.
    // This pass never reads a LocalShip-derived distance or contact projection.
    for receiver in endpoints.iter().filter(|endpoint| !endpoint.fleet) {
        let mut contacts: Vec<_> = endpoints
            .iter()
            .filter_map(|source| {
                let advertised = hailable.get(source.entity).ok()?;
                if source.uuid == receiver.uuid
                    || !source.radio.channels.iter().any(|channel| {
                        channel.key.is_none()
                            && super::route_available(
                                endpoints,
                                source,
                                receiver,
                                &channel.id,
                                paths,
                            )
                    })
                {
                    return None;
                }
                Some(CommsContact {
                    uuid: source.uuid.clone(),
                    name: crate::comms::roster::entity_contact_label(
                        advertised.display_name.as_deref(),
                        Some(&source.name),
                        &source.uuid,
                    ),
                    in_range: true,
                    is_urgent: false,
                })
            })
            .collect();
        contacts.sort_by(|a, b| a.name.cmp(&b.name).then(a.uuid.cmp(&b.uuid)));
        let Ok((_, _, _, _, _, _, _, _, _, mut boards, _, _)) = sources.get_mut(receiver.entity)
        else {
            continue;
        };
        match boards
            .0
            .entry(receiver.radio.system.clone())
            .or_insert_with(|| SystemBlackboard::Comms(CommsBlackboard::default()))
        {
            SystemBlackboard::Comms(board) => board.contacts = contacts,
            // Invalid/mismatched publications confer no alternate write scope.
            _ => {}
        }
    }
}
