//! The probes observe real append-only emissions; they never issue commands.
use super::graph;
use bevy::prelude::*;
use project_phoenix::{
    command_admission::AdmissionSet,
    core::messages::{AdmittedCommand, AdmittedCommands, SystemControlPayload as P},
    entities::spawner::EntityUuid,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
pub fn owns(i: usize, c: &AdmittedCommand) -> bool {
    match i {
        0 => c.target.0 == "red-alert" && matches!(c.payload, P::SetRedAlert { .. }),
        1 => {
            c.target.0 == "sensors"
                && matches!(
                    c.payload,
                    P::SetScienceTarget { .. } | P::ClearScienceTarget | P::ScanTarget { .. }
                )
        }
        _ => unreachable!(),
    }
}
pub fn projected(i: usize, cs: &[AdmittedCommand]) -> Vec<AdmittedCommand> {
    cs.iter()
        .filter(|c| match i {
            0 => owns(0, c),
            1 => owns(1, c) && !matches!(c.payload, P::ScanTarget { .. }),
            2 => owns(1, c) && matches!(c.payload, P::ScanTarget { .. }),
            _ => unreachable!(),
        })
        .cloned()
        .collect()
}
pub fn evidence(cs: &[AdmittedCommand]) -> Value {
    json!(cs.iter().map(|c|json!({"target":c.target,"payload":c.payload,"response_token":c.response_token,"feedback_correlation":c.feedback_correlation})).collect::<Vec<_>>())
}
#[derive(Resource, Default)]
pub struct Probes {
    pub active: bool,
    pub prefix: BTreeMap<String, Vec<AdmittedCommand>>,
    pub before: BTreeMap<(usize, String), Vec<AdmittedCommand>>,
    pub after: BTreeMap<(usize, String), Vec<AdmittedCommand>>,
}
impl Probes {
    pub fn begin(&mut self, prefix: BTreeMap<String, Vec<AdmittedCommand>>) {
        self.active = true;
        self.prefix = prefix;
        self.before.clear();
        self.after.clear();
    }
    pub fn chunks(&self, uuid: &str) -> [Vec<AdmittedCommand>; 2] {
        std::array::from_fn(|i| {
            self.after[&(i, uuid.into())]
                .iter()
                .filter(|c| owns(i, c))
                .cloned()
                .collect()
        })
    }
    pub fn report(&self) -> Value {
        assert_eq!(self.before.len(), 4);
        assert_eq!(self.after.len(), 4);
        json!(self.before.iter().map(|((i,u),v)|json!({"producer":i,"ship":u,"before":evidence(v),"after":evidence(&self.after[&(*i,u.clone())])})).collect::<Vec<_>>())
    }
}
fn before<const I: usize>(q: Query<(&EntityUuid, &AdmittedCommands)>, mut p: ResMut<Probes>) {
    if !p.active {
        return;
    }
    for (u, c) in &q {
        let Some(prefix) = p.prefix.get(&u.0) else {
            continue;
        };
        assert!(c.0.starts_with(prefix));
        assert!(
            !c.0.iter().any(|c| owns(I, c)),
            "only selected producer owns its emissions"
        );
        assert!(p.before.insert((I, u.0.clone()), c.0.clone()).is_none());
    }
}
fn after<const I: usize>(q: Query<(&EntityUuid, &AdmittedCommands)>, mut p: ResMut<Probes>) {
    if !p.active {
        return;
    }
    for (u, c) in &q {
        if !p.prefix.contains_key(&u.0) {
            continue;
        }
        assert!(
            c.0.starts_with(&p.before[&(I, u.0.clone())]),
            "full raw prefix preserved"
        );
        for c in c.0.iter().filter(|c| owns(I, c)) {
            assert_eq!(c.response_token, Some(format!("ai:{}", u.0)));
        }
        assert!(p.after.insert((I, u.0.clone()), c.0.clone()).is_none());
    }
}
pub fn names() -> [(&'static str, &'static str); 2] {
    [
        (
            std::any::type_name_of_val(&before::<0>),
            std::any::type_name_of_val(&after::<0>),
        ),
        (
            std::any::type_name_of_val(&before::<1>),
            std::any::type_name_of_val(&after::<1>),
        ),
    ]
}
pub fn install(app: &mut App) {
    app.init_resource::<Probes>();
    app.world_mut().schedule_scope(FixedUpdate, |_, s| {
        let a = graph::type_set(s.graph(), graph::key(s.graph(), graph::PRODUCERS[0]));
        let b = graph::type_set(s.graph(), graph::key(s.graph(), graph::PRODUCERS[1]));
        s.add_systems(
            (
                before::<0>.before(a),
                after::<0>.after(a),
                before::<1>.before(b),
                after::<1>.after(b),
            )
                .after(AdmissionSet),
        );
    });
}
/// Enumerate every stable interleaving of actual chunks, not synthetic domain samples.
pub fn interleavings(prefix: &[AdmittedCommand], chunks: &[Vec<AdmittedCommand>; 2]) -> usize {
    assert!(!prefix.is_empty() && chunks.iter().all(|c| !c.is_empty()));
    fn walk(
        a: &[AdmittedCommand],
        b: &[AdmittedCommand],
        out: Vec<AdmittedCommand>,
        rows: &mut Vec<Vec<AdmittedCommand>>,
    ) {
        if a.is_empty() && b.is_empty() {
            rows.push(out);
            return;
        }
        if let Some((head, tail)) = a.split_first() {
            let mut next = out.clone();
            next.push(head.clone());
            walk(tail, b, next, rows);
        }
        if let Some((head, tail)) = b.split_first() {
            let mut next = out;
            next.push(head.clone());
            walk(a, tail, next, rows);
        }
    }
    let baseline = [prefix, chunks[0].as_slice(), chunks[1].as_slice()].concat();
    let mut rows = Vec::new();
    walk(&chunks[0], &chunks[1], prefix.to_vec(), &mut rows);
    assert!(rows.len() >= 2);
    for row in &rows {
        assert!(row.starts_with(prefix));
        for i in 0..3 {
            assert_eq!(
                projected(i, row),
                projected(i, &baseline),
                "actual typed consumer subsequence {i}"
            );
        }
    }
    rows.len()
}
