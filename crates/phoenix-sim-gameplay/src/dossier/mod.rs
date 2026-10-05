/// The blackboard channel key dossiers are published under.
///
/// **Not a system id.** No `[[system]]` block declares it, no station owns it,
/// it registers no `ControlSource` and no `ControlSystem` message may target it
/// — a dossier is something the crew *knows*, not a thing aboard the ship. It is
/// carried inside a [`SystemId`] value for `operations`' reason: the blackboard
/// map and the `BlackboardUpdate` wire message are typed that way.
pub const DOSSIER_BLACKBOARD_KEY: &str = "dossiers";

use crate::core::messages::InfrastructureSnapshot;
/// A subject's published condition track plus the crew-facing labels for it
/// (issue #1030).
///
/// Built by the adapter from an [`InfrastructureSnapshot`] and the authored
/// labels beside it; see [`SubjectCondition::from_published`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SubjectCondition {
    /// Structural condition as a fraction of the authored ceiling.
    pub condition_fraction: f32,
    /// `(label id, held)` for each operational flag that authored a label, in
    /// authored order.
    pub flags: Vec<(String, bool)>,
    /// `(label id, amount)` for each capacity that authored a label, in authored
    /// order.
    pub capacities: Vec<(String, i64)>,
}

impl SubjectCondition {
    /// Pair a published snapshot with the authored labels for its flags and
    /// capacities, dropping every entry that has none.
    ///
    /// `flag_labels` / `capacity_labels` resolve a machine id to its authored
    /// `strings.csv` label. Taking them as lookups rather than reading the live
    /// config keeps this side of the boundary honest: the *values* can only ever
    /// be ones `crate::core::messages::infrastructure_snapshot_from_state` already published, and the
    /// labels can only ever be ones an author wrote for the crew.
    pub fn from_published(
        published: &InfrastructureSnapshot,
        flag_labels: impl Fn(&str) -> Option<String>,
        capacity_labels: impl Fn(&str) -> Option<String>,
    ) -> Self {
        Self {
            condition_fraction: published.condition_fraction,
            flags: published
                .flags
                .iter()
                .filter_map(|(id, held)| flag_labels(id).map(|label| (label, *held)))
                .collect(),
            capacities: published
                .capacities
                .iter()
                .filter_map(|(id, amount)| capacity_labels(id).map(|label| (label, *amount)))
                .collect(),
        }
    }
}
