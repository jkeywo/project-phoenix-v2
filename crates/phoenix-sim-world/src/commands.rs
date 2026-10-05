use crate::core::messages::{AiDirective, FlagKind, GamePhase, ModifierSlot, ObjectiveSource};
use crate::modifiers::IntModifierSlot;
use crate::objectives::UtilityConfig;
use crate::world::config::TriggerAction;
use uuid::Uuid;
/// A flag mutation to apply to a resolved `FlagStore`.
///
/// Mirrors `FlagStore`'s mutators one-for-one.
#[derive(Clone, Debug, PartialEq)]
pub enum FlagMutation {
    /// `set_flag` — counter := 1.
    Set,
    /// `clear_flag` — counter := 0.
    Clear,
    /// `increment_flag` — counter := counter.saturating_add(by).
    Increment(i64),
    /// `set_flag_value` — counter := value.
    SetValue(i64),
}

/// A single side effect for the Bevy applier to perform.
///
/// Entity targets are UUID strings; faction targets are already-resolved
/// `Uuid`s. Nothing here names a Bevy `Entity`.
#[derive(Clone, Debug, PartialEq)]
pub enum ActionCmd<SpawnConfig> {
    Addressed {
        recipients: crate::recipients::RecipientSelection,
        action: Box<TriggerAction>,
        origin_layer: Option<String>,
    },
    Presentation {
        ship: String,
        cue: crate::gm_presentation::PresentationCue,
    },
    SetContactInformation {
        observer: String,
        change: crate::gm_information::ContactInformationChange,
    },
    SetNpcDoctrine {
        uuid: String,
        id: String,
    },
    /// Add an objective with `targets` already resolved (explicit targets, or
    /// the trigger entity as fallback).
    AddObjective {
        id: String,
        text: String,
        /// Runtime values interpolated into `text`'s `{placeholder}` tokens by
        /// the client. See `messages::TEXT_PARAMS_SUFFIX`.
        text_params: std::collections::BTreeMap<String, String>,
        mandatory: bool,
        targets: Vec<String>,
        directive: AiDirective,
        utility: UtilityConfig,
        source: ObjectiveSource,
        /// An objective-specific Command stance contributed to a named target
        /// Station while this objective is active (issue #1110). `None` for
        /// objectives that contribute no stance.
        command_stance: Option<(
            crate::core::messages::StationId,
            crate::ship::config::StationStanceConfig,
        )>,
        /// Sub-world layer that authored the trigger adding this objective, or
        /// `None` for base-world triggers (issue #751). The applier records
        /// layer-owned objective ids so `UnloadWorld` removes them.
        origin_layer: Option<String>,
    },
    AddObjectiveInstance {
        spec: crate::objective_instances::ObjectiveInstanceSpec,
        text: String,
        text_params: std::collections::BTreeMap<String, String>,
        mandatory: bool,
        targets: Vec<String>,
        directive: AiDirective,
        utility: UtilityConfig,
        source: ObjectiveSource,
        command_stance: Option<(
            crate::core::messages::StationId,
            crate::ship::config::StationStanceConfig,
        )>,
        origin_layer: Option<String>,
    },
    /// Record an authored story beat on the mission timeline (issue #1338).
    ///
    /// `id` is the author's own semantic identifier — the scenario's
    /// punctuation, with no other meaning to the simulation. Nothing branches
    /// on it and no state moves: the applier buffers it onto the narrative
    /// effect queue and `narrative::emit_authored_and_marked_entity_narrative`
    /// turns it into a `NarrativeKind::BeatFired` event. This is the "authors
    /// explicitly mark beats" half of PRD #1337 — the sim never infers one.
    NarrativeBeat {
        id: String,
    },
    /// Record an authored outcome for a marked narrative entity (issue #1338).
    ///
    /// `entity` is the world's authored entity NAME, not a UUID, resolved by
    /// the applier against `WorldContentRuntime::name_to_uuid` for the reason
    /// every other name-carrying command here is. `outcome` is already
    /// validated at the script boundary
    /// ([`crate::core::narrative::NarrativeKind::parse_outcome`]), so a typo
    /// raises there rather than reaching this queue as a silently different
    /// beat.
    ///
    /// Spawn and death are emitted automatically for any entity carrying a
    /// [`crate::core::narrative::NarrativeMark`] — death on BOTH removal paths,
    /// the combat kill and the scripted
    /// [`ActionCmd::DestroyEntity`]. This is the door for the outcomes only an
    /// author can judge — escaped, rescued, abandoned, disabled.
    ///
    /// It is also how an author says a scripted removal was NOT a death.
    /// Because a marked hull `ctx.effects.destroy_entity(..)` removes reports
    /// `marked_entity_destroyed` by default, an author who despawns one to
    /// mean something else records that outcome **before, or on the same tick
    /// as, the destroy**; the automatic death is then suppressed and the
    /// authored outcome stands alone. Recording it a tick LATER is too late —
    /// the death has already been written. See
    /// [`crate::core::narrative::NarrativeKind::MarkedEntityDestroyed`].
    NarrativeOutcome {
        entity: String,
        outcome: crate::core::narrative::NarrativeKind,
    },
    /// Show a timed ship's-computer message on the Viewscreen (issue #1342).
    ///
    /// Fully resolved and validated at the script boundary before it ever
    /// reaches here: `severity` is already parsed
    /// ([`crate::core::computer_message::ComputerMessageSeverity::parse`])
    /// and `duration_secs` already checked positive
    /// (`world::script::effects::register_effects`'s `show_message` host fn).
    /// The applier does no name resolution at all — `station` is an authored
    /// Station id, not an entity name — so it only buffers this onto the
    /// computer-message effect queue for
    /// `crate::narrative::tick_computer_message` to apply.
    ShowComputerMessage {
        id: String,
        text: String,
        severity: crate::core::computer_message::ComputerMessageSeverity,
        duration_secs: i64,
        station: Option<crate::core::messages::StationId>,
    },
    /// Mark an objective complete. A no-op for unknown / non-Active ids.
    CompleteObjective {
        id: String,
    },
    /// Re-arm the trigger(s) with the given authored id (issue #751).
    ResetTrigger {
        id: String,
    },
    /// Mark an objective failed. A no-op for unknown / non-Active ids.
    FailObjective {
        id: String,
    },
    CompleteObjectiveInstance {
        key: crate::objective_instances::ObjectiveInstanceKey,
    },
    FailObjectiveInstance {
        key: crate::objective_instances::ObjectiveInstanceKey,
    },
    SetObjectiveInstanceProgress {
        key: crate::objective_instances::ObjectiveInstanceKey,
        progress: f32,
    },
    /// Move the named entity's infrastructure condition by `delta` points —
    /// negative degrades, positive repairs (issue #1025).
    ///
    /// `entity` is the world's authored entity NAME, not a UUID: the applier
    /// resolves it against `WorldContentRuntime::name_to_uuid` and *queues* the
    /// delta rather than applying it on the spot, so every condition move lands
    /// in the one system that owns operational-flag edges. A timed field-repair
    /// operation applies one small slice of this per tick.
    AdjustInfrastructureCondition {
        entity: String,
        delta: f32,
    },
    /// Move one of the named entity's published `[[infrastructure.capacity]]`
    /// levels by `delta` units — negative spends, positive returns
    /// (issue #1042).
    ///
    /// The capacity sibling of [`Self::AdjustInfrastructureCondition`], and it
    /// resolves and queues on identical terms: `entity` is the authored NAME,
    /// the applier looks it up in `WorldContentRuntime::name_to_uuid`, and
    /// `tick_infrastructure_condition` — the one system that re-publishes a
    /// structure's numbers onto the counters a script predicate reads — does the
    /// arithmetic. A `transfer` completing queues the same
    /// [`CapacityAdjustment`](crate::infrastructure::CapacityAdjustment); this
    /// is a scenario's door to the same queue.
    ///
    /// A DELTA rather than a set, matching every other move in this vocabulary
    /// (condition points, a transfer's cargo) and for the same reason: the sign
    /// convention lives at the call site, where the author can see what they
    /// meant. A scenario publishing a *computed* number writes
    /// `want - <the live counter>` and lands on the value it worked out.
    AdjustInfrastructureCapacity {
        entity: String,
        capacity: String,
        delta: i64,
    },
    /// Call out, settle, or re-price one side of a labour dispute
    /// (issue #1035).
    ///
    /// `id` is the world's authored `[[workforce]]` id, not an entity name and
    /// not a UUID: a workforce is a *party*, exactly as a commitment's
    /// `made_to` is, and the people who run a skyway are not any one hull. So
    /// there is nothing for the applier to resolve — it applies the mutation to
    /// [`WorkforceRegister`](crate::world::workforce::WorkforceRegister)
    /// directly, and a mutation naming a side this world never declared is a
    /// logged no-op rather than a load error, for the reason the register's
    /// own lookup returns "at work" for an unknown id.
    ///
    /// The mirror flag is **not** written here. The host fn that emits this
    /// pushes an ordinary [`ActionCmd::MutateFlag`] beside it, so the flag a
    /// script reads back gets its `FlagSet`/`FlagCleared` transition from the
    /// one path that emits them and an `on_flag_cleared` trigger chains off a
    /// settlement without this command knowing triggers exist.
    SetWorkforceState {
        id: String,
        mutation: crate::world::workforce::WorkforceMutation,
    },
    /// Command one of the named ship's POWER groups to a level (issue #1398).
    ///
    /// The state behind `hold_fire(name)` / `release_fire(name)` since #1398.
    /// The verbs kept their names and lost their own state: restraint is a
    /// reactor order now, so a scenario silences a hull through the same
    /// `PowerSystem` an Engineering officer commands and the fire gate has one
    /// thing to read instead of two.
    ///
    /// `entity` is the world's authored entity NAME, resolved by the applier
    /// against `WorldContentRuntime::name_to_uuid` for the reason every other
    /// name-carrying command here is, and queued rather than applied on the
    /// spot because the applier holds that map and no entity query at all.
    ///
    /// The mirror flag is **not** written here. `weapons_cold.*` is mirrored off
    /// the ship's own reactor every tick by
    /// `crate::ship::power::mirror_weapons_cold_flags`, so a scenario's order
    /// and an Engineering officer's order produce the same
    /// `FlagSet`/`FlagCleared` transition. A flag written here would have been
    /// written for the scenario's orders and silently absent for the crew's.
    SetGroupPower {
        entity: String,
        group: crate::core::messages::PowerGroupId,
        level: crate::modifiers::power_system::ScriptedPowerLevel,
    },
    /// Order the named civilian to hold, divert or dock (issue #1028).
    ///
    /// `entity` is the world's authored entity NAME, not a UUID, and the applier
    /// resolves it against `WorldContentRuntime::name_to_uuid` before *queueing*
    /// the order — it is not applied on the spot, so a scripted order goes
    /// through the same compliance state machine, the same acknowledgement
    /// delay and the same authored disposition a crew's order does. A scenario
    /// cannot remote-control traffic that a crew has to negotiate with.
    OrderCivilian {
        entity: String,
        order: crate::civilian::CivilianOrder,
    },
    /// Write one finding onto a subject's dossier (issue #1031).
    ///
    /// `subject` is the world's authored entity NAME, resolved by the applier
    /// against `WorldContentRuntime::name_to_uuid` for the reason every other
    /// name-carrying command here is — that map is the applier's, not the script
    /// boundary's — and a name no entity answers to is a warned no-op there.
    ///
    /// `gathered_at_tick` is stamped at the SCRIPT surface rather than filled in
    /// by the applier: it is the tick the handler ran on, which is what "when
    /// the crew learned it" means, and the applier drains on whatever tick it
    /// drains on.
    RecordDossierEvidence {
        subject: String,
        text: String,
        provenance: crate::dossier::evidence::EvidenceProvenance,
        gathered_at_tick: u64,
    },
    /// Add or update a float modifier on the entity with `uuid`.
    ApplyModifier {
        uuid: String,
        tag: String,
        slot: ModifierSlot,
        bonus: f32,
    },
    /// Remove a float modifier from the entity with `uuid`.
    RemoveModifier {
        uuid: String,
        tag: String,
        slot: ModifierSlot,
    },
    /// Add a boolean flag modifier to the entity with `uuid`.
    ApplyFlag {
        uuid: String,
        tag: String,
        kind: FlagKind,
    },
    /// Remove a boolean flag modifier from the entity with `uuid`.
    RemoveFlag {
        uuid: String,
        tag: String,
        kind: FlagKind,
    },
    /// Add or update an integer modifier on the entity with `uuid`.
    ApplyIntModifier {
        uuid: String,
        tag: String,
        slot: IntModifierSlot,
        bonus: i32,
    },
    /// Remove an integer modifier from the entity with `uuid`.
    RemoveIntModifier {
        uuid: String,
        tag: String,
        slot: IntModifierSlot,
    },
    /// Write the game-over reason resource — reason string plus the declared
    /// [`Outcome`](crate::core::balance::Outcome) (#843, `None` for an undeclared
    /// scripted end).
    ///
    /// Always emitted *before* `SetNextState` — `OnEnter(GamePhase::GameOver)`
    /// reads the reason, so the ordering is load-bearing.
    SetGameOverReason {
        reason: String,
        outcome: Option<crate::core::balance::Outcome>,
    },
    /// Write one row of the structured post-mission report (issue #1344).
    ///
    /// Buffered onto `EffectQueue<ReportRow>` by the applier and drained by
    /// [`crate::mission_report::apply_report_rows`], for the reason every #1223
    /// effect is: the applier holds no resources and no message writers.
    ///
    /// Writing the same row `id` again UPDATES it in place, so a scenario can
    /// re-state a row as the mission moves without the report growing; a row
    /// never written is simply absent from the report, which is how a
    /// genuinely inapplicable row is omitted rather than invented.
    ///
    /// The state word is already validated at the script boundary
    /// ([`crate::core::report::ReportRowState::parse`]), so a typo raises there
    /// rather than reaching this queue as a row nothing can style.
    SetReportRow(crate::core::report::ReportRow),
    /// Queue a game-phase transition.
    SetNextState {
        phase: GamePhase,
    },
    /// Additively load a sub-world. `loader_path` is the layer that issued the
    /// action, recorded so `parent:` from the new layer resolves up to it.
    LoadWorld {
        path: String,
        loader_path: Option<String>,
    },
    /// Unload a previously loaded sub-world.
    UnloadWorld {
        path: String,
    },
    /// Apply `mutation` to `name` in `target_layer`'s store (`None` = base
    /// world). `name` is already stripped of `parent:` prefixes and
    /// `target_layer` is the walk's resolved destination.
    MutateFlag {
        target_layer: Option<String>,
        name: String,
        mutation: FlagMutation,
    },
    /// Spawn an entity. `config` is the template already resolved via
    /// `DispatchContext::template_loader`, with the trigger's `name` patched
    /// in — the applier loads nothing. `position` is resolved (anchor lookups
    /// already done); `uuid` came from `DispatchContext::uuid_source`. When
    /// `layer_path` is `Some`, the applier records the spawned entity on that
    /// layer so `UnloadWorld` despawns it.
    ///
    /// A template that fails to resolve never reaches here: the dispatch arm
    /// returns a warning-only result instead — see `dispatch_spawn_entity`.
    ///
    /// `config` is boxed because `EntityConfig` dwarfs every other variant
    /// (clippy: `large_enum_variant`) and commands travel in `Vec<ActionCmd>`.
    SpawnEntity {
        config: Box<SpawnConfig>,
        name: String,
        uuid: String,
        position: [f32; 3],
        rotation: Option<[f32; 3]>,
        scale: Option<[f32; 3]>,
        layer_path: Option<String>,
        /// The template path `config` was resolved from, carried alongside the
        /// resolved config rather than instead of it (issue #863).
        ///
        /// The applier stamps it onto the spawned entity as part of its
        /// [`crate::world::spawn_origin::SpawnOrigin`], which is what lets a
        /// resume rebuild a mid-run spawn no fresh boot re-derives. `config` is
        /// still the thing that gets spawned — nothing re-loads the template
        /// here — so the two cannot disagree about *this* spawn; the path is
        /// what a *later* rebuild resolves.
        template_path: String,
        /// Optional inline TOML overrides already applied to `config` by the
        /// dispatch function; preserved here for auditing / test assertions —
        /// and, since issue #863, so the spawn's origin record can carry the
        /// same document a rebuild has to merge again.
        overrides: Option<toml::Value>,
    },
    /// Destroy the entity with `uuid` and run the destruction cascade.
    DestroyEntity {
        uuid: String,
    },
    /// Add `enemy_uuid` to `faction_uuid`'s enemies.
    ///
    /// Deliberately does *not* re-validate AI targets: adding a hostility
    /// cannot invalidate an existing engagement, and the next `enemy_in_range`
    /// tick picks the new relationship up organically.
    AddFactionEnemy {
        faction_uuid: Uuid,
        enemy_uuid: Uuid,
    },
    /// Remove `enemy_uuid` from `faction_uuid`'s enemies. **Only if the removal
    /// actually changed the registry** (`remove_enemy` returned true),
    /// re-validate every AI controller's target so an in-progress engagement
    /// does not stick on a now-friendly entity. Removing an absent hostility
    /// changes nothing, so it must not trigger the revalidation sweep.
    RemoveFactionEnemy {
        faction_uuid: Uuid,
        enemy_uuid: Uuid,
    },
}

