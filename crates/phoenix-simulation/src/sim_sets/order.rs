//! Explicit #1400 order of conflicting owners within each fixed phase.
//! Existing phase, run-condition and deferred-application boundaries remain
//! at their original registrations. These sets order execution only.
use super::FixedStep;
use bevy::prelude::*;

mod pass;
use pass::DeclaredOrder;

pub(crate) fn configure_fixed_order(app: &mut App) {
    use FixedStep::*;
    let mut order = DeclaredOrder::default();
    // LobbySystemSet: reviewed conflicting owner sequence.
    sequence(
        &mut order,
        &[
            HandleSetNameSystem,
            HandleReportStationEligibilitySystem,
            HandleSetStationRatingSystem,
            HandleSetAfkSystem,
            HandleSetSpectatorSystem,
            HandleSetReadySystem,
            HandleReleaseStationSystem,
            HandleSelectStationSystem,
            HandleIdentifySystem,
        ],
    );
    // Input: reviewed conflicting owner sequence.
    sequence(
        &mut order,
        &[
            TickComponentEffects,
            ReadSystemText,
            OperateIlluminationAi,
            HandleIlluminationCommands,
            AiTorpedoLoad,
            TickFrequencyHintHighFidelity,
            OperateCaptainAi,
            BackfillCaptainPrefersCinematicView,
            HandleSetRedAlert,
            HandleSetView,
            HandleSetObjectivePriority,
            ProjectActiveObjectiveStances,
            ReconcileStationStances,
            OperateCommandAi,
            HandleSetStationStance,
            ApplyAlertChangeToStances,
            ReconcileDirectedTargetControl,
            HandleStationRatingChange,
            HandleCoordinationEnqueue,
            AiPhaserAutoFire,
            TickWeaponsArcRequest,
            HandleSetPhaserMode,
            AiStrikeBoost,
            HandleSetStrikeBoost,
            HandleSetPhaserFrequency,
            HandleSetTorpedoVolleyTarget,
            // A GM withdrawal of a hostility must drop stranded tactical locks
            // before the retention tier gets to keep one (issue #1442).
            RevalidateGmFactionLocks,
            AiTargetSelection,
            HandleSetTarget,
            TickNpcAutoMatchFrequency,
            ApplyTacticalFrequencyHint,
            TickBlasterAutoFire,
            // Settle human and ordinary AI intent together before Physics.
            // AI-emitted fallback assignments must not be cleared next tick
            // before their consumer sees them; explicit orders still win.
            HandleWeaponAssignments,
            OperateExternalRepairAi,
            TrackShieldDamageHistory,
            EmitShieldsCoordination,
            OperateSensorsAi,
            OperateFocusedSensorsAi,
            TickSensorsFrequencyHint,
            TickSensorsThreatWarning,
            OperateCivilianOrderAi,
            OperateCommsAi,
            ApplyHostLossBackfill,
            ResolveHumanSeekingHosts,
            EnforceStationPuppetControl,
            ApplyGodModeToggle,
            ApplyAssignedStationRating,
            RetireDialoguesForPendingLayerUnloads,
            OperateIdentityEmitterAi,
            HandleIdentityEmitterCommands,
            TickIdentityEmitters,
            OperateConcealmentAi,
            HandleConcealmentCommands,
            OperateSignalProtectionAi,
            HandleSignalProtectionCommands,
            TickSignalProtection,
            OperateJammingAi,
            HandleJammingCommands,
            TickJamming,
            RefreshJammedCommsRange,
            HandleHail,
            HandleCommsRead,
            TickCommsRequestReading,
            OperateCommsResponseAi,
            HandleRespondToMessage,
            HandleCommsMediaCall,
            OperateTractorAi,
            OperateDockAi,
            OperateUmbilicalAi,
            OperateSecurityAi,
            OperateTransporterAi,
            TickCivilianTraffic,
            OperateDemolitionAi,
            HandleDemolitionCommands,
            // Working views consume the admitted lane after its Input-phase
            // producers; reading that shared buffer must have a declared order.
            ApplyConsoleViews,
        ],
    );
    // Input: reviewed conflicting owner sequence.
    sequence(
        &mut order,
        &[
            HandleExternalRepairCommands,
            HandleTractorCommands,
            HandleDockCommands,
            HandleUmbilicalCommands,
            HandleSecurityCommands,
            HandleTransporterCommands,
        ],
    );
    // Physics: reviewed conflicting owner sequence.
    sequence(
        &mut order,
        &[
            // Incoming energy and shield operation establish this interval's
            // funded protection before any projectile or environmental impact.
            AiPowerAllocation,
            AiShieldFocus,
            HandlePowerMessages,
            TickPowerSystem,
            TickConcealment,
            TickCommsRadio,
            HandleShieldsMessages,
            TickShieldFrequencyCycle,
            TickShields,
            PruneRepairRequestQueue,
            OperateRepairAi,
            HandleLoadTube,
            HandleUnloadTube,
            HandleSensorsMessages,
            OperateNavigationAi,
            HandleNavigationWaypoint,
            ApplyNavigationSharing,
            IssueNavigateToClearance,
            OperateTransitAi,
            HandleTransitCommands,
            OperateCraftBayAi,
            HandleCraftBayCommands,
            AnchorMissionClock,
            ArmMissionDeadlines,
            ArmMissionWorkforces,
            ApplyPendingScenarioLoads,
            ApplyWorldLayerChanges,
            SimulateLowLodShips,
            TickMounts,
            HandleFireBlaster,
            TickBlasterSystem,
            AdvanceCarrierBoltFlights,
            AdvanceTorpedoFlights,
            HandleDispatchRepairTeam,
            HandleRecallRepairTeam,
            HandleSetRepairPriority,
            HandleSetRepairTargetPriority,
            HandleDutyOfficerPosting,
            TickRepairTeams,
            HandleFirePhaser,
            UpdateRegionMembership,
            ApplyDamageZoneDamage,
            BuildWorldSnapshot,
            EmitAttackedOnNewAttacker,
            AiTorpedoAutoFire,
            HandleGuidanceCommands,
            TickIllumination,
            HandleFireTorpedo,
            RefreshGuidanceKnowledge,
            HandleTorpedoMagazineInterSystem,
            PublishShipDisablement,
            CollectWorldEvents,
            TickTriggerPipeline,
            TickScriptCallbacks,
            OpenScriptedCommsThreads,
            TickDelayedActions,
            BuildHelmAiSurfacesFrame,
            AiPolicyStateTick,
            AiHelmLateralThrust,
            AiHelmImpulse,
            AiHelmVerticalThrust,
            AiHelmThrust,
            AiHelmBoost,
            AiHelmSteering,
            ProcessHelmInputs,
            ApplyHelmMovementControls,
            IntegrateShipPhysics,
            TickTransit,
            PublishJoystickToEngines,
            SyncShipPosition,
            OperateHelmEngineAi,
            CommitCraftHandling,
        ],
    );
    // Physics: reviewed conflicting owner sequence.
    sequence(&mut order, &[TickImpulse, TickBoost]);
    // Damage: reviewed conflicting owner sequence.
    sequence(
        &mut order,
        &[
            ApplyGmDirectEffects,
            TickBeamsPrepare,
            HandleBlasterHits,
            TickTorpedoLifecycle,
            HandleCollisions,
            DetectDamageTierCrossings,
            SyncConsoleDamageTiers,
            TickBeamsApplyDamage,
            TickBeamsTickLifetimes,
        ],
    );
    // Modifiers: reviewed conflicting owner sequence.
    sequence(
        &mut order,
        &[
            TickExternalRepair,
            TickPowerBrownoutAdvisory,
            DrainScriptedPowerOrders,
            MirrorWeaponsColdFlags,
            RefreshAnchoredWaypoint,
            ShareNavigationObservations,
            TranslatePowerModifiers,
            TranslateImpulseModifiers,
            ApplyRadarDamageModifiers,
            TickTractor,
            FinishTractorActionFeedback,
            MoveCoupledTarget,
            ApplyTowLoadPenalty,
            TickDock,
            TickCraftHandling,
            TickInfrastructureCondition,
            TickUmbilical,
            FinishUmbilicalActionFeedback,
            TickSecurityTeams,
            RevealCivilianContacts,
            TickTransport,
            TickDebrisDrift,
            TickScans,
            AdvanceObjectiveCursors,
        ],
    );
    // Modifiers: reviewed conflicting owner sequence.
    sequence(&mut order, &[ApplyExternalRepair, ArrestHeldDeclines]);
    // Publish: reviewed conflicting owner sequence.
    sequence(
        &mut order,
        &[
            AdvanceSensorReports,
            PublishPruneRemovedStationPuppets,
            PublishIntentNarration,
            PublishWeaponsCoreBlackboard,
            PublishTacticalRadarBlackboard,
            PublishPhaserBankBlackboards,
            PublishTorpedoTubeBlackboards,
            PublishTorpedoMagazineBlackboard,
            PublishRepairBlackboard,
            PublishPowerBlackboard,
            PublishShieldsBlackboard,
            PublishSensorsBlackboard,
            PublishSensorRadarBlackboard,
            PublishNavigationBlackboard,
            PublishTransit,
            PublishCraftBay,
            PublishCommsBlackboard,
            PublishTractorBlackboard,
            PublishDockBlackboard,
            PublishUmbilicalBlackboard,
            PublishSecurityBlackboard,
            RecordCivilianCasualties,
            PublishTransporterBlackboard,
            PublishDossierBlackboard,
            PublishScanBlackboard,
            PublishDemolitionBlackboard,
            PublishCommandBlackboard,
            PublishHelmBlackboard,
            PublishCaptainBlackboard,
            PublishSensorReferences,
            PublishObservedApproaches,
        ],
    );
    // PublishAggregate: reviewed conflicting owner sequence.
    sequence(
        &mut order,
        &[
            AggregateDoctrineBlackboards,
            PublishViewscreenBlackboard,
            ClearLastAttackerOnDeath,
            ClearLastAttackerOnRedAlertOff,
            BroadcastBlackboardUpdates,
        ],
    );
    // Broadcast: reviewed conflicting owner sequence.
    sequence(
        &mut order,
        &[
            DetectReachedObjectiveCompletion,
            SettleStationPuppetFeedback,
            SettleRemovedStationFeedback,
            BroadcastCommsState,
            BroadcastObjectiveSummary,
            SimDispatch,
            UpdateCombatActivity,
        ],
    );
    // Keep identity, lobby projection, output and streaming at their
    // captured pre-admission boundaries. Empty headless-only markers add no
    // systems or run conditions to native and browser hosts.
    // Membership rollback must precede every pre-Input reader too, not only
    // the gameplay phase chain. These execution-only edges leave the existing
    // deferred barriers intact and do not reorder those owners among themselves.
    for owner in [
        UnregisterOnDespawn,
        EmitPhaseChangeBalanceEvents,
        ReconcileRuntimeEntities,
        SimProcessingAnchor,
        CheckDestroyedAsteroids,
        PreAdmissionPruneStationPuppets,
        ReplicateLocalCrewRatings,
        AdmitConsoleViewRequests,
    ] {
        order.before(super::SimSet::Membership, owner);
    }
    order.before(
        super::SimSet::Membership,
        crate::command_admission::AdmissionSet,
    );
    order.before(UnregisterOnDespawn, RegisterAiTokensOnSpawn);
    order.before(
        RegisterAiTokensOnSpawn,
        crate::command_admission::AdmissionSet,
    );
    order.before(EmitPhaseChangeBalanceEvents, super::SimSet::Input);
    order.before(HeadlessAutoStart, crate::lobby::LobbySystemSet);
    order.before(
        BroadcastWorldSetupOnStart,
        crate::core::broadcast::ReconnectBoundary,
    );
    order.before(
        SimProcessingAnchor,
        crate::core::broadcast::ReconnectBoundary,
    );
    order.before(crate::core::broadcast::ReconnectBoundary, DrainLobbyOutbox);
    order.before(DrainLobbyOutbox, crate::command_admission::AdmissionSet);
    order.before(
        crate::core::broadcast::ReconnectBoundary,
        ReconcileStationPuppetControl,
    );
    order.before(crate::lobby::LobbySystemSet, CheckDestroyedAsteroids);
    order.before(
        crate::core::broadcast::ReconnectBoundary,
        CheckDestroyedAsteroids,
    );
    order.before(UpdateAsteroidWindow, super::SimSet::Input);
    order.before(PublishScenarioState, EmitScenarioNarrative);
    // Both owners queue entity changes for the same existing flush. Preserve
    // their application order even though Commands hides those component writes.
    order.before(UpdateRegionMembership, LodAiShips);
    app.init_schedule(FixedUpdate);
    let schedule = app.get_schedule_mut(FixedUpdate).unwrap();
    assert!(
        schedule.systems().is_err(),
        "fixed order must be installed before initialization"
    );
    assert!(schedule.get_build_settings().auto_insert_apply_deferred);
    // Schedule::default installs AutoInsertApplyDeferred first. Appending our
    // pass preserves the original sync semantics before adding execution edges.
    schedule.add_build_pass(order);
}

fn sequence(order: &mut DeclaredOrder, owners: &[FixedStep]) {
    for pair in owners.windows(2) {
        order.before(pair[0], pair[1]);
    }
}
