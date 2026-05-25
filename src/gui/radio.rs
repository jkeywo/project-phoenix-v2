//! `RadioGroup` widget — exclusive-selection container over `GuiButton` members.
//!
//! Pressing any member atomically sets it active and deactivates all siblings,
//! then fires `RadioSelected { member: Entity }` on the group entity.

use bevy::prelude::*;

use super::{ButtonSize, GuiButtonMarker, StateVisuals, WidgetState, ButtonPressed};
use super::button::spawn_gui_button;

// ── Observer event ────────────────────────────────────────────────────────────

/// Fired on the `RadioGroup` entity after a member is selected.
#[derive(EntityEvent, Clone, Debug)]
pub struct RadioSelected {
    /// Target: the group entity the event is triggered on.
    #[event_target]
    pub entity: Entity,
    /// The member button entity that was just activated.
    pub member: Entity,
}

// ── Components ────────────────────────────────────────────────────────────────

/// Marker on the root group entity.
#[derive(Component, Default)]
pub struct RadioGroupMarker;

/// Attached to each member button.  Stores a back-reference to the group so
/// the observer can locate siblings without an extra query join.
#[derive(Component, Clone, Copy, Debug)]
pub struct RadioMember {
    pub group: Entity,
}

// ── Button config ─────────────────────────────────────────────────────────────

/// Configuration for one button within a `RadioGroup::spawn` call.
#[derive(Clone, Debug)]
pub struct RadioButtonConfig {
    pub size: ButtonSize,
}

// ── Pure helper ───────────────────────────────────────────────────────────────

/// Compute the next `active` flags for all members after selecting `selected`.
///
/// Returns a `Vec` of `(entity, is_active)` with exactly one `true` entry
/// (the selected member).  If `selected` is not in `members`, all become
/// `false`.
///
/// Pure function — fully unit-testable without a running `App`.
pub fn next_radio_selection(members: &[(Entity, bool)], selected: Entity) -> Vec<(Entity, bool)> {
    members.iter().map(|(e, _)| (*e, *e == selected)).collect()
}

// ── Spawn helper ──────────────────────────────────────────────────────────────

/// Namespace struct for the `RadioGroup` widget.
pub struct RadioGroup;

impl RadioGroup {
    /// Spawn a `RadioGroup` container with member buttons.
    ///
    /// - `button_configs` — one entry per button; order determines layout.
    /// - `state_visuals` — shared `StateVisuals` applied to every button.
    /// - `initial_selection` — index into `button_configs` for the
    ///   pre-selected member (defaults to none).
    ///
    /// Returns the group entity.  Attach additional `RadioSelected` observers
    /// via `commands.entity(group).observe(…)`.
    pub fn spawn(
        commands: &mut Commands,
        button_configs: Vec<RadioButtonConfig>,
        state_visuals: StateVisuals,
        initial_selection: Option<usize>,
    ) -> Entity {
        let group = commands
            .spawn((
                RadioGroupMarker,
                Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(4.0),
                    ..default()
                },
            ))
            .observe(|_: On<RadioSelected>| {})  // stub — callers add handlers
            .id();

        let mut member_entities: Vec<Entity> = Vec::new();

        for cfg in &button_configs {
            let btn = spawn_gui_button(
                commands,
                cfg.size.clone(),
                state_visuals.clone(),
            );
            commands.entity(btn).insert(RadioMember { group });
            commands.entity(btn).observe(on_radio_member_pressed);
            commands.entity(group).add_child(btn);
            member_entities.push(btn);
        }

        // Apply initial selection
        if let Some(idx) = initial_selection {
            if let Some(&initial_btn) = member_entities.get(idx) {
                if let Ok(mut entity_cmds) = commands.get_entity(initial_btn) {
                    entity_cmds.insert(WidgetState { active: true });
                }
            }
        }

        group
    }
}

// ── Observer ──────────────────────────────────────────────────────────────────

/// Observer attached to every `RadioGroup` member button.  Public so that
/// callers that manually add `RadioMember` to a `GuiButton` (e.g. when
/// absolute positioning prevents using `RadioGroup::spawn` directly) can
/// wire the same observer.
pub fn on_radio_member_pressed(
    trigger: On<ButtonPressed>,
    members: Query<(Entity, &RadioMember)>,
    mut widget_states: Query<&mut WidgetState, With<GuiButtonMarker>>,
    mut commands: Commands,
) {
    let pressed_entity = trigger.event().0;

    // Find the group this member belongs to.
    let Ok((_, pressed_member)) = members.get(pressed_entity) else { return };
    let group = pressed_member.group;

    // Atomically update all siblings.
    for (entity, member) in members.iter() {
        if member.group != group {
            continue;
        }
        if let Ok(mut state) = widget_states.get_mut(entity) {
            state.active = entity == pressed_entity;
        }
    }

    // Fire RadioSelected on the group entity.
    commands
        .entity(group)
        .trigger(|e| RadioSelected { entity: e, member: pressed_entity });
}

// ── Plugin ────────────────────────────────────────────────────────────────────

/// Sub-plugin for the radio group widget.  Registered automatically by `GuiPlugin`.
pub struct GuiRadioPlugin;

impl Plugin for GuiRadioPlugin {
    fn build(&self, app: &mut App) {
        // No additional systems needed — all logic is driven by the
        // per-entity ButtonPressed observer attached at spawn.
        let _ = app;
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entity(id: u64) -> Entity {
        Entity::from_bits(id)
    }

    #[test]
    fn selecting_b_when_a_is_active_leaves_only_b_active() {
        let a = make_entity(1u64);
        let b = make_entity(2u64);
        let c = make_entity(3u64);
        let members = vec![(a, true), (b, false), (c, false)];

        let result = next_radio_selection(&members, b);

        assert!(!result[0].1, "A should be inactive after selecting B");
        assert!( result[1].1, "B should be active after selecting B");
        assert!(!result[2].1, "C should be inactive after selecting B");
    }

    #[test]
    fn selecting_already_active_member_keeps_only_it_active() {
        let a = make_entity(1u64);
        let b = make_entity(2u64);
        let members = vec![(a, true), (b, false)];

        let result = next_radio_selection(&members, a);

        assert!(result[0].1, "A should remain active");
        assert!(!result[1].1, "B should remain inactive");
    }

    #[test]
    fn selecting_unknown_entity_deactivates_all() {
        let a = make_entity(1u64);
        let b = make_entity(2u64);
        let unknown = make_entity(99u64);
        let members = vec![(a, true), (b, false)];

        let result = next_radio_selection(&members, unknown);

        assert!(!result[0].1);
        assert!(!result[1].1);
    }

    #[test]
    fn empty_member_list_returns_empty() {
        let result = next_radio_selection(&[], make_entity(1u64));
        assert!(result.is_empty());
    }

    #[test]
    fn exactly_one_member_is_active_after_selection() {
        let entities: Vec<(Entity, bool)> = (1..=5)
            .map(|i| (make_entity(i as u64), i == 1))
            .collect();
        let selected = make_entity(3u64);
        let result = next_radio_selection(&entities, selected);
        let active_count = result.iter().filter(|(_, a)| *a).count();
        assert_eq!(active_count, 1);
        let active_entity = result.iter().find(|(_, a)| *a).unwrap().0;
        assert_eq!(active_entity, selected);
    }
}