/// Promote a script command at its existing ordered application slot.
/// Scripts cannot hold a resolved Gameplay template; spawn stays an authored action.
impl ActionCmd<std::convert::Infallible> {
    pub fn into_resolved<T>(self) -> ActionCmd<T> {
        match self {
            Self::Addressed {
                recipients,
                action,
                origin_layer,
            } => ActionCmd::Addressed {
                recipients,
                action,
                origin_layer,
            },
            Self::Presentation { ship, cue } => ActionCmd::Presentation { ship, cue },
            Self::SetContactInformation { observer, change } => {
                ActionCmd::SetContactInformation { observer, change }
            }
            Self::SetNpcDoctrine { uuid, id } => ActionCmd::SetNpcDoctrine { uuid, id },
            Self::AddObjective {
                id,
                text,
                text_params,
                mandatory,
                targets,
                directive,
                utility,
                source,
                command_stance,
                origin_layer,
            } => ActionCmd::AddObjective {
                id,
                text,
                text_params,
                mandatory,
                targets,
                directive,
                utility,
                source,
                command_stance,
                origin_layer,
            },
            Self::AddObjectiveInstance {
                spec,
                text,
                text_params,
                mandatory,
                targets,
                directive,
                utility,
                source,
                command_stance,
                origin_layer,
            } => ActionCmd::AddObjectiveInstance {
                spec,
                text,
                text_params,
                mandatory,
                targets,
                directive,
                utility,
                source,
                command_stance,
                origin_layer,
            },
            Self::NarrativeBeat { id } => ActionCmd::NarrativeBeat { id },
            Self::NarrativeOutcome { entity, outcome } => {
                ActionCmd::NarrativeOutcome { entity, outcome }
            }
            Self::ShowComputerMessage {
                id,
                text,
                severity,
                duration_secs,
                station,
            } => ActionCmd::ShowComputerMessage {
                id,
                text,
                severity,
                duration_secs,
                station,
            },
            Self::CompleteObjective { id } => ActionCmd::CompleteObjective { id },
            Self::ResetTrigger { id } => ActionCmd::ResetTrigger { id },
            Self::FailObjective { id } => ActionCmd::FailObjective { id },
            Self::CompleteObjectiveInstance { key } => ActionCmd::CompleteObjectiveInstance { key },
            Self::FailObjectiveInstance { key } => ActionCmd::FailObjectiveInstance { key },
            Self::SetObjectiveInstanceProgress { key, progress } => {
                ActionCmd::SetObjectiveInstanceProgress { key, progress }
            }
            Self::AdjustInfrastructureCondition { entity, delta } => {
                ActionCmd::AdjustInfrastructureCondition { entity, delta }
            }
            Self::AdjustInfrastructureCapacity {
                entity,
                capacity,
                delta,
            } => ActionCmd::AdjustInfrastructureCapacity {
                entity,
                capacity,
                delta,
            },
            Self::SetWorkforceState { id, mutation } => {
                ActionCmd::SetWorkforceState { id, mutation }
            }
            Self::SetGroupPower {
                entity,
                group,
                level,
            } => ActionCmd::SetGroupPower {
                entity,
                group,
                level,
            },
            Self::OrderCivilian { entity, order } => ActionCmd::OrderCivilian { entity, order },
            Self::RecordDossierEvidence {
                subject,
                text,
                provenance,
                gathered_at_tick,
            } => ActionCmd::RecordDossierEvidence {
                subject,
                text,
                provenance,
                gathered_at_tick,
            },
            Self::ApplyModifier {
                uuid,
                tag,
                slot,
                bonus,
            } => ActionCmd::ApplyModifier {
                uuid,
                tag,
                slot,
                bonus,
            },
            Self::RemoveModifier { uuid, tag, slot } => {
                ActionCmd::RemoveModifier { uuid, tag, slot }
            }
            Self::ApplyFlag { uuid, tag, kind } => ActionCmd::ApplyFlag { uuid, tag, kind },
            Self::RemoveFlag { uuid, tag, kind } => ActionCmd::RemoveFlag { uuid, tag, kind },
            Self::ApplyIntModifier {
                uuid,
                tag,
                slot,
                bonus,
            } => ActionCmd::ApplyIntModifier {
                uuid,
                tag,
                slot,
                bonus,
            },
            Self::RemoveIntModifier { uuid, tag, slot } => {
                ActionCmd::RemoveIntModifier { uuid, tag, slot }
            }
            Self::SetGameOverReason { reason, outcome } => {
                ActionCmd::SetGameOverReason { reason, outcome }
            }
            Self::SetNextState { phase } => ActionCmd::SetNextState { phase },
            Self::LoadWorld { path, loader_path } => ActionCmd::LoadWorld { path, loader_path },
            Self::UnloadWorld { path } => ActionCmd::UnloadWorld { path },
            Self::MutateFlag {
                target_layer,
                name,
                mutation,
            } => ActionCmd::MutateFlag {
                target_layer,
                name,
                mutation,
            },
            Self::SpawnEntity { config, .. } => match *config {},
            Self::DestroyEntity { uuid } => ActionCmd::DestroyEntity { uuid },
            Self::AddFactionEnemy {
                faction_uuid,
                enemy_uuid,
            } => ActionCmd::AddFactionEnemy {
                faction_uuid,
                enemy_uuid,
            },
            Self::RemoveFactionEnemy {
                faction_uuid,
                enemy_uuid,
            } => ActionCmd::RemoveFactionEnemy {
                faction_uuid,
                enemy_uuid,
            },
            Self::SetReportRow(value) => ActionCmd::SetReportRow(value),
        }
    }
}
