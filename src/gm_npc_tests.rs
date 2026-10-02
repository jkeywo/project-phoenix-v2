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
            authored_slot_id: None,
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
        objective_instance_scope: None,
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
