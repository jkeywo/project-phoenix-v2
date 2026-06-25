//! Server-side Comms console plugin (issue #427).
//!
//! Mirrors the `CaptainPlugin` pattern: a single `CommsConsoleStateComp`
//! entity holds the latest state snapshot. `recompute_comms_console_state`
//! rebuilds it each tick from live resources; `push_comms_console_state`
//! fires only when `Changed<CommsConsoleStateComp>` is detected, encoding
//! the state and emitting `ConsoleStateChanged { name: "Comms", json }` for
//! the wasm bridge to forward to the HTML panel.

use bevy::prelude::*;
use crate::ship_plugin::ShipSystemControlSources;
use crate::simulation::Ship;

use crate::console_bridge::ConsoleStateChanged;
use crate::messages::{CommsConsoleState, ObjectiveSnapshot};
use crate::world::server::ObjectiveManagerRes;
use crate::world::server::{CommsInboxRes, WorldContentRuntime};

pub struct CommsConsolePlugin;

impl Plugin for CommsConsolePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ConsoleStateChanged>();
        app.add_systems(Startup, spawn_comms_console_state_entity);
        app.add_systems(
            Update,
            (
                recompute_comms_console_state.in_set(crate::sim_sets::SimSet::Broadcast),
                push_comms_console_state.in_set(crate::sim_sets::SimSet::Broadcast),
                operate_comms_ai.in_set(crate::sim_sets::SimSet::Physics),
            ),
        );
    }
}

// ── Component ────────────────────────────────────────────────────────────────

#[derive(Component, Clone, PartialEq)]
pub struct CommsConsoleStateComp(pub CommsConsoleState);

fn spawn_comms_console_state_entity(mut commands: Commands) {
    commands.spawn(CommsConsoleStateComp(CommsConsoleState::default()));
}

// ── Recompute ────────────────────────────────────────────────────────────────

fn recompute_comms_console_state(
    inbox: Option<Res<CommsInboxRes>>,
    runtime: Option<Res<WorldContentRuntime>>,
    objectives: Option<Res<ObjectiveManagerRes>>,
    mut comp_q: Query<&mut CommsConsoleStateComp>,
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

    let next = CommsConsoleState {
        messages,
        objectives: objectives_snap,
        contacts,
    };

    for mut comp in comp_q.iter_mut() {
        if comp.0 != next {
            comp.0 = next.clone();
        }
    }
}

// ── Push ─────────────────────────────────────────────────────────────────────

fn push_comms_console_state(
    comp_q: Query<&CommsConsoleStateComp, Changed<CommsConsoleStateComp>>,
    mut writer: MessageWriter<ConsoleStateChanged>,
) {
    for comp in comp_q.iter() {
        if let Ok(json) = crate::core::codec::encode_console_state(&comp.0) {
            writer.write(ConsoleStateChanged {
                name: "Comms".into(),
                json,
            });
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::CommsMessage;
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

    /// Helper app with only spawn + recompute (no push, no message bus).
    fn recompute_test_app() -> App {
        let mut app = App::new();
        app.add_systems(Startup, spawn_comms_console_state_entity);
        app.add_systems(Update, recompute_comms_console_state);
        app.insert_resource(CommsInboxRes(crate::console::comms::CommsInbox::new()));
        app.insert_resource(WorldContentRuntime::default());
        app
    }

    /// Collect `ConsoleStateChanged` messages for push assertions.
    #[derive(Resource, Default)]
    struct ConsolePushes(Vec<ConsoleStateChanged>);

    fn collect_pushes(
        mut reader: MessageReader<ConsoleStateChanged>,
        mut sink: ResMut<ConsolePushes>,
    ) {
        for m in reader.read() {
            sink.0.push(m.clone());
        }
    }

    fn push_test_app() -> App {
        let mut app = App::new();
        app.add_message::<ConsoleStateChanged>()
            .init_resource::<ConsolePushes>()
            .add_systems(
                Update,
                (
                    push_comms_console_state,
                    collect_pushes.after(push_comms_console_state),
                ),
            );
        app.world_mut()
            .spawn(CommsConsoleStateComp(CommsConsoleState::default()));
        app
    }

    #[test]
    fn spawn_entity_exists_with_defaults() {
        let mut app = App::new();
        app.add_systems(Startup, spawn_comms_console_state_entity);
        app.update();

        let mut q = app.world_mut().query::<&CommsConsoleStateComp>();
        let comp = q.single(app.world()).unwrap();
        assert!(comp.0.messages.is_empty());
        assert!(comp.0.contacts.is_empty());
        assert!(comp.0.objectives.is_empty());
    }

    #[test]
    fn recompute_reflects_inbox_messages() {
        let mut app = recompute_test_app();
        app.world_mut()
            .resource_mut::<CommsInboxRes>()
            .0
            .inject(msg("m1"));
        app.update();

        let mut q = app.world_mut().query::<&CommsConsoleStateComp>();
        let comp = q.single(app.world()).unwrap();
        assert_eq!(comp.0.messages.len(), 1);
        assert_eq!(comp.0.messages[0].id, "m1");
    }

    #[test]
    fn push_fires_on_change_with_correct_name() {
        let mut app = push_test_app();

        // First tick: freshly spawned component is Changed → push fires.
        app.update();
        app.world_mut().resource_mut::<ConsolePushes>().0.clear();

        // Mutate component → should push exactly one ConsoleStateChanged.
        {
            let mut q = app.world_mut().query::<&mut CommsConsoleStateComp>();
            let mut comp = q.single_mut(app.world_mut()).unwrap();
            comp.0.messages.push(msg("m2"));
        }
        app.update();

        let pushes = &app.world().resource::<ConsolePushes>().0;
        assert_eq!(pushes.len(), 1, "expected one push after a change");
        assert_eq!(pushes[0].name, "Comms");
        assert!(
            pushes[0].json.contains("\"m2\""),
            "json: {}",
            pushes[0].json
        );

        // No further change → no further pushes.
        app.world_mut().resource_mut::<ConsolePushes>().0.clear();
        app.update();
        assert!(app.world().resource::<ConsolePushes>().0.is_empty());
    }

    #[test]
    fn push_does_not_fire_without_change() {
        let mut app = push_test_app();
        app.update();
        app.world_mut().resource_mut::<ConsolePushes>().0.clear();
        app.update();
        assert!(app.world().resource::<ConsolePushes>().0.is_empty());
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
