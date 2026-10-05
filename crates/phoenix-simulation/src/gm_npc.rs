//! Scenario-authored NPC doctrine choices, applied through ordinary AI doctrine.
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::messages::AiDirective;
use crate::entities::config::DoctrineObjective;
use crate::entities::spawner::{BehaviourSection, EntityTagsSection, EntityUuid};
use crate::gm_action::{GmActionOutcome, GmActionRefusalReason};
use crate::ship::components::ShipConfigComponent;
use crate::world::server::WorldContentRuntime;

pub mod inspector;

/// Runtime state retains the chosen authored contents even if its layer later
/// withdraws the palette. The baseline lets a complete snapshot overwrite clear
/// a bootstrap choice without retaining its doctrine accidentally.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AppliedNpcDoctrine {
    pub id: String,
    pub doctrine: Vec<DoctrineObjective>,
    pub baseline: Vec<DoctrineObjective>,
}

#[derive(Component, Clone, Debug, Default)]
pub struct NpcDoctrineState(pub Option<AppliedNpcDoctrine>);

/// Postcard cannot encode the unknown-length map produced by the authoring
/// schema's serde(flatten). Pin every field explicitly for the authoritative
/// fold, while keeping the ordinary schema intact for TOML and RON saves.
/// Exhaustive destructuring makes a future doctrine field a compile-time change
/// to this representation rather than silently omitting it from the digest.
fn doctrine_digest_fields(objective: &DoctrineObjective) -> impl Serialize + '_ {
    let DoctrineObjective {
        id,
        text,
        mandatory,
        directive_kind,
        directive_anchors,
        directive_loop,
        directive_target,
        directive_anchor,
        directive_dock_target,
        directive_hail_target,
        directive_scan_target,
        directive_operate_target,
        directive_order_target,
        directive_order_route,
        unrecognised_fields,
        base_priority,
        zero_gates,
        modifiers,
        target_speed,
        maintain_range,
        use_impulse,
    } = objective;
    (
        (
            id,
            text,
            mandatory,
            directive_kind,
            directive_anchors,
            directive_loop,
            directive_target,
            directive_anchor,
        ),
        (
            directive_dock_target,
            directive_hail_target,
            directive_scan_target,
            directive_operate_target,
            directive_order_target,
            directive_order_route,
            unrecognised_fields,
        ),
        (
            base_priority,
            zero_gates,
            modifiers,
            target_speed,
            maintain_range,
            use_impulse,
        ),
    )
}

pub(crate) fn applied_digest_fields(state: &AppliedNpcDoctrine) -> impl Serialize + '_ {
    (
        &state.id,
        state
            .doctrine
            .iter()
            .map(doctrine_digest_fields)
            .collect::<Vec<_>>(),
        state
            .baseline
            .iter()
            .map(doctrine_digest_fields)
            .collect::<Vec<_>>(),
    )
}

/// Compatibility is a property of the entity's authored consumers, never this
/// peer's LocalShip or transient AI rating. Damage/puppeting may temporarily
/// prevent operation without making an otherwise compatible profile invalid.
pub fn compatible(
    profile: &NpcDoctrinePaletteEntry,
    target: &str,
    tags: Option<&EntityTagsSection>,
    fleet: bool,
    civilian: bool,
    config: &crate::ship::config::ShipConfig,
    runtime: &WorldContentRuntime,
    anchors: Option<&std::collections::HashMap<String, [f32; 3]>>,
) -> bool {
    if fleet
        || civilian
        || tags.is_some_and(|tags| tags.0.iter().any(|tag| tag == "player"))
        || !profile.targets.iter().any(|name| {
            name == target
                || runtime
                    .name_to_uuid
                    .get(name)
                    .is_some_and(|uuid| uuid == target)
        })
    {
        return false;
    }
    use crate::ship::system_registry::*;
    let has = |kind: &str| config.systems.iter().any(|system| system.kind == kind);
    let helm = || has(HELM_THRUST_KIND) && has(HELM_STEERING_KIND);
    profile.doctrine.iter().all(|objective| {
        let Ok(directive) = crate::ai::core::parse_doctrine_directive(objective) else {
            return false;
        };
        match directive {
            AiDirective::None => true,
            AiDirective::Patrol { anchors: route, .. } => {
                helm()
                    && anchors
                        .is_some_and(|anchors| route.iter().all(|id| anchors.contains_key(id)))
            }
            AiDirective::Reach { anchor } | AiDirective::Retreat { anchor } => {
                helm() && anchors.is_some_and(|anchors| anchors.contains_key(&anchor))
            }
            AiDirective::Destroy { .. } => {
                has(TACTICAL_RADAR_KIND)
                    && (has(PHASER_BANK_KIND) || has(TORPEDO_TUBE_KIND) || has(BLASTER_BANK_KIND))
            }
            AiDirective::Dock { .. } => helm() && has(DOCK_KIND),
            // These producers still consume LocalShip mission objectives only.
            // Configuring their Systems does not give an NPC a doctrine consumer.
            AiDirective::Hail { .. } | AiDirective::Order { .. } => false,
            AiDirective::Scan { .. } => has(SENSORS_KIND),
            AiDirective::Tow { .. }
            | AiDirective::Stabilise { .. }
            | AiDirective::Escort { .. } => has(TRACTOR_KIND),
            AiDirective::Transfer { .. } => helm() && has(DOCK_KIND) && has(UMBILICAL_KIND),
            AiDirective::FieldRepair { .. } => has(REPAIR_KIND),
            AiDirective::Secure { .. } => has(SECURITY_KIND),
            AiDirective::Rescue { .. } => has(TRANSPORTER_KIND),
        }
    })
}

