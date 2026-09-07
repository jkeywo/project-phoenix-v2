//! Scenario-authored NPC doctrine choices, applied through ordinary AI doctrine.
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::messages::AiDirective;
use crate::entities::config::DoctrineObjective;
use crate::entities::spawner::{BehaviourSection, EntityTagsSection, EntityUuid};
use crate::gm_action::{GmActionOutcome, GmActionRefusalReason};
use crate::ship::components::ShipConfigComponent;
use crate::world::server::WorldContentRuntime;

pub fn bounded_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}

/// Targets are an explicit authored allow-list of NPC names or UUIDs. Neither
/// the browser nor the action can replace the profile's doctrine or eligibility.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawNpcDoctrinePaletteEntry {
    pub id: String,
    pub label: String,
    pub targets: Vec<String>,
    pub doctrine: Vec<DoctrineObjective>,
}

#[derive(Clone, Debug)]
pub struct NpcDoctrinePaletteEntry {
    pub id: String,
    pub label: String,
    pub targets: Vec<String>,
    pub doctrine: Vec<DoctrineObjective>,
    pub origin_layer: Option<String>,
}

pub fn parse_palette(
    raw: &[RawNpcDoctrinePaletteEntry],
) -> Result<Vec<NpcDoctrinePaletteEntry>, String> {
    let mut ids = std::collections::BTreeSet::new();
    raw.iter()
        .map(|entry| {
            if !bounded_id(&entry.id)
                || entry.label.trim().is_empty()
                || !ids.insert(&entry.id)
                || entry.targets.is_empty()
                || entry.targets.len() > 32
                || entry.targets.iter().any(|target| !bounded_id(target))
                || entry.doctrine.len() > 32
            {
                return Err("invalid or duplicate GM NPC doctrine palette identity/targets".into());
            }
            crate::entities::config::validate_doctrine_directives(&entry.doctrine)?;
            let mut doctrine_ids = std::collections::BTreeSet::new();
            for objective in &entry.doctrine {
                if !bounded_id(&objective.id)
                    || !doctrine_ids.insert(&objective.id)
                    || !objective.base_priority.is_finite()
                    || !objective.target_speed.is_finite()
                    || !(0.0..=1.0).contains(&objective.target_speed)
                    || !objective.maintain_range.is_finite()
                    || objective.maintain_range < 0.0
                    || objective.modifiers.iter().any(|m| {
                        !m.weight.is_finite() || m.threshold.is_some_and(|v| !v.is_finite())
                    })
                    || objective
                        .zero_gates
                        .iter()
                        .any(|g| g.threshold.is_some_and(|v| !v.is_finite()))
                {
                    return Err(
                        "GM NPC doctrine requires unique bounded objective ids and finite tuning"
                            .into(),
                    );
                }
            }
            Ok(NpcDoctrinePaletteEntry {
                id: entry.id.clone(),
                label: entry.label.clone(),
                targets: entry.targets.clone(),
                doctrine: entry.doctrine.clone(),
                origin_layer: None,
            })
        })
        .collect()
}

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

    pub fn projection(
        &self,
        runtime: &WorldContentRuntime,
    ) -> std::collections::BTreeMap<String, NpcDoctrineStatus> {
        self.ships
            .iter()
            .filter_map(
                |(uuid, tags, config, _, state, fleet, civilian, _, blackboards)| {
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
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NpcDoctrineStatus {
    pub current: Option<String>,
    pub intent: Option<String>,
    pub choices: Vec<NpcDoctrineChoice>,
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
mod tests {
    use super::*;

    #[test]
    fn authored_npc_palette_uses_strict_directives_and_bounded_identities() {
        let source = include_str!("../tests/fixtures/worlds/gm_npc_doctrine.toml");
        let config = crate::world::config::parse_world(source).unwrap();
        assert_eq!(config.gm_npc_doctrine_palette.len(), 2);
        for malformed in [
            source.replace("id = \"north\"", "id = \"\""),
            source.replace("id = \"east\"", "id = \"north\""),
            source.replace("id = \"north\"", &format!("id = \"{}\"", "a".repeat(129))),
            source.replace(
                "directive_anchor = \"north\"",
                "directive_hail_target = \"north\"",
            ),
            source.replace("target_speed = 0.4", "target_speed = nan"),
            source.replace("target_speed = 0.4", "target_speed = 1.1"),
            source.replace(
                "label = \"Fly north\"",
                "label = \"Fly north\"\npolicy = \"aggressive\"",
            ),
        ] {
            assert!(
                crate::world::config::parse_world(&malformed).is_err(),
                "invalid authored palette was accepted"
            );
        }
    }

    #[test]
    fn npc_ingress_is_closed_and_replication_checks_the_actual_gm_binding() {
        use crate::{
            command_admission::HostSlot,
            gm_action::*,
            lockstep::{FleetGm, FleetRoster, FleetShip},
        };
        let wire = r#"{"operator_id":"gm-one","correlation":"npc-1","action":"set_npc_doctrine","target":"courier","doctrine":"north"}"#;
        let request = crate::core::codec::decode_gm_action_request(wire).unwrap();
        for bad in [
            wire.replace("\"north\"", "\"\""),
            wire.replace(
                "\"doctrine\":\"north\"",
                "\"doctrine\":\"north\",\"policy\":{}",
            ),
            wire.replace("\"target\":\"courier\"", "\"controls\":{}"),
        ] {
            assert!(crate::core::codec::decode_gm_action_request(&bad).is_none());
        }
        let roster = FleetRoster::with_participants_and_gms(
            vec![FleetShip {
                host: HostSlot(1),
                ship_path: Some("assets/entities/alliance_cruiser.toml".into()),
                crew: vec![],
            }],
            vec![HostSlot(1), HostSlot(2)],
            vec![FleetGm {
                host: HostSlot(2),
                operator_id: "gm-one".into(),
            }],
            HostSlot(1),
            HostSlot(1),
        )
        .unwrap();
        let mut proposal = GmActionProposal {
            from: HostSlot(1),
            operator_id: request.operator_id,
            correlation: request.correlation,
            action: request.action,
        };
        assert_eq!(
            validate_fleet_frame(&GmActionFrame::Proposal(proposal.clone()), &roster),
            Err(GmActionRefusalReason::OperatorMismatch)
        );
        proposal.from = HostSlot(2);
        assert!(validate_fleet_frame(&GmActionFrame::Proposal(proposal.clone()), &roster).is_ok());
        let mut refusal = GmActionRefusal {
            sequenced_by: HostSlot(1),
            requester: HostSlot(2),
            operator_id: proposal.operator_id,
            correlation: proposal.correlation,
            action_kind: GmActionKind::NpcDoctrine,
            requested_active: true,
            tick: 8,
            reason: GmActionRefusalReason::NpcDoctrineIncompatible,
            target: Some("courier".into()),
            verb: None,
            lever: None,
            effect_scope: None,
            objective_verb: None,
            objective_recipients: None,
            comms_recipients: None,
            observer: None,
            npc_doctrine: Some("north".into()),
        };
        assert_eq!(refusal.logged().npc_doctrine.as_deref(), Some("north"));
        assert!(validate_fleet_frame(&GmActionFrame::Refused(refusal.clone()), &roster).is_ok());
        refusal.npc_doctrine = None;
        assert_eq!(
            validate_fleet_frame(&GmActionFrame::Refused(refusal), &roster),
            Err(GmActionRefusalReason::InvalidAction)
        );
    }

    #[test]
    fn default_npc_state_preserves_existing_digest_and_applied_contents_are_authoritative() {
        let mut world = World::new();
        let entity = world.spawn(EntityUuid("courier".into())).id();
        let original = crate::sim_digest::world_digest(&world);
        world.entity_mut(entity).insert(NpcDoctrineState::default());
        assert_eq!(crate::sim_digest::world_digest(&world), original);
        let profile = crate::world::config::parse_world(include_str!(
            "../tests/fixtures/worlds/gm_npc_doctrine.toml"
        ))
        .unwrap()
        .gm_npc_doctrine_palette
        .remove(0);
        let mut state = AppliedNpcDoctrine {
            id: profile.id,
            doctrine: profile.doctrine,
            baseline: vec![],
        };
        state.doctrine[0]
            .modifiers
            .push(crate::objectives::ConditionModifier {
                condition: "hull_below".into(),
                threshold: Some(0.5),
                weight: 8.0,
            });
        state.doctrine[0]
            .zero_gates
            .push(crate::objectives::ZeroGateCondition {
                condition: "red_alert".into(),
                threshold: None,
            });
        assert!(!vellum_digest::ShareCodec::new("NPC-TEST-")
            .encode(&applied_digest_fields(&state))
            .unwrap()
            .is_empty());
        let saved = ron::ser::to_string(&state).unwrap();
        assert_eq!(ron::from_str::<AppliedNpcDoctrine>(&saved).unwrap(), state);
        let fold = |world: &mut World, state: &AppliedNpcDoctrine| {
            world
                .entity_mut(entity)
                .insert(NpcDoctrineState(Some(state.clone())));
            crate::sim_digest::world_digest(world)
        };
        let selected = fold(&mut world, &state);
        assert_ne!(selected, original);
        state.doctrine[0].target_speed += 0.1;
        let changed_contents = fold(&mut world, &state);
        assert_ne!(
            changed_contents, selected,
            "same identity, different ordinary AI tuning"
        );
        state.baseline = state.doctrine.clone();
        let changed_baseline = fold(&mut world, &state);
        assert_ne!(
            changed_baseline, changed_contents,
            "restoration baseline is authoritative"
        );
        state.id = "another-authored-choice".into();
        assert_ne!(
            fold(&mut world, &state),
            changed_baseline,
            "selected identity is authoritative"
        );
    }
}
