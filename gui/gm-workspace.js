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
import { createGmAttentionPanel } from './gm-attention-panel.js';
import { createGmAttentionFilters } from './gm-attention-filters.js';
import { createGmHealthBanner } from './gm-health-banner.js';
import { createGmHealthPanel } from './gm-health-panel.js';
import { createGmWorkloadPanel } from './gm-workload-panel.js';
import { createGmWidgetsPanel } from './gm-widgets-panel.js';
import { createGmKnowledgeCompare } from './gm-knowledge-compare.js';
import { createGmJournalPanel } from './gm-journal-panel.js';
import { createGmFactionPanel } from './gm-faction-panel.js';
import { createGmCheckpointPanel } from './gm-checkpoint-panel.js';
import { createGmRestoreControl } from './gm-restore-control.js';
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
  // Saved action history (issue #1441). A pure reader over the same canonical
  // journal the Session controls settle their own results against — it rides
  // the `gm_session` projection rather than opening a second channel, because
  // it is not a second log.
  //
  // It also carries the Undo control (issue #1442): the inverse is an ordinary
  // typed GM action, so it is submitted through the same late-bound host
  // adapter every other panel uses and its answer arrives as a row of this same
  // journal. The confirmation controller is bound below, once it exists.
  let gmConfirmationsRef = null;
  const gmJournalPanel = createGmJournalPanel({
    doc: doc,
    t,
    getOperatorName: (id) => typeof win.__hostGmName === 'function'
      ? win.__hostGmName(id) : id,
    getOperator: () => typeof win.__hostLocalGm === 'function' ? win.__hostLocalGm() : null,
    submitUndo: (request) => typeof win.__hostUndoGmAction === 'function'
      && win.__hostUndoGmAction(request),
    confirmAction: (request) => (gmConfirmationsRef
      ? gmConfirmationsRef.request(request)
      : request.accept()),
  });
  win.__hostGmJournalState = gmJournalPanel.state;
  // Named checkpoints (issue #1445). The save API is the page's own — the same
  // one gui/save-slots.js is handed — so a bookmark is the ordinary manual
  // capture with a name, not a second storage path. Late-bound through the
  // window seam because the classic host script installs it after this module
  // island; an absent API leaves the panel readable and its controls inert
  // rather than throwing at mount.
  // Declared before the checkpoint panel so its selection callback can reach
  // it; assigned after the confirmation controller exists.
  let gmRestoreControl = null;
  const gmCheckpointPanel = createGmCheckpointPanel({
    doc: doc,
    t,
    api: {
      list: () => (typeof win.__hostGmCheckpointList === 'function'
        ? win.__hostGmCheckpointList() : []),
      create: (name) => (typeof win.__hostGmCheckpointCreate === 'function'
        ? win.__hostGmCheckpointCreate(name) : ''),
    },
    canCapture: () => win.__saveSlotsCaptureAvailable === true,
    // The restore control (issue #1446) sits under the SAME candidate list, so
    // the row a GM previewed is the row they restore: a second picker could
    // hold a different selection from the one whose reasons are on screen.
    onSelect: () => gmRestoreControl?.refresh(),
  });
  win.__hostGmCheckpointPanel = gmCheckpointPanel;
  win.__hostGmCheckpointState = gmCheckpointPanel.state;
  win.__hostGmSessionRefresh = gmSessionControls.refreshAdmission;
  win.__hostGmSessionReset = gmSessionControls.reset;
  win.__hostGmSessionState = gmSessionControls.state;
  let operatorStorage = null;
  try { operatorStorage = win.localStorage; } catch (_) { /* Settings reports unavailable storage. */ }
  const gmConfirmationProfile = createGmConfirmationProfile({ storage: operatorStorage, registry: hostSemanticActions });
  const gmConfirmations = createGmConfirmationController({ doc: doc, t, profile: gmConfirmationProfile });
  gmConfirmationsRef = gmConfirmations;
  // The faction-relation control (issue #1442): the only GM surface that can
  // move a hostility, and the source of the actions the journal's Undo
  // reverses. It reads the authored roster off this same `gm_session` payload.
  const gmFactionPanel = createGmFactionPanel({
    doc: doc,
    t,
    getOperator: () => typeof win.__hostLocalGm === 'function' ? win.__hostLocalGm() : null,
    submit: (request) => typeof win.__hostSetFactionHostility === 'function'
      && win.__hostSetFactionHostility(request),
    confirmAction: gmConfirmations.request,
  });
  win.__hostGmFactionState = gmFactionPanel.state;
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
  // The M4 attention queue (issue #1433). Its filters/snoozes are one private
  // controller so #1442's authored widgets reuse this exact state rather than
  // growing a second copy; the session id arrives later (the fleet join
  // resolves it), so `restore` is called then, like the role presets.
  // The panel is built FROM the controller, so it cannot be named here yet;
  // the repaint is resolved lazily instead. Without it a same-session
  // reconnect restores the operator's bands and snoozes into a queue that goes
  // on showing the old rows until membership happens to change.
  let repaintGmAttention = () => {};
  // The authored widget region (issue #1439) mirrors the same private filter
  // state, so it repaints on the same change. Late-bound for the same reason
  // the queue's repaint is: the controller is built before either consumer.
  let repaintGmWidgets = () => {};
  const gmAttentionFilters = createGmAttentionFilters({
    storage: operatorStorage,
    getSessionId: () => (typeof win.__hostGmSessionId === 'function' ? win.__hostGmSessionId() : null),
    getOperatorId: () => (typeof win.__hostLocalGm === 'function' ? (win.__hostLocalGm()?.id ?? null) : null),
    onChange: () => { repaintGmAttention(); repaintGmWidgets(); },
  });
  // Public technical health (issue #1437). Three late-bound edges, because the
  // two halves guard different things and neither may own both: the attention
  // panel owns the banner REGION (nothing an operator does to the queue can
  // reach it), the health component owns what a warning LOOKS like, and the
  // health panel owns the readable table and feeds the region from the same
  // parsed alerts it counts in its own summary.
  let gmHealthPanelRef = null;
  const gmHealthBanner = createGmHealthBanner({
    doc: doc, t,
    onAction: (alert) => {
      if (alert.ship) { gmProjection.select(alert.ship.entity_id); return; }
      gmHealthPanelRef?.focus();
    },
  });
  const gmAttentionPanel = createGmAttentionPanel({
    doc: doc, t, has,
    filters: gmAttentionFilters,
    renderBanners: (rows, container) => gmHealthBanner.render(rows, container),
    // Opening a row is a NAVIGATION to something that already exists on this
    // desk — the authored conversation route, (issue #1434) the authored beat's
    // own mission-panel row carrying the Fire/Pause/Skip its author declared, or
    // (issue #1435) simply the hull an idle-NPC row names. No GmAction, no
    // dialog, no panel switch: the operator presses the lever, and that press
    // takes the ordinary admission check and the ordinary apply-tick
    // revalidation with it.
    //
    // Each row lights only the halves it actually names. An idle-NPC row names
    // a ship and nothing else: selecting it is what draws that ship's inspector
    // and the actions the ship allows, and the absent route and event are why
    // nothing here chooses, offers or issues an order on the operator's behalf.
    onOpen: (occurrence) => {
      if (occurrence.target.ship) gmProjection.select(occurrence.target.ship.entity_id);
      if (occurrence.target.route) gmCommsPanel.focusRoute(occurrence.target.route);
      if (occurrence.target.event) gmMissionPanel.focusEvent(occurrence.target.event.id);
    },
  });
  repaintGmAttention = gmAttentionPanel.repaint;
  win.__hostGmAttentionState = gmAttentionPanel.state;
  win.__hostGmAttentionRestore = gmAttentionFilters.restore;
  win.__hostGmAttentionBanners = gmAttentionPanel.banners;
  const gmHealthPanel = createGmHealthPanel({
    doc: doc, t, has,
    banners: (alerts) => gmAttentionPanel.banners(alerts),
  });
  gmRestoreControl = createGmRestoreControl({
    doc: doc,
    t,
    getCandidate: () => gmCheckpointPanel.state().selected,
    getOperator: () => (typeof win.__hostLocalGm === 'function' ? win.__hostLocalGm() : null),
    submitRestore: (request) => typeof win.__hostRequestLiveRestore === 'function'
      && win.__hostRequestLiveRestore(request),
    submitResume: (correlation) => typeof win.__hostSetSessionPaused === 'function'
      && win.__hostSetSessionPaused(false, correlation),
    confirmAction: gmConfirmations.request,
  });
  win.__hostGmRestoreState = gmRestoreControl.state;
  gmHealthPanelRef = gmHealthPanel;
  win.__hostGmHealthState = gmHealthPanel.state;
  // The M4 Station-workload advisory (issue #1438). Read-only by construction:
  // it takes no callbacks because there is nothing on it to act with.
  const gmWorkloadPanel = createGmWorkloadPanel({ doc: doc, t, has });
  win.__hostGmWorkloadState = gmWorkloadPanel.state;
  // The typed world-authored widget region (issue #1439). It composes from the
  // surfaces above rather than from the Host Channel: the attention queue's own
  // parsed occurrences, the workload advisory's own parsed rows, the ONE
  // private filter controller and the shipped GM action buttons. Nothing it
  // draws can disagree with the panel it mirrors, and nothing it draws is a
  // new action — see gui/gm-widgets-panel.js.
  const gmWidgetsPanel = createGmWidgetsPanel({
    doc: doc, t, has,
    filters: gmAttentionFilters,
    readAttention: () => gmAttentionPanel.state(),
    readWorkload: () => gmWorkloadPanel.state(),
    repaintAttention: () => repaintGmAttention(),
  });
  repaintGmWidgets = gmWidgetsPanel.repaint;
  win.__hostGmWidgetsState = gmWidgetsPanel.state;
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
    // The authored widget composition follows the EFFECTIVE preset (issue
    // #1439), including the fallback to the built-in All a removed preset
    // resolves to — All authors no widgets, so the region goes away on its own.
    // Only the operator's own live switch carries the authored default filters
    // with it; a reconnect restores what they actually left behind.
    onEffective: (preset, context) => gmWidgetsPanel.setPreset(preset, context),
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
    gm_session:   function(p) {
      gmSessionControls.update(p);
      gmFactionPanel.update(p);
      gmJournalPanel.update(p);
      // A refused restore request never reaches the health projection — no
      // restore started — so its answer is read off the ONE canonical journal.
      let session = p;
      if (typeof session === 'string') {
        try { session = JSON.parse(session); } catch (_) { session = null; }
      }
      gmRestoreControl.settleJournal(session?.journal?.entries);
    },
    gm_mission:   function(p) { gmMissionPanel.update(p); gmObjectivePanel.update(p); },
    gm_comms:     function(p) { gmCommsPanel.update(p); shell.refresh(); },
    gm_attention: function(p) { gmAttentionPanel.update(p); gmWidgetsPanel.repaint(); },
    gm_health:    function(p) { gmHealthPanel.update(p); gmRestoreControl.update(p); },
    gm_workload:  function(p) { gmWorkloadPanel.update(p); gmWidgetsPanel.repaint(); },
    gm_spawn:     function(p) { gmSpawnPanel.update(p); shell.refresh(); },
  };
  return {
    handlers,
    dispose() { gmAttentionPanel.dispose(); gmWorkloadPanel.dispose(); gmWidgetsPanel.dispose(); shell.dispose(); },
    refreshAdmission() {
      shell.refresh();
      gmSessionControls.refreshAdmission();
      win.__hostGmMissionRefresh();
      gmCommsPanel.refreshAdmission();
      gmSpawnPanel.refreshAdmission();
      gmStationPuppet.refresh();
      gmFactionPanel.refreshAdmission();
      gmJournalPanel.refreshAdmission();
      gmCheckpointPanel.refresh();
      gmRestoreControl.refresh();
      win.__hostGmEffectRefresh();
    },
    reset() {
      gmAttentionPanel.reset();
      gmHealthPanel.reset();
      gmWorkloadPanel.reset();
      gmWidgetsPanel.reset();
      gmSessionControls.reset();
      gmFactionPanel.reset();
      gmJournalPanel.reset();
      gmCheckpointPanel.reset();
      gmRestoreControl.reset();
      win.__hostGmMissionReset();
      gmCommsPanel.reset();
      gmSpawnPanel.reset();
      win.__hostGmEffectReset();
    },
  };
}