#[derive(bevy::ecs::system::SystemParam)]
pub struct NpcDoctrineControl<'w, 's> {
    world_config: Option<Res<'w, crate::world::config::WorldConfig>>,
    identities: Query<'w, 's, &'static EntityUuid>,
    ships: Query<
        'w,
        's,
        (
            &'static EntityUuid,
            Option<&'static EntityTagsSection>,
            &'static ShipConfigComponent,
            &'static mut BehaviourSection,
            &'static mut NpcDoctrineState,
            Has<crate::lockstep::FleetSlotOf>,
            Has<crate::civilian::server::CivilianTraffic>,
            Option<&'static mut crate::ai::server::ObjectiveCursors>,
            Option<&'static crate::server_app::ShipSystemBlackboards>,
        ),
        With<crate::server_app::Ship>,
    >,
}

impl NpcDoctrineControl<'_, '_> {
    /// A Live Inspector edit must still describe the exact field and authored
    /// choice at the canonical apply tick. The ordinary adapter owns every
    /// compatibility check and reconciliation after this read check succeeds.
    pub fn apply_checked(
        &mut self,
        runtime: &WorldContentRuntime,
        target: &str,
        id: &str,
        expected_revision: &str,
    ) -> (GmActionOutcome, Option<GmActionRefusalReason>) {
        let Some(profile) = runtime.gm_npc_doctrine_palette.iter().find(|p| p.id == id) else {
            return (
                GmActionOutcome::Refused,
                Some(GmActionRefusalReason::UnknownNpcDoctrine),
            );
        };
        let Some((_, _, _, behaviour, state, ..)) =
            self.ships.iter().find(|(uuid, ..)| uuid.0 == target)
        else {
            return (
                GmActionOutcome::Refused,
                Some(if self.identities.iter().any(|uuid| uuid.0 == target) {
                    GmActionRefusalReason::NpcDoctrineIncompatible
                } else {
                    GmActionRefusalReason::UnknownEntity
                }),
            );
        };
        if inspector::revision(target, &behaviour.0.doctrine, state, profile) != expected_revision {
            return (
                GmActionOutcome::Refused,
                Some(GmActionRefusalReason::AffectedStateChanged),
            );
        }
        self.apply(runtime, target, id)
    }

