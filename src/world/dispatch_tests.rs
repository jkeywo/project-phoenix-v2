use super::*;
use crate::ai::faction::FactionConfig;
use crate::world::load::MemoryTemplateLoader;

/// Deterministic stand-in for `entity_loader::assign_uuid()`.
const STUB_UUID: &str = "stub-uuid-0001";

fn stub_uuid() -> String {
    STUB_UUID.to_string()
}

/// Template path the `spawn()` helper names.
const DESTROYER_TEMPLATE: &str = "assets/entities/destroyer.toml";

/// A minimal template config with a display name, so tests can observe
/// the trigger `name` overwriting it (or not, for an empty trigger name).
///
/// `mass` is set explicitly to what a real `from_toml` parse of an
/// unauthored-mass template would produce (issue #1154): the bare
/// `#[derive(Default)]` on `EntityConfig` gives `mass` its type default
/// (`0.0`), not `default_mass()`'s `DEFAULT_ENTITY_MASS` — only serde
/// deserialisation runs the field-level `#[serde(default = ...)]`. Left
/// at `0.0`, this stand-in template would fail `validate_mass` the moment
/// an override round-trips it through `apply_overrides`, unlike any
/// template a real loader would ever hand back.
fn destroyer_template() -> EntityConfig {
    EntityConfig {
        name: Some("Harrow Destroyer".to_string()),
        tags: vec!["npc".to_string()],
        mass: crate::entities::config::DEFAULT_ENTITY_MASS,
        ..Default::default()
    }
}

/// `destroyer_template()` as `dispatch_spawn_entity` patches and boxes
/// it: the trigger's `name` wins over the template's display name.
fn patched_destroyer_template(name: &str) -> Box<EntityConfig> {
    Box::new(EntityConfig {
        name: Some(name.to_string()),
        ..destroyer_template()
    })
}

/// Owned backing store for a `DispatchContext`. Tests mutate the public
/// fields then call `ctx()` to borrow a context out of it.
#[derive(Default)]
struct Fixture {
    origin_layer: Option<String>,
    entity_name: Option<String>,
    name_to_uuid: HashMap<String, String>,
    base_flags: FlagStore,
    layers: HashMap<String, LayerView>,
    base_anchors: HashMap<String, [f32; 3]>,
    factions: Option<FactionRegistry>,
    loader: MemoryTemplateLoader,
}

impl Fixture {
    fn new() -> Self {
        Self::default()
    }

    fn ctx(&self) -> DispatchContext<'_> {
        DispatchContext {
            origin_layer: self.origin_layer.clone(),
            entity_name: self.entity_name.clone(),
            name_to_uuid: &self.name_to_uuid,
            base_flags: &self.base_flags,
            layers: &self.layers,
            base_anchors: &self.base_anchors,
            factions: self.factions.as_ref(),
            uuid_source: &stub_uuid,
            template_loader: &self.loader,
        }
    }

    fn with_entity(mut self, name: &str, uuid: &str) -> Self {
        self.name_to_uuid.insert(name.to_string(), uuid.to_string());
        self
    }

    /// Pre-load the destroyer template that `spawn()` names, so the
    /// spawn arm's template load succeeds.
    fn with_destroyer(mut self) -> Self {
        self.loader = self
            .loader
            .with_template(DESTROYER_TEMPLATE, destroyer_template());
        self
    }
}

fn layer(loader_path: Option<&str>) -> LayerView {
    LayerView {
        flags: FlagStore::new(),
        loader_path: loader_path.map(str::to_string),
        anchors: HashMap::new(),
    }
}

/// A registry with two named factions plus their UUIDs.
fn two_factions() -> (FactionRegistry, Uuid, Uuid) {
    let harrow = Uuid::from_u128(1);
    let federation = Uuid::from_u128(2);
    let mut registry = FactionRegistry::new();
    registry.insert(FactionConfig {
        display_name: None,
        uuid: harrow,
        name: "Harrow".to_string(),
        enemies: vec![],
        compliance: None,
    });
    registry.insert(FactionConfig {
        display_name: None,
        uuid: federation,
        name: "Federation".to_string(),
        enemies: vec![],
        compliance: None,
    });
    (registry, harrow, federation)
}

fn add_objective(targets: Vec<&str>) -> TriggerAction {
    TriggerAction::AddObjective {
        id: "obj1".to_string(),
        text: "Destroy the convoy".to_string(),
        text_params: Default::default(),
        mandatory: true,
        targets: targets.into_iter().map(str::to_string).collect(),
        directive: AiDirective::default(),
        utility: UtilityConfig::default(),
        source: ObjectiveSource::default(),
        command_stance: None,
    }
}

fn spawn(anchor: Option<&str>, position: Option<[f32; 3]>, groups: Vec<&str>) -> TriggerAction {
    TriggerAction::SpawnEntity {
        template_path: DESTROYER_TEMPLATE.to_string(),
        name: "wave_1".to_string(),
        anchor: anchor.map(str::to_string),
        position,
        rotation: None,
        scale: None,
        groups: groups.into_iter().map(str::to_string).collect(),
        overrides: None,
    }
}

// ── AddObjective ──────────────────────────────────────────────────────

#[test]
fn add_objective_uses_explicit_targets() {
    let mut fx = Fixture::new();
    fx.entity_name = Some("trigger_ship".to_string());
    let out = dispatch_action(&add_objective(vec!["alpha", "beta"]), &fx.ctx());

    assert_eq!(
        out.commands,
        vec![ActionCmd::AddObjective {
            id: "obj1".to_string(),
            text: "Destroy the convoy".to_string(),
            text_params: Default::default(),
            mandatory: true,
            targets: vec!["alpha".to_string(), "beta".to_string()],
            directive: AiDirective::default(),
            utility: UtilityConfig::default(),
            source: ObjectiveSource::default(),
            command_stance: None,
            origin_layer: None,
        }]
    );
    assert!(out.warnings.is_empty());
    assert!(out.new_events.is_empty());
}

#[test]
fn add_objective_empty_targets_falls_back_to_trigger_entity() {
    let mut fx = Fixture::new();
    fx.entity_name = Some("trigger_ship".to_string());
    let out = dispatch_action(&add_objective(vec![]), &fx.ctx());

    let ActionCmd::AddObjective { targets, .. } = &out.commands[0] else {
        panic!("expected AddObjective");
    };
    assert_eq!(targets, &vec!["trigger_ship".to_string()]);
}

#[test]
fn add_objective_empty_targets_and_no_entity_name_resolves_empty() {
    let fx = Fixture::new();
    let out = dispatch_action(&add_objective(vec![]), &fx.ctx());

    let ActionCmd::AddObjective { targets, .. } = &out.commands[0] else {
        panic!("expected AddObjective");
    };
    assert!(targets.is_empty());
}

// ── CompleteObjective / FailObjective ─────────────────────────────────

