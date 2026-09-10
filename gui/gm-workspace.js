/** Shared full GM workspace for browser hosts and native local surfaces.
 * Transport and simulation stay behind the late-bound __host action adapter.
 * This module only composes the existing GM presenters and local input state.
 */
import './components/ph-navigation-map.js';
import { createGmActivityFeed } from './gm-activity-feed.js';
import { createGmLocalProjection } from './gm-local-projection.js';
import { createGmDirectEffectPanel } from './gm-direct-effect-panel.js';
import { createGmConfirmationProfile, createGmConfirmationController } from './gm-confirmation.js';
import { createGmSessionControls } from './gm-session-controls.js';
import { createGmMissionPanel } from './gm-mission-panel.js';
import { createGmObjectivePanel } from './gm-objective-panel.js';
import { createGmCommsPanel } from './gm-comms-panel.js';
import { createGmSpawnPanel } from './gm-spawn-panel.js';
import { createGmSystemPanel } from './gm-system-panel.js';
import { createGmContactPanel } from './gm-contact-panel.js';
import { createGmDespawnPanel } from './gm-despawn-panel.js';
import { createGmNpcPanel } from './gm-npc-panel.js';
import { createGmStationPuppet } from './gm-station-puppet.js';
import { createGmRolePresets } from './gm-role-presets.js';
import { createGmKnowledgeCompare } from './gm-knowledge-compare.js';
import {
  ActionFeedbackLifecycle,
  emitActionFeedbackTransition,
} from './action-feedback.js';
import { createHostActionRegistry } from './host-actions.js';
import { t, has } from './strings.js';
import { mountGmWorkspaceShell } from './gm-workspace-shell.js';