    pub fn apply(
        &mut self,
        runtime: &WorldContentRuntime,
        target: &str,
        id: &str,
    ) -> (GmActionOutcome, Option<GmActionRefusalReason>) {
        let Some(profile) = runtime
            .gm_npc_doctrine_palette
            .iter()
            .find(|profile| profile.id == id)
        else {
            return (
                GmActionOutcome::Refused,
                Some(GmActionRefusalReason::UnknownNpcDoctrine),
            );
        };
        let Some((_, tags, config, mut behaviour, mut state, fleet, civilian, cursors, _)) =
            self.ships.iter_mut().find(|(uuid, ..)| uuid.0 == target)
        else {
            return (
                GmActionOutcome::Refused,
                Some(if self.identities.iter().any(|uuid| uuid.0 == target) {
                    GmActionRefusalReason::NpcDoctrineIncompatible
                } else {
                    GmActionRefusalReason::UnknownEntity
                }),
            );
        };
        if !compatible(
            profile,
            target,
            tags,
            fleet,
            civilian,
            &config.0,
            runtime,
            self.world_config.as_ref().map(|config| &config.anchors),
        ) {
            return (
                GmActionOutcome::Refused,
                Some(GmActionRefusalReason::NpcDoctrineIncompatible),
            );
        }
        if state
            .0
            .as_ref()
            .is_some_and(|state| state.id == id && state.doctrine == profile.doctrine)
            && behaviour.0.doctrine == profile.doctrine
        {
            return (GmActionOutcome::NoOp, None);
        }
        let baseline = state
            .0
            .as_ref()
            .map(|state| state.baseline.clone())
            .unwrap_or_else(|| behaviour.0.doctrine.clone());
        if let Some(mut cursors) = cursors {
            // A replaced standing route starts at its own first waypoint. Keep
            // unrelated mission-objective progress intact.
            cursors.0.retain(|cursor| {
                !behaviour
                    .0
                    .doctrine
                    .iter()
                    .chain(&profile.doctrine)
                    .any(|objective| objective.id == cursor.objective_id)
            });
        }
        behaviour.0.doctrine = profile.doctrine.clone();
        state.0 = Some(AppliedNpcDoctrine {
            id: id.to_string(),
            doctrine: profile.doctrine.clone(),
            baseline,
        });
        (GmActionOutcome::Applied, None)
    }

    /// The GM doctrine `target` is currently running, for the inverse path.
    ///
    /// `None` for "no such NPC on this peer"; `Some(None)` for an NPC running
    /// its OWN authored doctrine, which has no palette id at all. The two are
    /// deliberately different answers: an inverse that could not tell them
    /// apart would restore a baseline onto an entity that never existed.
    pub fn applied_doctrine(&self, target: &str) -> Option<Option<String>> {
        self.ships
            .iter()
            .find(|(uuid, ..)| uuid.0 == target)
            .map(|(_, _, _, _, state, ..)| state.0.as_ref().map(|state| state.id.clone()))
    }

    /// Restore `target` to the doctrine it ran before an earlier GM action
    /// replaced it (issue #1442).
    ///
    /// `expected_current` is what that earlier action LEFT running. It is
    /// compared against live state here, at the canonical apply tick: if
    /// anything has moved this entity's doctrine since — another GM, a scenario
    /// trigger, a Rhai effect — the inverse is refused rather than overwriting
    /// the newer decision. Everything else about the entity may have changed
    /// freely; only this field is consulted.
    ///
    /// `restore` is the palette id to return to, or `None` for the entity's own
    /// authored doctrine, which is replayed from the baseline captured when the
    /// first GM doctrine landed. Nothing is fabricated: a palette id whose
    /// entry the world has since withdrawn is refused, not approximated.
    pub fn revert(
        &mut self,
        runtime: &WorldContentRuntime,
        target: &str,
        expected_current: Option<&str>,
        restore: Option<&str>,
    ) -> (GmActionOutcome, Option<GmActionRefusalReason>) {
        // A profile the world has withdrawn cannot be restored, and saying so
        // before touching the entity keeps the refusal free of side effects.
        let profile = match restore {
            None => None,
            Some(id) => {
                match runtime
                    .gm_npc_doctrine_palette
                    .iter()
                    .find(|profile| profile.id == id)
                {
                    Some(profile) => Some(profile),
                    None => {
                        return (
                            GmActionOutcome::Refused,
                            Some(GmActionRefusalReason::UnknownNpcDoctrine),
                        )
                    }
                }
            }
        };
        let anchors = self.world_config.as_ref().map(|config| &config.anchors);
        let Some((_, tags, config, mut behaviour, mut state, fleet, civilian, cursors, _)) =
            self.ships.iter_mut().find(|(uuid, ..)| uuid.0 == target)
        else {
            return (
                GmActionOutcome::Refused,
                Some(if self.identities.iter().any(|uuid| uuid.0 == target) {
                    GmActionRefusalReason::NpcDoctrineIncompatible
                } else {
                    GmActionRefusalReason::UnknownEntity
                }),
            );
        };
        if state.0.as_ref().map(|state| state.id.as_str()) != expected_current {
            return (
                GmActionOutcome::Refused,
                Some(GmActionRefusalReason::AffectedStateChanged),
            );
        }
        // Restoring an authored baseline needs the baseline, which only an
        // applied GM doctrine carries. Without one there is nothing recorded to
        // go back to, and inventing the entity's current doctrine as its
        // "original" would be a fabricated fact.
        let Some(applied) = state.0.clone() else {
            return (
                GmActionOutcome::Refused,
                Some(GmActionRefusalReason::AffectedStateChanged),
            );
        };
        if let Some(profile) = profile {
            if !compatible(
                profile, target, tags, fleet, civilian, &config.0, runtime, anchors,
            ) {
                return (
                    GmActionOutcome::Refused,
                    Some(GmActionRefusalReason::NpcDoctrineIncompatible),
                );
            }
        }
        let next = profile
            .map(|profile| profile.doctrine.clone())
            .unwrap_or_else(|| applied.baseline.clone());
        if let Some(mut cursors) = cursors {
            // A replaced standing route starts at its own first waypoint,
            // exactly as `apply` does. Unrelated mission-objective progress
            // stays intact.
            cursors.0.retain(|cursor| {
                !behaviour
                    .0
                    .doctrine
                    .iter()
                    .chain(&next)
                    .any(|objective| objective.id == cursor.objective_id)
            });
        }
        behaviour.0.doctrine = next.clone();
        state.0 = profile.map(|profile| AppliedNpcDoctrine {
            id: profile.id.clone(),
            doctrine: next,
            // The pre-GM baseline is captured once and carried through every
            // later change, so reverting twice still lands on the authored
            // doctrine rather than on an intermediate GM choice.
            baseline: applied.baseline.clone(),
        });
        (GmActionOutcome::Applied, None)
    }

