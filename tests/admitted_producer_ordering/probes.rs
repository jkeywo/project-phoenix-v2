//! Read-only ADC probes around unique existing production instances.
use super::{domains, graph_proof};
use bevy::prelude::*;
use project_phoenix::{
    command_admission::AdmissionSet,
    core::messages::{AdmittedCommand, AdmittedCommands, SystemId},
    entities::spawner::EntityUuid,
};
use std::collections::BTreeMap;

pub fn names() -> [(&'static str, &'static str); 4] {
    [
        (
            std::any::type_name_of_val(&before::<0>),
            std::any::type_name_of_val(&after::<0>),
        ),
        (
            std::any::type_name_of_val(&before::<1>),
            std::any::type_name_of_val(&after::<1>),
        ),
        (
            std::any::type_name_of_val(&before::<2>),
            std::any::type_name_of_val(&after::<2>),
        ),
        (
            std::any::type_name_of_val(&before::<3>),
            std::any::type_name_of_val(&after::<3>),
        ),
    ]
}

#[derive(Resource, Default)]
pub struct Probes {
    pub active: bool,
    pub ships: Vec<Entity>,
    pub arcs: BTreeMap<String, Vec<SystemId>>,
    pub expected_prefix: BTreeMap<String, Vec<AdmittedCommand>>,
    pub before: BTreeMap<(usize, String), Vec<AdmittedCommand>>,
    pub after: BTreeMap<(usize, String), Vec<AdmittedCommand>>,
}
impl Probes {
    pub fn begin(&mut self, prefix: BTreeMap<String, Vec<AdmittedCommand>>) {
        self.active = true;
        self.expected_prefix = prefix;
        self.before.clear();
        self.after.clear();
    }
    pub fn chunks(&self, uuid: &str) -> [Vec<AdmittedCommand>; 4] {
        std::array::from_fn(|i| {
            domains::projected(i, &self.after[&(i, uuid.to_owned())], &self.arcs[uuid])
        })
    }
}
fn before<const I: usize>(
    q: Query<(Entity, &EntityUuid, &AdmittedCommands)>,
    mut probes: ResMut<Probes>,
) {
    if !probes.active {
        return;
    }
    for (ship, uuid, admitted) in &q {
        if !probes.ships.contains(&ship) {
            continue;
        }
        assert!(
            admitted.0.starts_with(&probes.expected_prefix[&uuid.0]),
            "ordinary Admission prefix precedes producer"
        );
        assert!(
            domains::projected(I, &admitted.0, &probes.arcs[&uuid.0]).is_empty(),
            "no other same-domain emitter may counterfeit coverage"
        );
        assert!(probes
            .before
            .insert((I, uuid.0.clone()), admitted.0.clone())
            .is_none());
    }
}
fn after<const I: usize>(
    q: Query<(Entity, &EntityUuid, &AdmittedCommands)>,
    mut probes: ResMut<Probes>,
) {
    if !probes.active {
        return;
    }
    for (ship, uuid, admitted) in &q {
        if !probes.ships.contains(&ship) {
            continue;
        }
        assert!(
            admitted.0.starts_with(&probes.before[&(I, uuid.0.clone())]),
            "producer preserves complete raw prefix"
        );
        for cmd in domains::projected(I, &admitted.0, &probes.arcs[&uuid.0]) {
            assert_eq!(
                cmd.response_token.as_deref(),
                Some(format!("ai:{}", uuid.0).as_str()),
                "actual ship-owned AI admission"
            );
        }
        assert!(probes
            .after
            .insert((I, uuid.0.clone()), admitted.0.clone())
            .is_none());
    }
}
pub fn install(app: &mut App) {
    app.init_resource::<Probes>();
    app.world_mut().schedule_scope(FixedUpdate, |_, schedule| {
        let sets: Vec<_> = graph_proof::PRODUCERS
            .iter()
            .map(|n| graph_proof::type_set(schedule.graph(), graph_proof::key(schedule.graph(), n)))
            .collect();
        macro_rules! pair {
            ($i:literal) => {
                // A relative producer edge alone lets the before-probe run
                // ahead of AdmissionSet. Both probes must follow admission;
                // the actual producers retain their own production phases.
                schedule.add_systems(
                    (before::<$i>.before(sets[$i]), after::<$i>.after(sets[$i]))
                        .after(AdmissionSet),
                );
            };
        }
        pair!(0);
        pair!(1);
        pair!(2);
        pair!(3);
    });
}
