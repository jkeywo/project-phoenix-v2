//! Own bay records plus exact permitted dated contacts; no remote machinery view.
use super::*;
use bevy::ecs::entity_disabling::Disabled;
use phoenix_model::craft::*;
use phoenix_sim_gameplay::ship::craft::BayOperation;

pub(crate) fn publish(world: &mut World) {
    let mut query = world.query::<(Entity, &EntityUuid, &ShipBayJobs)>();
    let mut owners: Vec<_> = query
        .iter(world)
        .flat_map(|(entity, uuid, jobs)| {
            jobs.0
                .keys()
                .map(move |owner| (uuid.0.clone(), entity, owner.clone()))
        })
        .collect();
    owners.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.2.cmp(&b.2)));
    for (uuid, parent, owner) in owners {
        let Some(context) = runtime::context(world, parent, &owner) else {
            continue;
        };
        let mut view = BayView {
            capacity_space: context.config.capacity_space,
            ..Default::default()
        };
        if let Some(mut bodies) = world
            .try_query_filtered::<(&EntityUuid, &PhysicalProbe, &ProbeCustody), Allow<Disabled>>()
        {
            for (id, probe, custody) in bodies.iter(world) {
                if matches!(&custody.0,CraftLocation::Carried{parent_uuid,bay} if parent_uuid==&uuid && bay==&owner)
                {
                    view.stored.push(StoredCraftView {
                        id: id.0.clone(),
                        space: probe.0.space,
                    });
                }
            }
        }
        view.stored.sort_by(|a, b| a.id.cmp(&b.id));
        view.occupied_space = view
            .stored
            .iter()
            .fold(0_u64, |sum, row| sum.saturating_add(row.space));
        if let (Some(config), Some(boards), Some(_)) = (
            world.get::<crate::ship_plugin::ShipConfigComponent>(parent),
            world.get::<crate::server_app::ShipSystemBlackboards>(parent),
            world.get::<crate::science::broad::BroadTracks>(parent),
        ) {
            let observations = config.0.observations_for_system(&owner, &boards.0);
            let mut contacts = BTreeMap::new();
            for source in boards.0.keys() {
                let Some(board) = observations.get(source) else {
                    continue;
                };
                for record in
                    crate::science::contact_routing::observations(board).unwrap_or_default()
                {
                    if !record.predicted
                        && record.selectable
                        && runtime::recovery_route(world, parent, &owner, &record.contact_id)
                            .is_some()
                    {
                        contacts
                            .entry(record.contact_id.clone())
                            .and_modify(|tick: &mut u64| *tick = (*tick).max(record.observed_tick))
                            .or_insert(record.observed_tick);
                    }
                }
            }
            view.contacts = contacts
                .into_iter()
                .map(|(contact, observed_tick)| RecoveryContactView {
                    contact,
                    observed_tick,
                })
                .collect();
        }
        let state = &world.get::<ShipBayJobs>(parent).unwrap().0[&owner];
        let active = state.job.as_ref().is_some_and(|job| job.outcome.is_none());
        view.can_launch = context.available && !active && !view.stored.is_empty();
        view.can_recover = context.available
            && !active
            && view.occupied_space < view.capacity_space
            && !view.contacts.is_empty();
        view.job = state.job.as_ref().map(|job| BayJobView {
            id: job.id.0,
            operation: job.activity.operation,
            craft: match job.activity.operation {
                BayOperation::Launch => Some(job.activity.craft_uuid.clone()),
                BayOperation::Recover => {
                    runtime::alias(world, parent, &owner, &job.activity.craft_uuid)
                }
            },
            fraction: (job.activity.worked_secs / job.activity.required_secs()) as f32,
            paused: job.outcome.is_none() && (job.paused || !context.available),
            ready: job.activity.ready(),
            outcome: job.outcome,
        });
        if let Some(mut boards) = world.get_mut::<crate::server_app::ShipSystemBlackboards>(parent)
        {
            boards
                .0
                .insert(owner, crate::core::messages::SystemBlackboard::Bay(view));
        }
    }
}