    pub fn projection(
        &self,
        runtime: &WorldContentRuntime,
    ) -> std::collections::BTreeMap<String, NpcDoctrineStatus> {
        self.ships
            .iter()
            .filter_map(
                |(uuid, tags, config, behaviour, state, fleet, civilian, _, blackboards)| {
                    if fleet
                        || civilian
                        || tags.is_some_and(|tags| tags.0.iter().any(|tag| tag == "player"))
                    {
                        return None;
                    }
                    let choices: Vec<_> = runtime
                        .gm_npc_doctrine_palette
                        .iter()
                        .filter(|profile| {
                            compatible(
                                profile,
                                &uuid.0,
                                tags,
                                fleet,
                                civilian,
                                &config.0,
                                runtime,
                                self.world_config.as_ref().map(|config| &config.anchors),
                            )
                        })
                        .map(|profile| NpcDoctrineChoice {
                            id: profile.id.clone(),
                            label: profile.label.clone(),
                            revision: inspector::revision(
                                &uuid.0,
                                &behaviour.0.doctrine,
                                state,
                                profile,
                            ),
                            origin_layer: profile.origin_layer.clone(),
                        })
                        .collect();
                    if choices.is_empty() && state.0.is_none() {
                        return None;
                    }
                    let intent = blackboards
                        .and_then(|boards| {
                            boards
                                .0
                                .get(&crate::ship::system_registry::viewscreen_system_id())
                        })
                        .and_then(|board| match board {
                            crate::core::messages::SystemBlackboard::Viewscreen(board) => {
                                Some(board)
                            }
                            _ => None,
                        })
                        .and_then(|board| {
                            board
                                .scored_objectives
                                .iter()
                                .find(|objective| objective.score > 0.0)
                        })
                        .map(|objective| objective.snapshot.text.clone());
                    Some((
                        uuid.0.clone(),
                        NpcDoctrineStatus {
                            current: state.0.as_ref().map(|state| state.id.clone()),
                            intent,
                            choices,
                            inspector: inspector::NpcInspector::default(),
                        },
                    ))
                },
            )
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NpcDoctrineChoice {
    pub id: String,
    pub label: String,
    pub revision: String,
    pub origin_layer: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NpcDoctrineStatus {
    pub current: Option<String>,
    pub intent: Option<String>,
    pub choices: Vec<NpcDoctrineChoice>,
    pub inspector: inspector::NpcInspector,
}

/// The scenario action's adapter uses the very same apply-boundary operation as
/// a canonical GM grant. Script scheduling only decides when this is called.
pub fn apply_scenario_command(
    In((target, id)): In<(String, String)>,
    runtime: Res<WorldContentRuntime>,
    mut control: NpcDoctrineControl,
) {
    let (outcome, reason) = control.apply(&runtime, &target, &id);
    if outcome == GmActionOutcome::Refused {
        bevy::log::warn!("NPC doctrine '{id}' for '{target}' refused: {reason:?}");
    }
}

#[cfg(test)]
#[path = "gm_npc_tests.rs"]
mod tests;

pub use phoenix_sim_world::gm_npc::*;