export function mountGmWorkspace({ win = window, doc = win.document } = {}) {
  const shell = mountGmWorkspaceShell({ doc, win, t, has, selectEntity: id => gmProjection.select(id) });
  win.__hostGmShellMetadata = shell.metadata;
  // Late-bound because the panel needs the projection's selection and the
  // projection needs the panel's `select`; the closure resolves at call time,
  // after both exist.
  let gmDirectEffect = null;
  let gmContact = null;
  let gmSystem = null;
  let gmDespawn = null;
  let gmNpc = null;
  let gmObjectivePanel = null;
  const gmProjection = createGmLocalProjection({
    doc: doc,
    t,
    onSelectionChanged: (entity) => {
      shell.selection(entity);
      if (gmDirectEffect) gmDirectEffect.select(entity);
      if (gmDespawn) gmDespawn.select(entity);
      if (gmContact) gmContact.select(entity);
      if (gmSystem) gmSystem.select(entity);
      if (gmNpc) gmNpc.select(entity);
      // Objectives narrow to the selected ship; the mission panel's events
      // stay scenario-wide.
      if (gmObjectivePanel) gmObjectivePanel.select(entity);
    },
  });
  const gmActivity = createGmActivityFeed({
    doc: doc,
    t,
    containsEntity: (id) => gmProjection.contains(id),
    selectEntity: (id) => gmProjection.select(id),
  });
  const hostActionFeedback = new ActionFeedbackLifecycle({
    onTransition: (value) => emitActionFeedbackTransition(win, value),
  });
  const hostSemanticActions = createHostActionRegistry({
    actionFeedback: hostActionFeedback,
    // Resolve at activation time: the classic page seam binds after this
    // module island, while the registry itself is page-scoped from boot.
    toggleQrCode: () => typeof win.__hostToggleQrCode === 'function'
      ? win.__hostToggleQrCode() : false,
  });
  const gmSessionControls = createGmSessionControls({
    doc: doc,
    t,
    actions: hostSemanticActions,
    actionFeedback: hostActionFeedback,
    submitSessionPaused: (active, correlation) =>
      typeof win.__hostSetSessionPaused === 'function'
        && win.__hostSetSessionPaused(active, correlation),
    getOperator: () => typeof win.__hostLocalGm === 'function'
      ? win.__hostLocalGm() : null,
    getOperatorName: (id) => typeof win.__hostGmName === 'function'
      ? win.__hostGmName(id) : id,
  });
  win.__hostGmSessionRefresh = gmSessionControls.refreshAdmission;
  win.__hostGmSessionReset = gmSessionControls.reset;
  win.__hostGmSessionState = gmSessionControls.state;
  let operatorStorage = null;
  try { operatorStorage = win.localStorage; } catch (_) { /* Settings reports unavailable storage. */ }
  const gmConfirmationProfile = createGmConfirmationProfile({ storage: operatorStorage, registry: hostSemanticActions });
  const gmConfirmations = createGmConfirmationController({ doc: doc, t, profile: gmConfirmationProfile });
  win.__hostGmConfirmationProfile = gmConfirmationProfile;
  win.__hostGmConfirmations = gmConfirmations;
  hostSemanticActions.setConfirmationHandler(({ definition, accept }) => gmConfirmations.request({
    category: definition.confirmationCategory,
    description: t(definition.labelId),
    preview: () => t(definition.accessibilityLabelId),
    accept,
  }));
  const gmMissionPanel = createGmMissionPanel({
    doc: doc,
    win: win,
    t,
    actionFeedback: hostActionFeedback,
    confirmAction: gmConfirmations.request,
    submitFireEvent: (request) =>
      typeof win.__hostFireGmEvent === 'function'
        && win.__hostFireGmEvent(request),
    submitSetEventPaused: (request) =>
      typeof win.__hostSetGmEventPaused === 'function'
        && win.__hostSetGmEventPaused(request),
    submitArmSkip: (request) =>
      typeof win.__hostArmGmEventSkip === 'function'
        && win.__hostArmGmEventSkip(request),
    getOperator: () => typeof win.__hostLocalGm === 'function'
      ? win.__hostLocalGm() : null,
    getOperatorName: (id) => typeof win.__hostGmName === 'function'
      ? win.__hostGmName(id) : id,
  });
  gmObjectivePanel = createGmObjectivePanel({ doc: doc, t, actionFeedback: hostActionFeedback,
    confirmAction: gmConfirmations.request,
    getOperator: () => win.__hostLocalGm?.() || null,
    getOperatorName: (id) => win.__hostGmName?.(id) || id,
    getShipName: (id) => {
      const ship = win.__hostGmStationState?.().projection?.ships?.find((row) => row.ship_id === id);
      return ship?.name ? (has(ship.name) ? t(ship.name) : ship.name) : id;
    },
    submit: (request) => win.__hostObjectiveAction?.(request) ?? false,
  });
  win.__hostGmObjectiveState = gmObjectivePanel.state;
  win.__hostGmMissionRefresh = () => { gmMissionPanel.refreshAdmission(); gmObjectivePanel.refreshAdmission(); };
  win.__hostGmMissionReset = () => { gmMissionPanel.reset(); gmObjectivePanel.reset(); };
  win.__hostGmMissionState = gmMissionPanel.state;
  // The placement panel (issue #1305). It reads the SAME chart the omniscient
  // projection renders into, so the gesture that picks a world position is the
  // one the operator is already looking at rather than a second map.
  const gmCommsPanel = createGmCommsPanel({ doc: doc, t, actionFeedback: hostActionFeedback,
    confirmAction: gmConfirmations.request,
    getOperator: () => win.__hostLocalGm?.() || null,
    getOperatorName: id => win.__hostGmName?.(id) || id,
    submitTransmission: request => win.__hostTransmitComms?.(request) === true,
  });
  win.__hostGmCommsRefresh = gmCommsPanel.refreshAdmission;
  win.__hostGmCommsReset = gmCommsPanel.reset;
  win.__hostGmCommsState = gmCommsPanel.state;
  const gmSpawnPanel = createGmSpawnPanel({
    doc: doc,
    win: win,
    t,
    actionFeedback: hostActionFeedback,
    confirmAction: gmConfirmations.request,
    getMap: () => doc.getElementById('gm-entity-map'),
    submitPlacement: (request) =>
      typeof win.__hostSpawnPaletteEntity === 'function'
        && win.__hostSpawnPaletteEntity(request),
    getOperator: () => typeof win.__hostLocalGm === 'function'
      ? win.__hostLocalGm() : null,
    getOperatorName: (id) => typeof win.__hostGmName === 'function'
      ? win.__hostGmName(id) : id,
  });
  win.__hostGmSpawnRefresh = gmSpawnPanel.refreshAdmission;
  win.__hostGmSpawnReset = gmSpawnPanel.reset;
  win.__hostGmSpawnState = gmSpawnPanel.state;
  win.__hostGmActivityState = gmActivity.state;
  const gmStationPuppet = createGmStationPuppet({
    doc: doc,
    win: win,
    t,
    confirmAction: gmConfirmations.request,
    getOperator: () => typeof win.__hostLocalGm === 'function'
      ? win.__hostLocalGm() : null,
    submitStationPuppet: (request) =>
      typeof win.__hostSetStationPuppet === 'function'
        && win.__hostSetStationPuppet(request),
    submitStationCommand: (request) =>
      typeof win.__hostIssueStationCommand === 'function'
        && win.__hostIssueStationCommand(request),
  });
  win.__hostGmStationRefresh = gmStationPuppet.refresh;
  win.__hostGmStationState = gmStationPuppet.state;
  // Presentation-only role presets (issue #1319). `onSelect` fires only on
  // the operator's OWN explicit live switch; the classic script below turns
  // that into a `rememberGmIdentity` write when a reconnectable GM identity
  // exists (fleet member/reconnect path only -- an owner GM has none, same
  // as `reconnectCredential`).
  const gmRolePresets = createGmRolePresets({
    doc: doc,
    t,
    onSelect: (presetId) => {
      if (typeof win.__hostGmRolePresetChanged === 'function') {
        win.__hostGmRolePresetChanged(presetId);
      }
    },
  });
  win.__hostGmRolePresetsSetAvailable = gmRolePresets.setAvailablePresets;
  win.__hostGmRolePresetsRestore = gmRolePresets.restore;
  win.__hostGmRolePresetsState = gmRolePresets.state;
  // Truth / Crew Knowledge / Difference comparison (issue #1318). A pure
  // consumer of the other two controllers' already-parsed public state —
  // never re-parses the raw Host Channel payload itself — and keeps its
  // own selection independent of gmStationPuppet's takeover selection.
  const gmKnowledgeCompare = createGmKnowledgeCompare({ doc: doc, t });
  win.__hostGmKnowledgeState = gmKnowledgeCompare.state;
  // Direct damage/repair on the selected entity (issue #1310). Its
  // authoritative results ride the same `gm_entity` projection the map does.
  gmDirectEffect = createGmDirectEffectPanel({
    doc: doc,
    win: win,
    t,
    actionFeedback: hostActionFeedback,
    confirmAction: gmConfirmations.request,
    getEntity: (id) => gmProjection.state().entities.find((entity) => entity.entity_id === id),
    submitDirectEffect: (request) =>
      typeof win.__hostApplyDirectEffect === 'function'
        && win.__hostApplyDirectEffect(request),
    getOperator: () => typeof win.__hostLocalGm === 'function'
      ? win.__hostLocalGm() : null,
    getOperatorName: (id) => typeof win.__hostGmName === 'function'
      ? win.__hostGmName(id) : id,
  });
  gmSystem = createGmSystemPanel({ doc: doc, t,
    confirmAction: gmConfirmations.request,
    getOperator: () => typeof win.__hostLocalGm === 'function' ? win.__hostLocalGm() : null,
    submit: request => win.__hostSetSystemDisabled(request),
  });
  win.__hostGmSystemState = gmSystem.state;
  gmContact = createGmContactPanel({ doc: doc, t,
    confirmAction: gmConfirmations.request,
    getOperator: () => typeof win.__hostLocalGm === 'function' ? win.__hostLocalGm() : null,
    submit: request => win.__hostSetContactOverride(request),
  });
  win.__hostGmContactState = gmContact.state;
  gmDespawn = createGmDespawnPanel({ doc: doc, t,
    confirmAction: gmConfirmations.request,
    getOperator: () => typeof win.__hostLocalGm === 'function' ? win.__hostLocalGm() : null,
    submit: (request) => win.__hostDespawnEntity(request),
  });
  win.__hostGmDespawnState = gmDespawn.state;
  gmNpc = createGmNpcPanel({ doc: doc, t, getOperator: () => win.__hostLocalGm?.() || null,
    submit: request => win.__hostSetNpcDoctrine(request),
    confirmAction: gmConfirmations.request,
  });
  win.__hostGmNpcState = gmNpc.state;
  win.__hostGmEffectRefresh = function() { gmDirectEffect.refreshAdmission(); gmDespawn.refreshAdmission(); gmContact.refreshAdmission(); gmSystem.refreshAdmission(); gmNpc.refreshAdmission(); };

  win.__hostGmEffectReset = function() { gmConfirmations.cancel(); gmDirectEffect.reset(); gmDespawn.reset(); gmContact.reset(); gmSystem.reset(); gmNpc.reset(); };
  win.__hostGmEffectState = gmDirectEffect.state;
  win.__hostSemanticActions = hostSemanticActions;
  win.__hostActionFeedback = hostActionFeedback;
  const handlers = {
    gm_entity:    function(p) {
      gmDespawn.update(p);
      gmContact.update(p);
      gmSystem.update(p);
      gmNpc.update(p);
      if (gmProjection.update(p)) {
        gmActivity.reconcileAvailability();
        gmKnowledgeCompare.updateTruth(gmProjection.state().entities);
        shell.refresh(gmProjection.state());
      }
      // The directed-effect results ride the same payload, so they fold
      // even if the entity list itself was rejected as malformed.
      if (gmDirectEffect) gmDirectEffect.update(p);
      gmConfirmations.refresh();
    },
    gm_activity:  function(p) { gmActivity.update(p); },
    gm_station:   function(p) {
      if (gmStationPuppet.update(p)) {
        gmKnowledgeCompare.updateStations(gmStationPuppet.state().projection);
        shell.refresh(null, gmStationPuppet.state().projection);
      }
    },
    gm_session:   function(p) { gmSessionControls.update(p); },
    gm_mission:   function(p) { gmMissionPanel.update(p); gmObjectivePanel.update(p); },
    gm_comms:     function(p) { gmCommsPanel.update(p); shell.refresh(); },
    gm_spawn:     function(p) { gmSpawnPanel.update(p); shell.refresh(); },
  };
  return {
    handlers,
    dispose() { shell.dispose(); },
    refreshAdmission() {
      shell.refresh();
      gmSessionControls.refreshAdmission();
      win.__hostGmMissionRefresh();
      gmCommsPanel.refreshAdmission();
      gmSpawnPanel.refreshAdmission();
      gmStationPuppet.refresh();
      win.__hostGmEffectRefresh();
    },
    reset() {
      gmSessionControls.reset();
      win.__hostGmMissionReset();
      gmCommsPanel.reset();
      gmSpawnPanel.reset();
      win.__hostGmEffectReset();
    },
  };
}
