//! Server-side Comms console plugin (issue #427, migrated to blackboard #565).

use bevy::prelude::*;
use crate::ship_plugin::ShipSystemControlSources;
use crate::simulation::Ship;

use crate::messages::{CommsBlackboard, ObjectiveSnapshot, SystemBlackboard, SystemId};
use crate::world::server::ObjectiveManagerRes;
use crate::world::server::{CommsInboxRes, WorldContentRuntime};

pub struct CommsConsolePlugin;

impl Plugin for CommsConsolePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                publish_comms_blackboard.in_set(crate::sim_sets::SimSet::Publish),
                operate_comms_ai.in_set(crate::sim_sets::SimSet::Physics),
            ),
        );
    }
}

// ── Blackboard publish ────────────────────────────────────────────────────────

fn publish_comms_blackboard(
    inbox: Option<Res<CommsInboxRes>>,
    runtime: Option<Res<WorldContentRuntime>>,
    objectives: Option<Res<ObjectiveManagerRes>>,
    mut blackboards: ResMut<crate::server_app::SystemBlackboards>,
) {
    let mut messages = inbox.as_ref().map(|r| r.0.messages()).unwrap_or_default();

    if let Some(rt) = runtime.as_ref() {
        for m in messages.iter_mut() {
            if let Some(flag) = rt.range_flags.get(&m.sender_uuid).copied() {
                m.sender_in_range = flag;
            } else if rt.range_active && uuid::Uuid::parse_str(&m.sender_uuid).is_ok() {
                m.sender_in_range = false;
            }
        }
    }

    let objectives_snap: Vec<ObjectiveSnapshot> = objectives
        .as_ref()
        .map(|o| o.0.sorted_snapshots())
        .unwrap_or_default();

    let mut contacts = runtime
        .as_ref()
        .map(|rt| rt.contacts.clone())
        .unwrap_or_default();
    for contact in contacts.iter_mut() {
        contact.is_urgent = messages
            .iter()
            .any(|m| m.sender_uuid == contact.uuid && m.is_urgent && !m.is_read);
    }

    let bb = CommsBlackboard {
        messages,
        objectives: objectives_snap,
        contacts,
    };

    blackboards.0.insert(
        SystemId(crate::system_registry::COMMS_SYSTEM_ID.to_string()),
        SystemBlackboard::Comms(bb),
    );
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::CommsMessage;
    use crate::server_app::SystemBlackboards;
    use crate::world::server::CommsInboxRes;

    fn msg(id: &str) -> CommsMessage {
        CommsMessage {
            id: id.into(),
            sender_uuid: "sender-uuid".into(),
            sender_name: "Station Alpha".into(),
            subject: "Test".into(),
            body: "Body text".into(),
            responses: vec!["OK".into()],
            selected_response: None,
            is_read: false,
            is_orphaned: false,
            sender_in_range: true,
            thread_id: id.into(),
            is_urgent: false,
        }
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.init_resource::<SystemBlackboards>()
            .insert_resource(CommsInboxRes(crate::console::comms::CommsInbox::new()))
            .insert_resource(WorldContentRuntime::default())
            .add_systems(Update, publish_comms_blackboard);
        app
    }

    fn comms_bb(app: &App) -> CommsBlackboard {
        let bbs = app.world().resource::<SystemBlackboards>();
        let key = SystemId(crate::system_registry::COMMS_SYSTEM_ID.to_string());
        let SystemBlackboard::Comms(bb) = bbs.0.get(&key).expect("comms blackboard missing").clone()
        else { panic!("wrong blackboard variant"); };
        bb
    }

    #[test]
    fn blackboard_reflects_inbox_messages() {
        let mut app = test_app();
        app.world_mut()
            .resource_mut::<CommsInboxRes>()
            .0
            .inject(msg("m1"));
        app.update();

        let bb = comms_bb(&app);
        assert_eq!(bb.messages.len(), 1);
        assert_eq!(bb.messages[0].id, "m1");
    }
}

// ── AI controller stub ─────────────────────────────────────────────────────────

/// Per-kind AI plugin for comms.
///
/// Gated on policy.operate_ai for the Comms system. No behaviour is
/// implemented yet — this is a compile-verified stub that will be filled in
/// when the Comms AI controller is designed.
fn operate_comms_ai(
    ships: Query<&ShipSystemControlSources, With<Ship>>,
) {
    for sources in &ships {
        let policy = sources
            .0
            .policy_for(&crate::system_registry::comms_system_id());
        if !policy.operate_ai {
            continue;
        }
        // TODO: implement comms AI logic
    }
}