#[test]
fn complete_objective_emits_command() {
    let fx = Fixture::new();
    let action = TriggerAction::CompleteObjective {
        id: "obj1".to_string(),
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert_eq!(
        out.commands,
        vec![ActionCmd::CompleteObjective {
            id: "obj1".to_string()
        }]
    );
    assert!(out.warnings.is_empty());
}

#[test]
fn fail_objective_emits_command() {
    let fx = Fixture::new();
    let action = TriggerAction::FailObjective {
        id: "obj1".to_string(),
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert_eq!(
        out.commands,
        vec![ActionCmd::FailObjective {
            id: "obj1".to_string()
        }]
    );
}

// ── SetAiState ────────────────────────────────────────────────────────

#[test]
fn set_ai_state_is_a_noop_that_warns() {
    let fx = Fixture::new();
    let action = TriggerAction::SetAiState {
        entity: "raider".to_string(),
        state: "Attack".to_string(),
        target: Some("player".to_string()),
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert!(out.commands.is_empty());
    assert!(out.new_events.is_empty());
    assert_eq!(out.warnings.len(), 1);
    assert!(out.warnings[0].contains("doctrine-based AI"));
}

// ── Modifier arms ─────────────────────────────────────────────────────

#[test]
fn apply_modifier_resolves_name_to_uuid() {
    let fx = Fixture::new().with_entity("raider", "uuid-raider");
    let action = TriggerAction::ApplyModifier {
        entity: "raider".to_string(),
        tag: "buff".to_string(),
        slot: ModifierSlot::MaxSpeed,
        bonus: 2.5,
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert_eq!(
        out.commands,
        vec![ActionCmd::ApplyModifier {
            uuid: "uuid-raider".to_string(),
            tag: "buff".to_string(),
            slot: ModifierSlot::MaxSpeed,
            bonus: 2.5,
        }]
    );
    assert!(out.warnings.is_empty());
}

#[test]
fn apply_modifier_unknown_entity_warns_and_emits_nothing() {
    let fx = Fixture::new();
    let action = TriggerAction::ApplyModifier {
        entity: "ghost".to_string(),
        tag: "buff".to_string(),
        slot: ModifierSlot::MaxSpeed,
        bonus: 2.5,
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert!(out.commands.is_empty());
    assert_eq!(
        out.warnings,
        vec!["ApplyModifier: unknown entity name 'ghost'".to_string()]
    );
}

#[test]
fn remove_modifier_resolves_name_to_uuid() {
    let fx = Fixture::new().with_entity("raider", "uuid-raider");
    let action = TriggerAction::RemoveModifier {
        entity: "raider".to_string(),
        tag: "buff".to_string(),
        slot: ModifierSlot::MaxSpeed,
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert_eq!(
        out.commands,
        vec![ActionCmd::RemoveModifier {
            uuid: "uuid-raider".to_string(),
            tag: "buff".to_string(),
            slot: ModifierSlot::MaxSpeed,
        }]
    );
}

#[test]
fn remove_modifier_unknown_entity_warns_and_emits_nothing() {
    let fx = Fixture::new();
    let action = TriggerAction::RemoveModifier {
        entity: "ghost".to_string(),
        tag: "buff".to_string(),
        slot: ModifierSlot::MaxSpeed,
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert!(out.commands.is_empty());
    assert_eq!(
        out.warnings,
        vec!["RemoveModifier: unknown entity name 'ghost'".to_string()]
    );
}

#[test]
fn apply_flag_resolves_name_to_uuid() {
    let fx = Fixture::new().with_entity("raider", "uuid-raider");
    let action = TriggerAction::ApplyFlag {
        entity: "raider".to_string(),
        tag: "cloak".to_string(),
        kind: FlagKind::CommsJammed,
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert_eq!(
        out.commands,
        vec![ActionCmd::ApplyFlag {
            uuid: "uuid-raider".to_string(),
            tag: "cloak".to_string(),
            kind: FlagKind::CommsJammed,
        }]
    );
}

#[test]
fn apply_flag_unknown_entity_warns_and_emits_nothing() {
    let fx = Fixture::new();
    let action = TriggerAction::ApplyFlag {
        entity: "ghost".to_string(),
        tag: "cloak".to_string(),
        kind: FlagKind::CommsJammed,
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert!(out.commands.is_empty());
    assert_eq!(
        out.warnings,
        vec!["ApplyFlag: unknown entity name 'ghost'".to_string()]
    );
}

#[test]
fn remove_flag_resolves_name_to_uuid() {
    let fx = Fixture::new().with_entity("raider", "uuid-raider");
    let action = TriggerAction::RemoveFlag {
        entity: "raider".to_string(),
        tag: "cloak".to_string(),
        kind: FlagKind::CommsJammed,
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert_eq!(
        out.commands,
        vec![ActionCmd::RemoveFlag {
            uuid: "uuid-raider".to_string(),
            tag: "cloak".to_string(),
            kind: FlagKind::CommsJammed,
        }]
    );
}

#[test]
fn remove_flag_unknown_entity_warns_and_emits_nothing() {
    let fx = Fixture::new();
    let action = TriggerAction::RemoveFlag {
        entity: "ghost".to_string(),
        tag: "cloak".to_string(),
        kind: FlagKind::CommsJammed,
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert!(out.commands.is_empty());
    assert_eq!(
        out.warnings,
        vec!["RemoveFlag: unknown entity name 'ghost'".to_string()]
    );
}

#[test]
fn apply_int_modifier_resolves_name_to_uuid() {
    let fx = Fixture::new().with_entity("raider", "uuid-raider");
    let action = TriggerAction::ApplyIntModifier {
        entity: "raider".to_string(),
        tag: "crew".to_string(),
        slot: IntModifierSlot::RepairTeams,
        bonus: 3,
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert_eq!(
        out.commands,
        vec![ActionCmd::ApplyIntModifier {
            uuid: "uuid-raider".to_string(),
            tag: "crew".to_string(),
            slot: IntModifierSlot::RepairTeams,
            bonus: 3,
        }]
    );
}

#[test]
fn apply_int_modifier_unknown_entity_warns_and_emits_nothing() {
    let fx = Fixture::new();
    let action = TriggerAction::ApplyIntModifier {
        entity: "ghost".to_string(),
        tag: "crew".to_string(),
        slot: IntModifierSlot::RepairTeams,
        bonus: 3,
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert!(out.commands.is_empty());
    assert_eq!(
        out.warnings,
        vec!["ApplyIntModifier: unknown entity name 'ghost'".to_string()]
    );
}

#[test]
fn remove_int_modifier_resolves_name_to_uuid() {
    let fx = Fixture::new().with_entity("raider", "uuid-raider");
    let action = TriggerAction::RemoveIntModifier {
        entity: "raider".to_string(),
        tag: "crew".to_string(),
        slot: IntModifierSlot::RepairTeams,
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert_eq!(
        out.commands,
        vec![ActionCmd::RemoveIntModifier {
            uuid: "uuid-raider".to_string(),
            tag: "crew".to_string(),
            slot: IntModifierSlot::RepairTeams,
        }]
    );
}

#[test]
fn remove_int_modifier_unknown_entity_warns_and_emits_nothing() {
    let fx = Fixture::new();
    let action = TriggerAction::RemoveIntModifier {
        entity: "ghost".to_string(),
        tag: "crew".to_string(),
        slot: IntModifierSlot::RepairTeams,
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert!(out.commands.is_empty());
    assert_eq!(
        out.warnings,
        vec!["RemoveIntModifier: unknown entity name 'ghost'".to_string()]
    );
}

// ── GameOver ──────────────────────────────────────────────────────────

#[test]
fn game_over_sets_reason_before_state() {
    let fx = Fixture::new();
    let action = TriggerAction::GameOver {
        message: Some("The ship was lost".to_string()),
        outcome: None,
    };
    let out = dispatch_action(&action, &fx.ctx());

    // Ordering is load-bearing: OnEnter(GameOver) reads the reason.
    assert_eq!(
        out.commands,
        vec![
            ActionCmd::SetGameOverReason {
                reason: "The ship was lost".to_string(),
                outcome: None,
            },
            ActionCmd::SetNextState {
                phase: GamePhase::GameOver
            },
        ]
    );
}

#[test]
fn game_over_without_message_yields_empty_reason_not_none() {
    let fx = Fixture::new();
    let action = TriggerAction::GameOver {
        message: None,
        outcome: None,
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert_eq!(
        out.commands[0],
        ActionCmd::SetGameOverReason {
            reason: String::new(),
            outcome: None,
        }
    );
}

// ── LoadWorld / UnloadWorld ───────────────────────────────────────────

#[test]
fn load_world_records_origin_layer_as_loader_path() {
    let mut fx = Fixture::new();
    fx.origin_layer = Some("worlds/sub.toml".to_string());
    let action = TriggerAction::LoadWorld {
        path: "worlds/next.toml".to_string(),
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert_eq!(
        out.commands,
        vec![ActionCmd::LoadWorld {
            path: "worlds/next.toml".to_string(),
            loader_path: Some("worlds/sub.toml".to_string()),
        }]
    );
}

#[test]
fn load_world_from_base_world_has_no_loader_path() {
    let fx = Fixture::new();
    let action = TriggerAction::LoadWorld {
        path: "worlds/next.toml".to_string(),
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert_eq!(
        out.commands,
        vec![ActionCmd::LoadWorld {
            path: "worlds/next.toml".to_string(),
            loader_path: None,
        }]
    );
}

#[test]
fn unload_world_emits_command() {
    let fx = Fixture::new();
    let action = TriggerAction::UnloadWorld {
        path: "worlds/sub.toml".to_string(),
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert_eq!(
        out.commands,
        vec![ActionCmd::UnloadWorld {
            path: "worlds/sub.toml".to_string()
        }]
    );
}

// ── SetWorldFlag ──────────────────────────────────────────────────────

#[test]
fn set_world_flag_emits_mutation_and_flag_set_event() {
    let fx = Fixture::new();
    let action = TriggerAction::SetWorldFlag {
        name: "alarm".to_string(),
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert_eq!(
        out.commands,
        vec![ActionCmd::MutateFlag {
            target_layer: None,
            name: "alarm".to_string(),
            mutation: FlagMutation::Set,
        }]
    );
    assert_eq!(
        out.new_events,
        vec![WorldEvent::FlagSet {
            name: "alarm".to_string(),
            origin_layer: None,
        }]
    );
}

/// Pins flag idempotence, which depends on `base_flags` being the *live*
/// store: a second `set_flag` of an armed flag must still command the
/// mutation but emit no transition, so a downstream `on_flag_set` trigger
/// fires exactly once no matter how many triggers set it
/// (`assets/worlds/before_the_fire.toml:275`). Passing a per-pass snapshot
/// instead would make both setters read `before = 0` and double-fire.
#[test]
fn set_world_flag_on_already_set_flag_emits_no_transition_event() {
    let mut fx = Fixture::new();
    fx.base_flags.set_flag_value("alarm", 1);
    let action = TriggerAction::SetWorldFlag {
        name: "alarm".to_string(),
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert_eq!(
        out.commands,
        vec![ActionCmd::MutateFlag {
            target_layer: None,
            name: "alarm".to_string(),
            mutation: FlagMutation::Set,
        }]
    );
    assert!(out.new_events.is_empty());
}

// ── ClearWorldFlag ────────────────────────────────────────────────────

#[test]
fn clear_world_flag_emits_mutation_and_flag_cleared_event() {
    let mut fx = Fixture::new();
    fx.base_flags.set_flag_value("alarm", 1);
    let action = TriggerAction::ClearWorldFlag {
        name: "alarm".to_string(),
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert_eq!(
        out.commands,
        vec![ActionCmd::MutateFlag {
            target_layer: None,
            name: "alarm".to_string(),
            mutation: FlagMutation::Clear,
        }]
    );
    assert_eq!(
        out.new_events,
        vec![WorldEvent::FlagCleared {
            name: "alarm".to_string(),
            origin_layer: None,
        }]
    );
}

#[test]
fn clear_world_flag_already_clear_mutates_without_an_event() {
    let fx = Fixture::new();
    let action = TriggerAction::ClearWorldFlag {
        name: "alarm".to_string(),
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert_eq!(out.commands.len(), 1);
    assert!(out.new_events.is_empty());
}

// ── IncrementWorldFlag ────────────────────────────────────────────────

#[test]
fn increment_world_flag_zero_to_nonzero_emits_flag_set() {
    let fx = Fixture::new();
    let action = TriggerAction::IncrementWorldFlag {
        name: "kills".to_string(),
        by: 1,
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert_eq!(
        out.commands,
        vec![ActionCmd::MutateFlag {
            target_layer: None,
            name: "kills".to_string(),
            mutation: FlagMutation::Increment(1),
        }]
    );
    assert_eq!(
        out.new_events,
        vec![WorldEvent::FlagSet {
            name: "kills".to_string(),
            origin_layer: None,
        }]
    );
}

#[test]
fn increment_world_flag_nonzero_to_nonzero_emits_no_event() {
    let mut fx = Fixture::new();
    fx.base_flags.set_flag_value("kills", 3);
    let action = TriggerAction::IncrementWorldFlag {
        name: "kills".to_string(),
        by: 2,
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert_eq!(out.commands.len(), 1);
    assert!(out.new_events.is_empty());
}

#[test]
fn increment_world_flag_to_zero_emits_flag_cleared() {
    let mut fx = Fixture::new();
    fx.base_flags.set_flag_value("kills", 2);
    let action = TriggerAction::IncrementWorldFlag {
        name: "kills".to_string(),
        by: -2,
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert_eq!(
        out.new_events,
        vec![WorldEvent::FlagCleared {
            name: "kills".to_string(),
            origin_layer: None,
        }]
    );
}

#[test]
fn increment_world_flag_overflow_does_not_panic() {
    let mut fx = Fixture::new();
    fx.base_flags.set_flag_value("kills", i64::MAX);
    let action = TriggerAction::IncrementWorldFlag {
        name: "kills".to_string(),
        by: 5,
    };
    let out = dispatch_action(&action, &fx.ctx());

    // Only the no-panic property is testable here: `ActionCmd::MutateFlag`
    // carries the mutation, not the resulting value, so saturating-vs-
    // wrapping is unobservable through `dispatch_action` — both stay
    // non-zero, so both emit one command and no event. `saturating_add`
    // fidelity is `FlagStore`'s contract and is tested at `flags.rs`.
    assert_eq!(out.commands.len(), 1);
    assert!(out.new_events.is_empty());
}

// ── SetWorldFlagValue ─────────────────────────────────────────────────

#[test]
fn set_world_flag_value_zero_emits_flag_cleared() {
    let mut fx = Fixture::new();
    fx.base_flags.set_flag_value("alarm", 7);
    let action = TriggerAction::SetWorldFlagValue {
        name: "alarm".to_string(),
        value: 0,
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert_eq!(
        out.commands,
        vec![ActionCmd::MutateFlag {
            target_layer: None,
            name: "alarm".to_string(),
            mutation: FlagMutation::SetValue(0),
        }]
    );
    assert_eq!(
        out.new_events,
        vec![WorldEvent::FlagCleared {
            name: "alarm".to_string(),
            origin_layer: None,
        }]
    );
}

#[test]
fn set_world_flag_value_nonzero_to_nonzero_emits_no_event() {
    let mut fx = Fixture::new();
    fx.base_flags.set_flag_value("alarm", 7);
    let action = TriggerAction::SetWorldFlagValue {
        name: "alarm".to_string(),
        value: 9,
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert_eq!(out.commands.len(), 1);
    assert!(out.new_events.is_empty());
}

// ── Flag layer walking ────────────────────────────────────────────────

#[test]
fn flag_without_prefix_targets_the_origin_layer() {
    let mut fx = Fixture::new();
    fx.origin_layer = Some("sub.toml".to_string());
    fx.layers.insert("sub.toml".to_string(), layer(None));
    let action = TriggerAction::SetWorldFlag {
        name: "alarm".to_string(),
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert_eq!(
        out.commands,
        vec![ActionCmd::MutateFlag {
            target_layer: Some("sub.toml".to_string()),
            name: "alarm".to_string(),
            mutation: FlagMutation::Set,
        }]
    );
    assert_eq!(
        out.new_events,
        vec![WorldEvent::FlagSet {
            name: "alarm".to_string(),
            origin_layer: Some("sub.toml".to_string()),
        }]
    );
}

#[test]
fn flag_parent_prefix_walks_one_layer_up_to_base() {
    let mut fx = Fixture::new();
    fx.origin_layer = Some("sub.toml".to_string());
    // `loader_path: None` = loaded by the base world.
    fx.layers.insert("sub.toml".to_string(), layer(None));
    let action = TriggerAction::SetWorldFlag {
        name: "parent:alarm".to_string(),
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert_eq!(
        out.commands,
        vec![ActionCmd::MutateFlag {
            target_layer: None,
            name: "alarm".to_string(),
            mutation: FlagMutation::Set,
        }]
    );
    assert_eq!(
        out.new_events,
        vec![WorldEvent::FlagSet {
            name: "alarm".to_string(),
            origin_layer: None,
        }]
    );
}

#[test]
fn flag_parent_prefix_reads_the_target_layers_store_not_the_origins() {
    let mut fx = Fixture::new();
    fx.origin_layer = Some("inner.toml".to_string());
    // inner was loaded by outer; outer was loaded by the base world.
    fx.layers
        .insert("inner.toml".to_string(), layer(Some("outer.toml")));
    let mut outer = layer(None);
    // Already set in the *parent* store: no transition should be emitted.
    outer.flags.set_flag_value("alarm", 1);
    fx.layers.insert("outer.toml".to_string(), outer);

    let action = TriggerAction::SetWorldFlag {
        name: "parent:alarm".to_string(),
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert_eq!(
        out.commands,
        vec![ActionCmd::MutateFlag {
            target_layer: Some("outer.toml".to_string()),
            name: "alarm".to_string(),
            mutation: FlagMutation::Set,
        }]
    );
    assert!(out.new_events.is_empty());
}

#[test]
fn flag_double_parent_prefix_walks_two_layers_up() {
    let mut fx = Fixture::new();
    fx.origin_layer = Some("inner.toml".to_string());
    fx.layers
        .insert("inner.toml".to_string(), layer(Some("outer.toml")));
    fx.layers.insert("outer.toml".to_string(), layer(None));

    let action = TriggerAction::SetWorldFlag {
        name: "parent:parent:alarm".to_string(),
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert_eq!(
        out.commands,
        vec![ActionCmd::MutateFlag {
            target_layer: None,
            name: "alarm".to_string(),
            mutation: FlagMutation::Set,
        }]
    );
}

#[test]
fn flag_walk_past_base_world_warns_and_emits_nothing() {
    // Origin is already the base world, so any `parent:` overruns.
    let fx = Fixture::new();
    let action = TriggerAction::SetWorldFlag {
        name: "parent:alarm".to_string(),
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert!(out.commands.is_empty());
    assert!(out.new_events.is_empty());
    assert_eq!(out.warnings.len(), 1);
    assert!(out.warnings[0].contains("walks past base world"));
}

#[test]
fn flag_walk_past_base_world_from_a_layer_warns_and_emits_nothing() {
    let mut fx = Fixture::new();
    fx.origin_layer = Some("sub.toml".to_string());
    fx.layers.insert("sub.toml".to_string(), layer(None));
    // sub → base → overrun.
    let action = TriggerAction::SetWorldFlag {
        name: "parent:parent:alarm".to_string(),
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert!(out.commands.is_empty());
    assert!(out.new_events.is_empty());
    assert_eq!(out.warnings.len(), 1);
    assert!(out.warnings[0].contains("walks past base world"));
}

#[test]
fn flag_target_layer_missing_from_map_warns_and_emits_nothing() {
    let mut fx = Fixture::new();
    // The trigger's own layer is not in the map, and there is no `parent:`
    // to walk, so the resolved target is a layer we cannot find.
    fx.origin_layer = Some("ghost.toml".to_string());
    let action = TriggerAction::SetWorldFlag {
        name: "alarm".to_string(),
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert!(out.commands.is_empty());
    assert!(out.new_events.is_empty());
    assert_eq!(out.warnings.len(), 1);
    assert!(out.warnings[0].contains("missing from WorldLayerMap"));
}

#[test]
fn flag_layer_missing_mid_walk_is_silent_and_treated_as_base() {
    let mut fx = Fixture::new();
    // `ghost.toml` is absent from the map: the walk silently resolves its
    // loader_path to `None` (base) and carries on. This is deliberate —
    // only the *final* lookup warns.
    fx.origin_layer = Some("ghost.toml".to_string());
    let action = TriggerAction::SetWorldFlag {
        name: "parent:alarm".to_string(),
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert!(out.warnings.is_empty());
    assert_eq!(
        out.commands,
        vec![ActionCmd::MutateFlag {
            target_layer: None,
            name: "alarm".to_string(),
            mutation: FlagMutation::Set,
        }]
    );
    assert_eq!(
        out.new_events,
        vec![WorldEvent::FlagSet {
            name: "alarm".to_string(),
            origin_layer: None,
        }]
    );
}

// ── SpawnEntity ───────────────────────────────────────────────────────

#[test]
fn spawn_entity_with_explicit_position() {
    let fx = Fixture::new().with_destroyer();
    let out = dispatch_action(&spawn(None, Some([1.0, 2.0, 3.0]), vec![]), &fx.ctx());

    assert_eq!(
        out.commands,
        vec![ActionCmd::SpawnEntity {
            config: patched_destroyer_template("wave_1"),
            name: "wave_1".to_string(),
            uuid: STUB_UUID.to_string(),
            position: [1.0, 2.0, 3.0],
            rotation: None,
            scale: None,
            layer_path: None,
            template_path: DESTROYER_TEMPLATE.to_string(),
            overrides: None,
        }]
    );
    assert_eq!(
        out.name_to_uuid_inserts,
        vec![("wave_1".to_string(), STUB_UUID.to_string())]
    );
    assert!(out.entity_group_inserts.is_empty());
    assert!(out.warnings.is_empty());
}

#[test]
fn spawn_entity_resolves_anchor_from_base_world() {
    let mut fx = Fixture::new().with_destroyer();
    fx.base_anchors
        .insert("staging".to_string(), [10.0, 0.0, -5.0]);
    let out = dispatch_action(&spawn(Some("staging"), None, vec![]), &fx.ctx());

    let ActionCmd::SpawnEntity { position, .. } = &out.commands[0] else {
        panic!("expected SpawnEntity");
    };
    assert_eq!(position, &[10.0, 0.0, -5.0]);
}

#[test]
fn spawn_entity_resolves_anchor_from_the_origin_layer() {
    let mut fx = Fixture::new().with_destroyer();
    fx.origin_layer = Some("sub.toml".to_string());
    let mut sub = layer(None);
    sub.anchors.insert("staging".to_string(), [4.0, 5.0, 6.0]);
    fx.layers.insert("sub.toml".to_string(), sub);
    // A same-named base anchor must NOT win for a layer-authored trigger.
    fx.base_anchors
        .insert("staging".to_string(), [99.0, 99.0, 99.0]);

    let out = dispatch_action(&spawn(Some("staging"), None, vec![]), &fx.ctx());

    assert_eq!(
        out.commands,
        vec![ActionCmd::SpawnEntity {
            config: patched_destroyer_template("wave_1"),
            name: "wave_1".to_string(),
            uuid: STUB_UUID.to_string(),
            position: [4.0, 5.0, 6.0],
            rotation: None,
            scale: None,
            layer_path: Some("sub.toml".to_string()),
            template_path: DESTROYER_TEMPLATE.to_string(),
            overrides: None,
        }]
    );
}

#[test]
fn spawn_entity_position_wins_over_anchor() {
    let mut fx = Fixture::new().with_destroyer();
    fx.base_anchors
        .insert("staging".to_string(), [10.0, 0.0, -5.0]);
    let out = dispatch_action(
        &spawn(Some("staging"), Some([1.0, 1.0, 1.0]), vec![]),
        &fx.ctx(),
    );

    let ActionCmd::SpawnEntity { position, .. } = &out.commands[0] else {
        panic!("expected SpawnEntity");
    };
    assert_eq!(position, &[1.0, 1.0, 1.0]);
}

#[test]
fn spawn_entity_unknown_anchor_warns_and_emits_nothing() {
    let fx = Fixture::new();
    let out = dispatch_action(&spawn(Some("nowhere"), None, vec![]), &fx.ctx());

    assert!(out.commands.is_empty());
    assert!(out.name_to_uuid_inserts.is_empty());
    assert_eq!(
        out.warnings,
        vec!["SpawnEntity 'wave_1' anchor 'nowhere' not found".to_string()]
    );
}

#[test]
fn spawn_entity_without_anchor_or_position_warns_and_emits_nothing() {
    let fx = Fixture::new();
    let out = dispatch_action(&spawn(None, None, vec![]), &fx.ctx());

    assert!(out.commands.is_empty());
    assert!(out.name_to_uuid_inserts.is_empty());
    assert_eq!(
        out.warnings,
        vec!["SpawnEntity 'wave_1' has neither anchor nor position".to_string()]
    );
}

/// The contingency gate (issue #715): a spawn whose template fails to
/// resolve must produce NO command and NO name/group inserts — only a
/// warning. Before #715 this gate was the applier's `spawn_failed` local,
/// exercisable only through a full Bevy app.
#[test]
fn spawn_entity_template_not_found_warns_and_emits_nothing() {
    // No `.with_destroyer()`: the loader has no templates at all.
    let fx = Fixture::new();
    let out = dispatch_action(&spawn(None, Some([0.0, 0.0, 0.0]), vec!["wave"]), &fx.ctx());

    assert!(out.commands.is_empty());
    assert!(out.name_to_uuid_inserts.is_empty());
    assert!(out.entity_group_inserts.is_empty());
    assert_eq!(
        out.warnings,
        vec![
            "SpawnEntity 'wave_1' template 'assets/entities/destroyer.toml' not found".to_string()
        ]
    );
}

#[test]
fn spawn_entity_registers_every_group() {
    let fx = Fixture::new().with_destroyer();
    let out = dispatch_action(
        &spawn(None, Some([0.0, 0.0, 0.0]), vec!["wave", "hostiles"]),
        &fx.ctx(),
    );

    assert_eq!(
        out.entity_group_inserts,
        vec![
            ("wave".to_string(), "wave_1".to_string()),
            ("hostiles".to_string(), "wave_1".to_string()),
        ]
    );
}

#[test]
fn spawn_entity_carries_rotation_and_scale_through() {
    let fx = Fixture::new().with_destroyer();
    let action = TriggerAction::SpawnEntity {
        template_path: DESTROYER_TEMPLATE.to_string(),
        name: "wave_1".to_string(),
        anchor: None,
        position: Some([0.0, 0.0, 0.0]),
        rotation: Some([0.0, 1.57, 0.0]),
        scale: Some([2.0, 2.0, 2.0]),
        groups: vec![],
        overrides: None,
    };
    let out = dispatch_action(&action, &fx.ctx());

    let ActionCmd::SpawnEntity {
        rotation, scale, ..
    } = &out.commands[0]
    else {
        panic!("expected SpawnEntity");
    };
    assert_eq!(rotation, &Some([0.0, 1.57, 0.0]));
    assert_eq!(scale, &Some([2.0, 2.0, 2.0]));
}

#[test]
fn spawn_entity_uuid_comes_from_the_injected_source() {
    let fx = Fixture::new().with_destroyer();
    let counter = std::cell::Cell::new(0u32);
    let source = || {
        counter.set(counter.get() + 1);
        format!("uuid-{}", counter.get())
    };
    let ctx = DispatchContext {
        uuid_source: &source,
        ..fx.ctx()
    };

    let first = dispatch_action(&spawn(None, Some([0.0, 0.0, 0.0]), vec![]), &ctx);
    let second = dispatch_action(&spawn(None, Some([0.0, 0.0, 0.0]), vec![]), &ctx);

    assert_eq!(
        first.name_to_uuid_inserts,
        vec![("wave_1".to_string(), "uuid-1".to_string())]
    );
    assert_eq!(
        second.name_to_uuid_inserts,
        vec![("wave_1".to_string(), "uuid-2".to_string())]
    );
}

// ── DestroyEntity ─────────────────────────────────────────────────────

#[test]
fn destroy_entity_emits_command_and_destroyed_event() {
    let fx = Fixture::new().with_entity("wave_1", "uuid-wave-1");
    let action = TriggerAction::DestroyEntity {
        entity: "wave_1".to_string(),
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert_eq!(
        out.commands,
        vec![ActionCmd::DestroyEntity {
            uuid: "uuid-wave-1".to_string()
        }]
    );
    // The event lets chained `on_destroyed` triggers fire.
    assert_eq!(
        out.new_events,
        vec![WorldEvent::Destroyed {
            uuid: "uuid-wave-1".to_string()
        }]
    );
    assert!(out.warnings.is_empty());
}

#[test]
fn destroy_entity_unknown_name_warns_and_emits_nothing() {
    let fx = Fixture::new();
    let action = TriggerAction::DestroyEntity {
        entity: "ghost".to_string(),
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert!(out.commands.is_empty());
    assert!(out.new_events.is_empty());
    assert_eq!(
        out.warnings,
        vec!["DestroyEntity: unknown entity name 'ghost'".to_string()]
    );
}

// ── AddFactionEnemy ───────────────────────────────────────────────────

#[test]
fn add_faction_enemy_resolves_both_names_to_uuids() {
    let (registry, harrow, federation) = two_factions();
    let mut fx = Fixture::new();
    fx.factions = Some(registry);
    let action = TriggerAction::AddFactionEnemy {
        faction: "Harrow".to_string(),
        enemy: "Federation".to_string(),
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert_eq!(
        out.commands,
        vec![ActionCmd::AddFactionEnemy {
            faction_uuid: harrow,
            enemy_uuid: federation,
        }]
    );
    assert!(out.warnings.is_empty());
}

#[test]
fn add_faction_enemy_without_registry_warns_and_emits_nothing() {
    let fx = Fixture::new();
    let action = TriggerAction::AddFactionEnemy {
        faction: "Harrow".to_string(),
        enemy: "Federation".to_string(),
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert!(out.commands.is_empty());
    assert_eq!(
        out.warnings,
        vec!["AddFactionEnemy skipped: FactionRegistryResource not present".to_string()]
    );
}

#[test]
fn add_faction_enemy_unknown_faction_warns_and_emits_nothing() {
    let (registry, _, _) = two_factions();
    let mut fx = Fixture::new();
    fx.factions = Some(registry);
    let action = TriggerAction::AddFactionEnemy {
        faction: "Nobody".to_string(),
        enemy: "Federation".to_string(),
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert!(out.commands.is_empty());
    assert_eq!(
        out.warnings,
        vec!["AddFactionEnemy: unknown faction name 'Nobody'".to_string()]
    );
}

#[test]
fn add_faction_enemy_unknown_enemy_warns_and_emits_nothing() {
    let (registry, _, _) = two_factions();
    let mut fx = Fixture::new();
    fx.factions = Some(registry);
    let action = TriggerAction::AddFactionEnemy {
        faction: "Harrow".to_string(),
        enemy: "Nobody".to_string(),
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert!(out.commands.is_empty());
    assert_eq!(
        out.warnings,
        vec!["AddFactionEnemy: unknown enemy faction name 'Nobody'".to_string()]
    );
}

#[test]
fn faction_name_lookup_is_case_sensitive() {
    let (registry, _, _) = two_factions();
    let mut fx = Fixture::new();
    fx.factions = Some(registry);
    let action = TriggerAction::AddFactionEnemy {
        faction: "harrow".to_string(),
        enemy: "Federation".to_string(),
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert!(out.commands.is_empty());
    assert_eq!(out.warnings.len(), 1);
}

// ── RemoveFactionEnemy ────────────────────────────────────────────────

#[test]
fn remove_faction_enemy_resolves_both_names_to_uuids() {
    let (registry, harrow, federation) = two_factions();
    let mut fx = Fixture::new();
    fx.factions = Some(registry);
    let action = TriggerAction::RemoveFactionEnemy {
        faction: "Harrow".to_string(),
        enemy: "Federation".to_string(),
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert_eq!(
        out.commands,
        vec![ActionCmd::RemoveFactionEnemy {
            faction_uuid: harrow,
            enemy_uuid: federation,
        }]
    );
    assert!(out.warnings.is_empty());
}

#[test]
fn remove_faction_enemy_without_registry_warns_and_emits_nothing() {
    let fx = Fixture::new();
    let action = TriggerAction::RemoveFactionEnemy {
        faction: "Harrow".to_string(),
        enemy: "Federation".to_string(),
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert!(out.commands.is_empty());
    assert_eq!(
        out.warnings,
        vec!["RemoveFactionEnemy skipped: FactionRegistryResource not present".to_string()]
    );
}

#[test]
fn remove_faction_enemy_unknown_faction_warns_and_emits_nothing() {
    let (registry, _, _) = two_factions();
    let mut fx = Fixture::new();
    fx.factions = Some(registry);
    let action = TriggerAction::RemoveFactionEnemy {
        faction: "Nobody".to_string(),
        enemy: "Federation".to_string(),
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert!(out.commands.is_empty());
    assert_eq!(
        out.warnings,
        vec!["RemoveFactionEnemy: unknown faction name 'Nobody'".to_string()]
    );
}

#[test]
fn remove_faction_enemy_unknown_enemy_warns_and_emits_nothing() {
    let (registry, _, _) = two_factions();
    let mut fx = Fixture::new();
    fx.factions = Some(registry);
    let action = TriggerAction::RemoveFactionEnemy {
        faction: "Harrow".to_string(),
        enemy: "Nobody".to_string(),
    };
    let out = dispatch_action(&action, &fx.ctx());

    assert!(out.commands.is_empty());
    assert_eq!(
        out.warnings,
        vec!["RemoveFactionEnemy: unknown enemy faction name 'Nobody'".to_string()]
    );
}

// ── dispatch_state_action (direct) ────────────────────────────────────
//
// The tests above drive the six state arms through `dispatch_action`,
// proving the routing arm delegates. These call `dispatch_state_action`
// directly, proving the extracted function is what produces the result and
// that the two entry points agree (issue #711).

#[test]
fn state_add_objective_falls_back_to_trigger_entity_directly() {
    let mut fx = Fixture::new();
    fx.entity_name = Some("trigger_ship".to_string());
    let out = dispatch_state_action(&add_objective(vec![]), &fx.ctx());

    assert_eq!(
        out.commands,
        vec![ActionCmd::AddObjective {
            id: "obj1".to_string(),
            text: "Destroy the convoy".to_string(),
            text_params: Default::default(),
            mandatory: true,
            targets: vec!["trigger_ship".to_string()],
            directive: AiDirective::default(),
            utility: UtilityConfig::default(),
            source: ObjectiveSource::default(),
            command_stance: None,
            origin_layer: None,
        }]
    );
    assert!(out.warnings.is_empty());
    assert!(out.new_events.is_empty());
}

#[test]
fn state_add_objective_matches_dispatch_action_routing() {
    let mut fx = Fixture::new();
    fx.entity_name = Some("trigger_ship".to_string());
    let action = add_objective(vec!["alpha", "beta"]);

    // Both entry points must produce byte-identical results.
    assert_eq!(
        dispatch_action(&action, &fx.ctx()),
        dispatch_state_action(&action, &fx.ctx())
    );
}

#[test]
fn state_complete_objective_emits_command_directly() {
    let fx = Fixture::new();
    let action = TriggerAction::CompleteObjective {
        id: "obj1".to_string(),
    };
    let out = dispatch_state_action(&action, &fx.ctx());

    assert_eq!(
        out.commands,
        vec![ActionCmd::CompleteObjective {
            id: "obj1".to_string()
        }]
    );
    assert!(out.warnings.is_empty());
}

#[test]
fn state_fail_objective_emits_command_directly() {
    let fx = Fixture::new();
    let action = TriggerAction::FailObjective {
        id: "obj1".to_string(),
    };
    let out = dispatch_state_action(&action, &fx.ctx());

    assert_eq!(
        out.commands,
        vec![ActionCmd::FailObjective {
            id: "obj1".to_string()
        }]
    );
}

#[test]
fn state_game_over_sets_reason_before_state_directly() {
    let fx = Fixture::new();
    let action = TriggerAction::GameOver {
        message: Some("The ship was lost".to_string()),
        outcome: None,
    };
    let out = dispatch_state_action(&action, &fx.ctx());

    // Ordering is load-bearing: OnEnter(GameOver) reads the reason first.
    assert_eq!(
        out.commands,
        vec![
            ActionCmd::SetGameOverReason {
                reason: "The ship was lost".to_string(),
                outcome: None,
            },
            ActionCmd::SetNextState {
                phase: GamePhase::GameOver
            },
        ]
    );
}

#[test]
fn state_game_over_without_message_yields_empty_reason_not_none_directly() {
    let fx = Fixture::new();
    let action = TriggerAction::GameOver {
        message: None,
        outcome: None,
    };
    let out = dispatch_state_action(&action, &fx.ctx());

    assert_eq!(
        out.commands[0],
        ActionCmd::SetGameOverReason {
            reason: String::new(),
            outcome: None,
        }
    );
}

#[test]
fn state_add_faction_enemy_emits_command_directly() {
    let (registry, harrow, federation) = two_factions();
    let mut fx = Fixture::new();
    fx.factions = Some(registry);
    let action = TriggerAction::AddFactionEnemy {
        faction: "Harrow".to_string(),
        enemy: "Federation".to_string(),
    };
    let out = dispatch_state_action(&action, &fx.ctx());

    assert_eq!(
        out.commands,
        vec![ActionCmd::AddFactionEnemy {
            faction_uuid: harrow,
            enemy_uuid: federation,
        }]
    );
    assert!(out.warnings.is_empty());
}

#[test]
fn state_add_faction_enemy_without_registry_warns_directly() {
    let fx = Fixture::new();
    let action = TriggerAction::AddFactionEnemy {
        faction: "Harrow".to_string(),
        enemy: "Federation".to_string(),
    };
    let out = dispatch_state_action(&action, &fx.ctx());

    assert!(out.commands.is_empty());
    assert_eq!(
        out.warnings,
        vec!["AddFactionEnemy skipped: FactionRegistryResource not present".to_string()]
    );
}

#[test]
fn state_remove_faction_enemy_emits_command_directly() {
    let (registry, harrow, federation) = two_factions();
    let mut fx = Fixture::new();
    fx.factions = Some(registry);
    let action = TriggerAction::RemoveFactionEnemy {
        faction: "Harrow".to_string(),
        enemy: "Federation".to_string(),
    };
    let out = dispatch_state_action(&action, &fx.ctx());

    assert_eq!(
        out.commands,
        vec![ActionCmd::RemoveFactionEnemy {
            faction_uuid: harrow,
            enemy_uuid: federation,
        }]
    );
    assert!(out.warnings.is_empty());
}

#[test]
#[should_panic(expected = "dispatch_state_action called with non-state action")]
fn state_action_on_non_state_variant_panics() {
    // The guard exists so a routing bug in `dispatch_action` fails loudly
    // rather than silently returning an empty result.
    let fx = Fixture::new();
    let action = TriggerAction::UnloadWorld {
        path: "worlds/sub.toml".to_string(),
    };
    let _ = dispatch_state_action(&action, &fx.ctx());
}

// ── dispatch_entity_modifier_action (direct) ──────────────────────────
//
// The tests above drive the six modifier/flag arms through
// `dispatch_action`, proving the routing arm delegates. These call
// `dispatch_entity_modifier_action` directly, proving the extracted
// function is what produces the result and that the two entry points
// agree (issue #712).

#[test]
fn modifier_apply_modifier_resolves_name_to_uuid_directly() {
    let fx = Fixture::new().with_entity("raider", "uuid-raider");
    let action = TriggerAction::ApplyModifier {
        entity: "raider".to_string(),
        tag: "buff".to_string(),
        slot: ModifierSlot::MaxSpeed,
        bonus: 2.5,
    };
    let out = dispatch_entity_modifier_action(&action, &fx.ctx());

    assert_eq!(
        out,
        DispatchResult {
            commands: vec![ActionCmd::ApplyModifier {
                uuid: "uuid-raider".to_string(),
                tag: "buff".to_string(),
                slot: ModifierSlot::MaxSpeed,
                bonus: 2.5,
            }],
            ..Default::default()
        }
    );
}

#[test]
fn modifier_apply_modifier_unknown_entity_warns_directly() {
    let fx = Fixture::new();
    let action = TriggerAction::ApplyModifier {
        entity: "ghost".to_string(),
        tag: "buff".to_string(),
        slot: ModifierSlot::MaxSpeed,
        bonus: 2.5,
    };
    let out = dispatch_entity_modifier_action(&action, &fx.ctx());

    assert_eq!(
        out,
        DispatchResult {
            warnings: vec!["ApplyModifier: unknown entity name 'ghost'".to_string()],
            ..Default::default()
        }
    );
}

#[test]
fn modifier_apply_modifier_matches_dispatch_action_routing() {
    let fx = Fixture::new().with_entity("raider", "uuid-raider");
    let action = TriggerAction::ApplyModifier {
        entity: "raider".to_string(),
        tag: "buff".to_string(),
        slot: ModifierSlot::MaxSpeed,
        bonus: 2.5,
    };

    // Both entry points must produce byte-identical results.
    assert_eq!(
        dispatch_action(&action, &fx.ctx()),
        dispatch_entity_modifier_action(&action, &fx.ctx())
    );
}

#[test]
fn modifier_remove_modifier_resolves_name_to_uuid_directly() {
    let fx = Fixture::new().with_entity("raider", "uuid-raider");
    let action = TriggerAction::RemoveModifier {
        entity: "raider".to_string(),
        tag: "buff".to_string(),
        slot: ModifierSlot::MaxSpeed,
    };
    let out = dispatch_entity_modifier_action(&action, &fx.ctx());

    assert_eq!(
        out,
        DispatchResult {
            commands: vec![ActionCmd::RemoveModifier {
                uuid: "uuid-raider".to_string(),
                tag: "buff".to_string(),
                slot: ModifierSlot::MaxSpeed,
            }],
            ..Default::default()
        }
    );
}

#[test]
fn modifier_remove_modifier_unknown_entity_warns_directly() {
    let fx = Fixture::new();
    let action = TriggerAction::RemoveModifier {
        entity: "ghost".to_string(),
        tag: "buff".to_string(),
        slot: ModifierSlot::MaxSpeed,
    };
    let out = dispatch_entity_modifier_action(&action, &fx.ctx());

    assert_eq!(
        out,
        DispatchResult {
            warnings: vec!["RemoveModifier: unknown entity name 'ghost'".to_string()],
            ..Default::default()
        }
    );
}

#[test]
fn modifier_apply_flag_resolves_name_to_uuid_directly() {
    let fx = Fixture::new().with_entity("raider", "uuid-raider");
    let action = TriggerAction::ApplyFlag {
        entity: "raider".to_string(),
        tag: "cloak".to_string(),
        kind: FlagKind::CommsJammed,
    };
    let out = dispatch_entity_modifier_action(&action, &fx.ctx());

    assert_eq!(
        out,
        DispatchResult {
            commands: vec![ActionCmd::ApplyFlag {
                uuid: "uuid-raider".to_string(),
                tag: "cloak".to_string(),
                kind: FlagKind::CommsJammed,
            }],
            ..Default::default()
        }
    );
}

#[test]
fn modifier_apply_flag_unknown_entity_warns_directly() {
    let fx = Fixture::new();
    let action = TriggerAction::ApplyFlag {
        entity: "ghost".to_string(),
        tag: "cloak".to_string(),
        kind: FlagKind::CommsJammed,
    };
    let out = dispatch_entity_modifier_action(&action, &fx.ctx());

    assert_eq!(
        out,
        DispatchResult {
            warnings: vec!["ApplyFlag: unknown entity name 'ghost'".to_string()],
            ..Default::default()
        }
    );
}

#[test]
fn modifier_remove_flag_resolves_name_to_uuid_directly() {
    let fx = Fixture::new().with_entity("raider", "uuid-raider");
    let action = TriggerAction::RemoveFlag {
        entity: "raider".to_string(),
        tag: "cloak".to_string(),
        kind: FlagKind::CommsJammed,
    };
    let out = dispatch_entity_modifier_action(&action, &fx.ctx());

    assert_eq!(
        out,
        DispatchResult {
            commands: vec![ActionCmd::RemoveFlag {
                uuid: "uuid-raider".to_string(),
                tag: "cloak".to_string(),
                kind: FlagKind::CommsJammed,
            }],
            ..Default::default()
        }
    );
}

#[test]
fn modifier_remove_flag_unknown_entity_warns_directly() {
    let fx = Fixture::new();
    let action = TriggerAction::RemoveFlag {
        entity: "ghost".to_string(),
        tag: "cloak".to_string(),
        kind: FlagKind::CommsJammed,
    };
    let out = dispatch_entity_modifier_action(&action, &fx.ctx());

    assert_eq!(
        out,
        DispatchResult {
            warnings: vec!["RemoveFlag: unknown entity name 'ghost'".to_string()],
            ..Default::default()
        }
    );
}

#[test]
fn modifier_apply_int_modifier_resolves_name_to_uuid_directly() {
    let fx = Fixture::new().with_entity("raider", "uuid-raider");
    let action = TriggerAction::ApplyIntModifier {
        entity: "raider".to_string(),
        tag: "crew".to_string(),
        slot: IntModifierSlot::RepairTeams,
        bonus: 3,
    };
    let out = dispatch_entity_modifier_action(&action, &fx.ctx());

    assert_eq!(
        out,
        DispatchResult {
            commands: vec![ActionCmd::ApplyIntModifier {
                uuid: "uuid-raider".to_string(),
                tag: "crew".to_string(),
                slot: IntModifierSlot::RepairTeams,
                bonus: 3,
            }],
            ..Default::default()
        }
    );
}

#[test]
fn modifier_apply_int_modifier_unknown_entity_warns_directly() {
    let fx = Fixture::new();
    let action = TriggerAction::ApplyIntModifier {
        entity: "ghost".to_string(),
        tag: "crew".to_string(),
        slot: IntModifierSlot::RepairTeams,
        bonus: 3,
    };
    let out = dispatch_entity_modifier_action(&action, &fx.ctx());

    assert_eq!(
        out,
        DispatchResult {
            warnings: vec!["ApplyIntModifier: unknown entity name 'ghost'".to_string()],
            ..Default::default()
        }
    );
}

#[test]
fn modifier_remove_int_modifier_resolves_name_to_uuid_directly() {
    let fx = Fixture::new().with_entity("raider", "uuid-raider");
    let action = TriggerAction::RemoveIntModifier {
        entity: "raider".to_string(),
        tag: "crew".to_string(),
        slot: IntModifierSlot::RepairTeams,
    };
    let out = dispatch_entity_modifier_action(&action, &fx.ctx());

    assert_eq!(
        out,
        DispatchResult {
            commands: vec![ActionCmd::RemoveIntModifier {
                uuid: "uuid-raider".to_string(),
                tag: "crew".to_string(),
                slot: IntModifierSlot::RepairTeams,
            }],
            ..Default::default()
        }
    );
}

#[test]
fn modifier_remove_int_modifier_unknown_entity_warns_directly() {
    let fx = Fixture::new();
    let action = TriggerAction::RemoveIntModifier {
        entity: "ghost".to_string(),
        tag: "crew".to_string(),
        slot: IntModifierSlot::RepairTeams,
    };
    let out = dispatch_entity_modifier_action(&action, &fx.ctx());

    assert_eq!(
        out,
        DispatchResult {
            warnings: vec!["RemoveIntModifier: unknown entity name 'ghost'".to_string()],
            ..Default::default()
        }
    );
}

#[test]
#[should_panic(expected = "dispatch_entity_modifier_action called with non-modifier action")]
fn entity_modifier_action_on_non_modifier_variant_panics() {
    // The guard exists so a routing bug in `dispatch_action` fails loudly
    // rather than silently returning an empty result.
    let fx = Fixture::new();
    let action = TriggerAction::UnloadWorld {
        path: "worlds/sub.toml".to_string(),
    };
    let _ = dispatch_entity_modifier_action(&action, &fx.ctx());
}

// ── dispatch_world_flag_action (direct) ───────────────────────────────
//
// The tests above drive the four world-flag arms through
// `dispatch_action`, proving the routing arm delegates. These call
// `dispatch_world_flag_action` directly, proving the extracted function
// is what produces the result and that the two entry points agree
// (issue #713).

#[test]
fn world_flag_set_emits_mutation_and_flag_set_event_directly() {
    let fx = Fixture::new();
    let action = TriggerAction::SetWorldFlag {
        name: "alarm".to_string(),
    };
    let out = dispatch_world_flag_action(&action, &fx.ctx());

    assert_eq!(
        out,
        DispatchResult {
            commands: vec![ActionCmd::MutateFlag {
                target_layer: None,
                name: "alarm".to_string(),
                mutation: FlagMutation::Set,
            }],
            new_events: vec![WorldEvent::FlagSet {
                name: "alarm".to_string(),
                origin_layer: None,
            }],
            ..Default::default()
        }
    );
}

#[test]
fn world_flag_clear_emits_mutation_and_flag_cleared_event_directly() {
    let mut fx = Fixture::new();
    fx.base_flags.set_flag_value("alarm", 1);
    let action = TriggerAction::ClearWorldFlag {
        name: "alarm".to_string(),
    };
    let out = dispatch_world_flag_action(&action, &fx.ctx());

    assert_eq!(
        out,
        DispatchResult {
            commands: vec![ActionCmd::MutateFlag {
                target_layer: None,
                name: "alarm".to_string(),
                mutation: FlagMutation::Clear,
            }],
            new_events: vec![WorldEvent::FlagCleared {
                name: "alarm".to_string(),
                origin_layer: None,
            }],
            ..Default::default()
        }
    );
}

#[test]
fn world_flag_increment_zero_to_nonzero_emits_flag_set_directly() {
    let fx = Fixture::new();
    let action = TriggerAction::IncrementWorldFlag {
        name: "kills".to_string(),
        by: 2,
    };
    let out = dispatch_world_flag_action(&action, &fx.ctx());

    assert_eq!(
        out,
        DispatchResult {
            commands: vec![ActionCmd::MutateFlag {
                target_layer: None,
                name: "kills".to_string(),
                mutation: FlagMutation::Increment(2),
            }],
            new_events: vec![WorldEvent::FlagSet {
                name: "kills".to_string(),
                origin_layer: None,
            }],
            ..Default::default()
        }
    );
}

#[test]
fn world_flag_set_value_to_zero_emits_flag_cleared_directly() {
    let mut fx = Fixture::new();
    fx.base_flags.set_flag_value("alarm", 7);
    let action = TriggerAction::SetWorldFlagValue {
        name: "alarm".to_string(),
        value: 0,
    };
    let out = dispatch_world_flag_action(&action, &fx.ctx());

    assert_eq!(
        out,
        DispatchResult {
            commands: vec![ActionCmd::MutateFlag {
                target_layer: None,
                name: "alarm".to_string(),
                mutation: FlagMutation::SetValue(0),
            }],
            new_events: vec![WorldEvent::FlagCleared {
                name: "alarm".to_string(),
                origin_layer: None,
            }],
            ..Default::default()
        }
    );
}

/// Layer-chain happy path: a `parent:` prefix from a nested layer resolves
/// to its loader layer, and both the command and the event carry the
/// stripped name plus the *resolved* target layer.
#[test]
fn world_flag_parent_prefix_resolves_to_the_loader_layer_directly() {
    let mut fx = Fixture::new();
    fx.origin_layer = Some("inner.toml".to_string());
    // inner was loaded by outer; outer was loaded by the base world.
    fx.layers
        .insert("inner.toml".to_string(), layer(Some("outer.toml")));
    fx.layers.insert("outer.toml".to_string(), layer(None));
    let action = TriggerAction::SetWorldFlag {
        name: "parent:alarm".to_string(),
    };
    let out = dispatch_world_flag_action(&action, &fx.ctx());

    assert_eq!(
        out,
        DispatchResult {
            commands: vec![ActionCmd::MutateFlag {
                target_layer: Some("outer.toml".to_string()),
                name: "alarm".to_string(),
                mutation: FlagMutation::Set,
            }],
            new_events: vec![WorldEvent::FlagSet {
                name: "alarm".to_string(),
                origin_layer: Some("outer.toml".to_string()),
            }],
            ..Default::default()
        }
    );
}

#[test]
fn world_flag_walk_past_base_world_warns_and_emits_nothing_directly() {
    // Origin is already the base world, so any `parent:` overruns.
    let fx = Fixture::new();
    let action = TriggerAction::SetWorldFlag {
        name: "parent:alarm".to_string(),
    };
    let out = dispatch_world_flag_action(&action, &fx.ctx());

    assert_eq!(
        out,
        DispatchResult {
            warnings: vec![
                "'parent:alarm' from origin None walks past base world — ignoring".to_string()
            ],
            ..Default::default()
        }
    );
}

#[test]
fn world_flag_target_layer_missing_warns_and_emits_nothing_directly() {
    let mut fx = Fixture::new();
    // The trigger's own layer is not in the map, and there is no `parent:`
    // to walk, so the resolved target is a layer we cannot find.
    fx.origin_layer = Some("ghost.toml".to_string());
    let action = TriggerAction::SetWorldFlag {
        name: "alarm".to_string(),
    };
    let out = dispatch_world_flag_action(&action, &fx.ctx());

    assert_eq!(
        out,
        DispatchResult {
            warnings: vec![
                "target layer 'ghost.toml' missing from WorldLayerMap — ignoring 'alarm'"
                    .to_string()
            ],
            ..Default::default()
        }
    );
}

#[test]
fn world_flag_layer_missing_mid_walk_is_silent_and_treated_as_base_directly() {
    let mut fx = Fixture::new();
    // `ghost.toml` is absent from the map: the walk silently resolves its
    // loader_path to `None` (base) and carries on. This is deliberate —
    // only the *final* lookup warns.
    fx.origin_layer = Some("ghost.toml".to_string());
    let action = TriggerAction::SetWorldFlag {
        name: "parent:alarm".to_string(),
    };
    let out = dispatch_world_flag_action(&action, &fx.ctx());

    assert_eq!(
        out,
        DispatchResult {
            commands: vec![ActionCmd::MutateFlag {
                target_layer: None,
                name: "alarm".to_string(),
                mutation: FlagMutation::Set,
            }],
            new_events: vec![WorldEvent::FlagSet {
                name: "alarm".to_string(),
                origin_layer: None,
            }],
            ..Default::default()
        }
    );
}

/// Transition-edge case: setting an already-set flag still commands the
/// mutation but emits no event, because the boolean view did not flip.
/// The routed twin (`set_world_flag_on_already_set_flag_emits_no_transition_event`,
/// issue #708) pins why this depends on `base_flags` being the live store.
#[test]
fn world_flag_set_on_already_set_flag_emits_no_event_directly() {
    let mut fx = Fixture::new();
    fx.base_flags.set_flag_value("alarm", 1);
    let action = TriggerAction::SetWorldFlag {
        name: "alarm".to_string(),
    };
    let out = dispatch_world_flag_action(&action, &fx.ctx());

    assert_eq!(
        out,
        DispatchResult {
            commands: vec![ActionCmd::MutateFlag {
                target_layer: None,
                name: "alarm".to_string(),
                mutation: FlagMutation::Set,
            }],
            ..Default::default()
        }
    );
}

#[test]
fn world_flag_set_matches_dispatch_action_routing() {
    let mut fx = Fixture::new();
    fx.origin_layer = Some("sub.toml".to_string());
    fx.layers.insert("sub.toml".to_string(), layer(None));
    let action = TriggerAction::SetWorldFlag {
        name: "parent:alarm".to_string(),
    };

    // Both entry points must produce byte-identical results.
    assert_eq!(
        dispatch_action(&action, &fx.ctx()),
        dispatch_world_flag_action(&action, &fx.ctx())
    );
}

#[test]
#[should_panic(expected = "dispatch_world_flag_action called with non-world-flag action")]
fn world_flag_action_on_non_world_flag_variant_panics() {
    // The guard exists so a routing bug in `dispatch_action` fails loudly
    // rather than silently returning an empty result.
    let fx = Fixture::new();
    let action = TriggerAction::UnloadWorld {
        path: "worlds/sub.toml".to_string(),
    };
    let _ = dispatch_world_flag_action(&action, &fx.ctx());
}

// ── dispatch_destroy_entity (direct) ──────────────────────────────────
//
// The tests above drive the DestroyEntity arm through `dispatch_action`
// (`destroy_entity_emits_command_and_destroyed_event`,
// `destroy_entity_unknown_name_warns_and_emits_nothing`), proving the
// routing arm delegates. These call `dispatch_destroy_entity` directly,
// proving the extracted function is what produces the result and that
// the two entry points agree (issue #714).

#[test]
fn destroy_known_entity_emits_command_and_destroyed_event_directly() {
    let fx = Fixture::new().with_entity("wave_1", "uuid-wave-1");
    let action = TriggerAction::DestroyEntity {
        entity: "wave_1".to_string(),
    };
    let out = dispatch_destroy_entity(&action, &fx.ctx());

    assert_eq!(
        out,
        DispatchResult {
            commands: vec![ActionCmd::DestroyEntity {
                uuid: "uuid-wave-1".to_string()
            }],
            new_events: vec![WorldEvent::Destroyed {
                uuid: "uuid-wave-1".to_string()
            }],
            ..Default::default()
        }
    );
}

#[test]
fn destroy_unknown_entity_warns_and_emits_nothing_directly() {
    let fx = Fixture::new();
    let action = TriggerAction::DestroyEntity {
        entity: "ghost".to_string(),
    };
    let out = dispatch_destroy_entity(&action, &fx.ctx());

    assert_eq!(
        out,
        DispatchResult {
            warnings: vec!["DestroyEntity: unknown entity name 'ghost'".to_string()],
            ..Default::default()
        }
    );
}

#[test]
fn destroy_entity_matches_dispatch_action_routing() {
    let fx = Fixture::new().with_entity("wave_1", "uuid-wave-1");
    let action = TriggerAction::DestroyEntity {
        entity: "wave_1".to_string(),
    };

    // Both entry points must produce byte-identical results.
    assert_eq!(
        dispatch_action(&action, &fx.ctx()),
        dispatch_destroy_entity(&action, &fx.ctx())
    );
}

#[test]
#[should_panic(expected = "dispatch_destroy_entity called with non-destroy action")]
fn destroy_entity_on_non_destroy_variant_panics() {
    // The guard exists so a routing bug in `dispatch_action` fails loudly
    // rather than silently returning an empty result.
    let fx = Fixture::new();
    let action = TriggerAction::UnloadWorld {
        path: "worlds/sub.toml".to_string(),
    };
    let _ = dispatch_destroy_entity(&action, &fx.ctx());
}

// ── dispatch_spawn_entity (direct) ────────────────────────────────────
//
// The tests above drive the SpawnEntity arm through `dispatch_action`,
// proving the routing arm delegates. These call `dispatch_spawn_entity`
// directly, proving the extracted function is what produces the result
// and that the two entry points agree (issue #715). This is also where
// the behaviour that MOVED in #715 — template loading behind
// `DispatchContext::template_loader`, and the failed-spawn contingency
// gate — is pinned.

#[test]
fn spawn_template_loads_with_patched_name_and_inserts_directly() {
    let fx = Fixture::new().with_destroyer();
    let out = dispatch_spawn_entity(
        &spawn(None, Some([1.0, 2.0, 3.0]), vec!["wave", "hostiles"]),
        &fx.ctx(),
    );

    assert_eq!(
        out,
        DispatchResult {
            commands: vec![ActionCmd::SpawnEntity {
                config: patched_destroyer_template("wave_1"),
                name: "wave_1".to_string(),
                uuid: STUB_UUID.to_string(),
                position: [1.0, 2.0, 3.0],
                rotation: None,
                scale: None,
                layer_path: None,
                template_path: DESTROYER_TEMPLATE.to_string(),
                overrides: None,
            }],
            name_to_uuid_inserts: vec![("wave_1".to_string(), STUB_UUID.to_string())],
            entity_group_inserts: vec![
                ("wave".to_string(), "wave_1".to_string()),
                ("hostiles".to_string(), "wave_1".to_string()),
            ],
            ..Default::default()
        }
    );
}

/// The shipped test for the contingency gate (issue #715): a template
/// that fails to resolve produces a warning-only result — no command, no
/// name → uuid insert, no group insert. Before #715 this gate lived in
/// the applier (`spawn_failed`), where a #710 review flagged it had only
/// throwaway coverage.
#[test]
fn spawn_template_not_found_warns_and_emits_nothing_directly() {
    // No `.with_destroyer()`: the loader has no templates at all.
    let fx = Fixture::new();
    let out = dispatch_spawn_entity(&spawn(None, Some([1.0, 2.0, 3.0]), vec!["wave"]), &fx.ctx());

    assert_eq!(
        out,
        DispatchResult {
            warnings: vec![
                "SpawnEntity 'wave_1' template 'assets/entities/destroyer.toml' not found"
                    .to_string()
            ],
            ..Default::default()
        }
    );
}

/// **A `_remove` tombstone in a `spawn_entity` override WARNS** (issue
/// #911), and the spawn still happens on the unmodified template.
///
/// This is the other instance-layer entry point — `entity_loader::
/// apply_overrides` is the first — and it is the one that cannot fail the
/// load, so the warning is the only signal an author gets. It must exist:
/// a tombstone is subtractive, the author asked for something to be GONE,
/// and before #911's fix it was accepted in silence (`DoctrineObjective`
/// is not `deny_unknown_fields`, so the marker vanished into serde and the
/// doctrine survived). Nothing exercised this path's override arm at all
/// before this test.
#[test]
fn spawn_entity_override_carrying_a_tombstone_warns_and_keeps_the_template() {
    let fx = Fixture::new().with_destroyer();
    let action = TriggerAction::SpawnEntity {
        template_path: DESTROYER_TEMPLATE.to_string(),
        name: "wave_1".to_string(),
        anchor: None,
        position: Some([0.0, 0.0, 0.0]),
        rotation: None,
        scale: None,
        groups: vec![],
        overrides: Some(
            toml::from_str("[[behaviour.doctrine]]\nid = \"destroy-hostiles\"\n_remove = true\n")
                .unwrap(),
        ),
    };
    let out = dispatch_spawn_entity(&action, &fx.ctx());

    assert_eq!(
        out.warnings.len(),
        1,
        "the tombstone must be reported, got {:?}",
        out.warnings
    );
    assert!(
        out.warnings[0].contains(crate::entities::entity_override::REMOVE_KEY),
        "the warning must name the marker so the author can find it, got {:?}",
        out.warnings[0]
    );
    // The spawn still happens, on the template as authored — a partial
    // spawn is better than none, exactly as for a failed reparse.
    assert_eq!(out.commands.len(), 1, "the template still spawns");
}

/// The control: an override WITHOUT a tombstone still applies here. Pinned
/// alongside the test above so "warns" cannot be achieved by rejecting
/// every override.
#[test]
fn spawn_entity_override_without_a_tombstone_still_applies() {
    let fx = Fixture::new().with_destroyer();
    let action = TriggerAction::SpawnEntity {
        template_path: DESTROYER_TEMPLATE.to_string(),
        name: "wave_1".to_string(),
        anchor: None,
        position: Some([0.0, 0.0, 0.0]),
        rotation: None,
        scale: None,
        groups: vec![],
        overrides: Some(toml::from_str(r#"tags = ["npc", "enemy"]"#).unwrap()),
    };
    let out = dispatch_spawn_entity(&action, &fx.ctx());

    assert!(out.warnings.is_empty(), "got {:?}", out.warnings);
    let ActionCmd::SpawnEntity { config, .. } = &out.commands[0] else {
        panic!("expected a SpawnEntity command, got {:?}", out.commands[0])
    };
    assert_eq!(
        config.tags,
        vec!["npc".to_string(), "enemy".to_string()],
        "an instance override REPLACES tags — the pre-#911 rule, unchanged"
    );
}

/// A failed spawn must not consume a uuid: the source is drawn only
/// after the template resolves (template before uuid — the pre-#710
/// inline ordering), so failures leave the uuid sequence untouched.
#[test]
fn spawn_failed_template_load_does_not_consume_a_uuid_directly() {
    let fx = Fixture::new().with_destroyer();
    let counter = std::cell::Cell::new(0u32);
    let source = || {
        counter.set(counter.get() + 1);
        format!("uuid-{}", counter.get())
    };
    let ctx = DispatchContext {
        uuid_source: &source,
        ..fx.ctx()
    };

    // A template the loader does not know: warns, draws nothing.
    let missing = TriggerAction::SpawnEntity {
        template_path: "assets/entities/missing.toml".to_string(),
        name: "wave_1".to_string(),
        anchor: None,
        position: Some([0.0, 0.0, 0.0]),
        rotation: None,
        scale: None,
        groups: vec![],
        overrides: None,
    };
    let out = dispatch_spawn_entity(&missing, &ctx);
    assert_eq!(out.warnings.len(), 1);
    assert_eq!(counter.get(), 0, "a failed spawn must not consume a uuid");

    // The next successful spawn draws the FIRST uuid, not the second.
    let out = dispatch_spawn_entity(&spawn(None, Some([0.0, 0.0, 0.0]), vec![]), &ctx);
    assert_eq!(
        out.name_to_uuid_inserts,
        vec![("wave_1".to_string(), "uuid-1".to_string())]
    );
}

#[test]
fn spawn_anchor_resolves_from_the_origin_layer_directly() {
    let mut fx = Fixture::new().with_destroyer();
    fx.origin_layer = Some("sub.toml".to_string());
    let mut sub = layer(None);
    sub.anchors.insert("staging".to_string(), [4.0, 5.0, 6.0]);
    fx.layers.insert("sub.toml".to_string(), sub);
    // A same-named base anchor must NOT win for a layer-authored trigger.
    fx.base_anchors
        .insert("staging".to_string(), [99.0, 99.0, 99.0]);

    let out = dispatch_spawn_entity(&spawn(Some("staging"), None, vec![]), &fx.ctx());

    let ActionCmd::SpawnEntity {
        position,
        layer_path,
        ..
    } = &out.commands[0]
    else {
        panic!("expected SpawnEntity");
    };
    assert_eq!(position, &[4.0, 5.0, 6.0]);
    // Origin-layer tracking: the command records the authoring layer so
    // the applier attaches the spawn to it for cascade unload.
    assert_eq!(layer_path, &Some("sub.toml".to_string()));
}

/// Layer-originated triggers look ONLY in their own layer's anchor
/// table: a same-named base-world anchor must not rescue a layer trigger
/// whose own layer lacks the anchor.
#[test]
fn spawn_origin_layer_anchor_missing_warns_despite_base_anchor_directly() {
    let mut fx = Fixture::new().with_destroyer();
    fx.origin_layer = Some("sub.toml".to_string());
    fx.layers.insert("sub.toml".to_string(), layer(None)); // no anchors
    fx.base_anchors
        .insert("staging".to_string(), [99.0, 99.0, 99.0]);

    let out = dispatch_spawn_entity(&spawn(Some("staging"), None, vec![]), &fx.ctx());

    assert_eq!(
        out,
        DispatchResult {
            warnings: vec!["SpawnEntity 'wave_1' anchor 'staging' not found".to_string()],
            ..Default::default()
        }
    );
}

#[test]
fn spawn_anchor_resolves_from_the_base_world_directly() {
    let mut fx = Fixture::new().with_destroyer();
    fx.base_anchors
        .insert("staging".to_string(), [10.0, 0.0, -5.0]);

    let out = dispatch_spawn_entity(&spawn(Some("staging"), None, vec![]), &fx.ctx());

    let ActionCmd::SpawnEntity {
        position,
        layer_path,
        ..
    } = &out.commands[0]
    else {
        panic!("expected SpawnEntity");
    };
    assert_eq!(position, &[10.0, 0.0, -5.0]);
    // Base-world origin: no layer to attach the spawn to.
    assert_eq!(layer_path, &None);
}

#[test]
fn spawn_rotation_and_scale_pass_through_directly() {
    let fx = Fixture::new().with_destroyer();
    let action = TriggerAction::SpawnEntity {
        template_path: DESTROYER_TEMPLATE.to_string(),
        name: "wave_1".to_string(),
        anchor: None,
        position: Some([0.0, 0.0, 0.0]),
        rotation: Some([0.0, 1.57, 0.0]),
        scale: Some([2.0, 2.0, 2.0]),
        groups: vec![],
        overrides: None,
    };
    let out = dispatch_spawn_entity(&action, &fx.ctx());

    let ActionCmd::SpawnEntity {
        rotation, scale, ..
    } = &out.commands[0]
    else {
        panic!("expected SpawnEntity");
    };
    assert_eq!(rotation, &Some([0.0, 1.57, 0.0]));
    assert_eq!(scale, &Some([2.0, 2.0, 2.0]));
}

#[test]
fn spawn_registers_every_group_directly() {
    let fx = Fixture::new().with_destroyer();
    let out = dispatch_spawn_entity(
        &spawn(None, Some([0.0, 0.0, 0.0]), vec!["wave", "hostiles"]),
        &fx.ctx(),
    );

    assert_eq!(
        out.entity_group_inserts,
        vec![
            ("wave".to_string(), "wave_1".to_string()),
            ("hostiles".to_string(), "wave_1".to_string()),
        ]
    );
}

/// An empty trigger `name` leaves the template's display name in place —
/// the patch is conditional on `!name.is_empty()`.
#[test]
fn spawn_empty_name_keeps_the_template_display_name_directly() {
    let fx = Fixture::new().with_destroyer();
    let action = TriggerAction::SpawnEntity {
        template_path: DESTROYER_TEMPLATE.to_string(),
        name: String::new(),
        anchor: None,
        position: Some([0.0, 0.0, 0.0]),
        rotation: None,
        scale: None,
        groups: vec![],
        overrides: None,
    };
    let out = dispatch_spawn_entity(&action, &fx.ctx());

    let ActionCmd::SpawnEntity { config, .. } = &out.commands[0] else {
        panic!("expected SpawnEntity");
    };
    assert_eq!(config.name, Some("Harrow Destroyer".to_string()));
}

#[test]
fn spawn_entity_matches_dispatch_action_routing() {
    let mut fx = Fixture::new().with_destroyer();
    fx.origin_layer = Some("sub.toml".to_string());
    let mut sub = layer(None);
    sub.anchors.insert("staging".to_string(), [4.0, 5.0, 6.0]);
    fx.layers.insert("sub.toml".to_string(), sub);
    let action = spawn(Some("staging"), None, vec!["wave"]);

    // Both entry points must produce byte-identical results.
    assert_eq!(
        dispatch_action(&action, &fx.ctx()),
        dispatch_spawn_entity(&action, &fx.ctx())
    );
}

#[test]
#[should_panic(expected = "dispatch_spawn_entity called with non-spawn action")]
fn spawn_entity_on_non_spawn_variant_panics() {
    // The guard exists so a routing bug in `dispatch_action` fails loudly
    // rather than silently returning an empty result.
    let fx = Fixture::new();
    let action = TriggerAction::UnloadWorld {
        path: "worlds/sub.toml".to_string(),
    };
    let _ = dispatch_spawn_entity(&action, &fx.ctx());
}
