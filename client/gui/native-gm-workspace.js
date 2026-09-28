/** Native host-local transport adapter for the shared full GM workspace. */
import './strings-boot.js';
import { mountGmWorkspace } from './gm-workspace.js';
import { createHostChannel } from './host-channel.js';
import { t, has, localiseTree, applyToDom } from './strings.js';
import { createLocalePreference, GM_LOCALE_STORAGE_KEY } from './locale-preference.js';
import { mountSurfaceLanguage } from './surface-language.js';

import { nativeGmActionAdapter, installGmActionBindings } from './gm-action-bindings.js';

export function mountNativeGmWorkspace({ bridge, win = window, doc = win.document }) {
  win.__phoenixGmPage = true;
  // The reused host markup includes launch overlays that its browser boot
  // normally dismisses. That boot never runs on this private native surface.
  // World selection belongs to the viewscreen, not the GM's Ready controls.
  for (const id of ['landing-panel', 'scenario-panel', 'wasm-spinner']) {
    doc.getElementById(id)?.remove();
  }
  let metadata = { phase: 'Lobby', gms: [] };
  let disposed = false;
  let lastStartResult = null;
  let saveOutcomes = [];
  let manualSave = null;
  let actionBindings;
  const settleCheckpoint = () => {
    if (disposed || !actionBindings?.isCurrent('__hostGmCheckpointCreate')) return;
    const state = win.__hostGmCheckpointState?.();
    const outcome = saveOutcomes.find(row => row.slot === state?.pendingSlotId);
    if (outcome) win.__hostGmCheckpointPanel?.reportOutcome(outcome.ok, outcome.error || '');
    const manualOutcome = saveOutcomes.find(row => row.slot === manualSave?.slot);
    if (manualOutcome) {
      manualSave.status.textContent = manualOutcome.ok ? t('server.gm.checkpoint.confirmed', {
        name: manualSave.name, tick: manualOutcome.tick, time: new Date().toLocaleTimeString(),
      }) : manualOutcome.error;
      manualSave.slot = null;
      manualSave.pending = false;
      manualSave.button.disabled = metadata.phase !== 'InProgress';
      if (manualOutcome.ok) {
        manualSave.input.value = '';
        win.__hostGmTemporaryActions?.succeeded('manual-save');
      }
    }
  };
  const getOperator = () => {
    if (disposed) return null;
    const operator = bridge.getOperator();
    return operator && operator.connected !== false && typeof operator.id === 'string'
      ? operator : null;
  };
  win.__hostLocalGm = getOperator;
  win.__hostGmInspectorInterest = panels => bridge.inspectorInterest?.(panels);
  win.__hostGmConsoleInterest = request => bridge.consoleInterest?.(request);
  if (bridge.saveRequest) {
    win.__hostGmCheckpointList = () => disposed ? [] : bridge.saveRequest('list');
  }
  win.__hostGmName = id => metadata.gms.find(row => row.id === id)?.name || id;
  actionBindings = installGmActionBindings(win, nativeGmActionAdapter({
    getOperator,
    submitAction: request => bridge.submitAction(request),
    createCheckpoint: bridge.saveRequest ? name => bridge.saveRequest('create', name).then(slot => {
      if (!actionBindings.isCurrent('__hostGmCheckpointCreate')) return '';
      win.setTimeout(settleCheckpoint, 0);
      return slot;
    }, error => {
      if (!actionBindings.isCurrent('__hostGmCheckpointCreate')) return '';
      throw error;
    }) : undefined,
  }));

  applyToDom(doc);
  const workspace = mountGmWorkspace({ win, doc, requireNativeProvider: true });
  const locale = createLocalePreference({ doc,
    nav: win.PhoenixOsLocale ? { nativeLocale: win.PhoenixOsLocale } : win.navigator,
    storage: win.PhoenixLocaleStorage || win.localStorage, storageKey: GM_LOCALE_STORAGE_KEY,
    findConsoles: () => [], onChange: () => workspace.refreshLanguage() });
  const languageHost = doc.getElementById('gm-language-control');
  if (languageHost) languageHost.append(mountSurfaceLanguage({ doc, id: 'gm-language', preference: locale }).root);
  const reloadLanguage = () => locale.reloadStored();
  win.addEventListener('phoenix-native-locale-loaded', reloadLanguage);
  locale.apply();
  const dispatch = createHostChannel({
    handlers: workspace.handlers,
    strings: { t, has, localiseTree },
  });
  const readyButton = doc.getElementById('gm-ready-btn');
  const forceButton = doc.getElementById('gm-force-start-btn');
  const result = doc.getElementById('gm-start-result');
  if (bridge.saveRequest) {
    const manual = doc.getElementById('manual-save-panel');
    if (manual) {
      manual.hidden = false;
      const input = doc.createElement('input'); input.placeholder = t('server.gm.checkpoint.name_required');
      input.setAttribute('aria-label', input.placeholder);
      const button = doc.createElement('button'); button.textContent = t('server.gm.shell.layout.panel.manual_save');
      const status = doc.createElement('p'); status.setAttribute('role', 'status');
      manualSave = { button, status, input, slot: null, name: '', pending: false };
      button.addEventListener('click', async () => {
        if (manualSave.pending || disposed || !actionBindings.isCurrent('__hostGmCheckpointCreate')) return;
        manualSave.pending = true;
        button.disabled = true;
        try {
          manualSave.name = input.value;
          const slot = await bridge.saveRequest('create', input.value);
          if (disposed || !actionBindings.isCurrent('__hostGmCheckpointCreate')) return;
          manualSave.slot = slot;
          status.textContent = t('server.gm.checkpoint.pending', { name: input.value });
          settleCheckpoint();
        } catch (error) {
          if (disposed || !actionBindings.isCurrent('__hostGmCheckpointCreate')) return;
          manualSave.pending = false;
          status.textContent = error.message; button.disabled = metadata.phase !== 'InProgress';
        }
      });
      manual.replaceChildren(input, button, status);
    }
  }
  const recoveryButton = doc.createElement('button');
  recoveryButton.id = 'gm-return-to-host-lobby';
  recoveryButton.type = 'button';
  recoveryButton.hidden = true;
  recoveryButton.textContent = t('server.gm.return_to_host_lobby');
  doc.getElementById('gm-start-controls')?.append(recoveryButton);
  let recoveryPending = false;
  const inLobby = () => metadata.phase === 'Lobby';
  function renderStart() {
    const operator = getOperator();
    const recoveryAvailable = inLobby() && metadata.host_lobby_unavailable === true;
    recoveryButton.hidden = !recoveryAvailable;
    if (!recoveryAvailable) recoveryPending = false;
    recoveryButton.disabled = !operator || recoveryPending || !recoveryAvailable;
    doc.getElementById('gm-start-controls')?.setAttribute('aria-hidden', operator ? 'false' : 'true');
    if (readyButton) {
      readyButton.textContent = t(operator?.ready ? 'server.gm.start.unready' : 'server.gm.start.ready');
      readyButton.disabled = !operator || !inLobby();
    }
    win.__hostGmSessionContext?.({ phase: metadata.phase });
    win.__saveSlotsCaptureAvailable = metadata.phase === 'InProgress';
    if (manualSave) manualSave.button.disabled = !win.__saveSlotsCaptureAvailable || manualSave.pending;
    win.__hostGmCheckpointPanel?.setPhase(metadata.phase);
    if (forceButton) {
      forceButton.textContent = t('server.gm.start.force');
      forceButton.disabled = !operator || !inLobby();
    }
    const policy = doc.getElementById('gm-start-policy');
    if (policy) policy.textContent = metadata.start_policy
      ? t('server.gm.start.summary', {
          ready: metadata.start_policy.ready_total,
          connected: metadata.start_policy.connected_total,
        }) : t('server.gm.start.waiting_policy');
    const start = metadata.start_result;
    if (result && start?.grant_id && start.grant_id !== lastStartResult) {
      lastStartResult = start.grant_id;
      const key = start.status === 'applied' ? 'server.gm.start.force_applied'
        : start.status === 'no-op' ? 'server.gm.start.already_started'
        : start.reason === 'validation-failed' ? 'server.gm.start.validation_failed'
        : 'server.gm.start.force_refused';
      result.textContent = t(key, { name: win.__hostGmName(start.operator_id) });
    }
  }
  function setReady() {
    const operator = getOperator();
    if (!disposed && operator && inLobby()) bridge.setReady(!operator.ready);
  }
  function forceStart() {
    if (disposed || !getOperator() || !inLobby()) return;
    const accepted = bridge.forceStart();
    if (result) result.textContent = accepted === false ? t('server.gm.start.force_refused') : '';
  }
  function returnToHostLobby() {
    if (disposed || !getOperator() || !inLobby() || metadata.host_lobby_unavailable !== true || recoveryPending) return;
    recoveryPending = bridge.returnToHostLobby?.() === true;
    renderStart();
  }
  readyButton?.addEventListener('click', setReady);
  forceButton?.addEventListener('click', forceStart);
  recoveryButton.addEventListener('click', returnToHostLobby);
  const unsubscribe = bridge.subscribe((channel, payload) => {
    if (disposed) return;
    if (channel === 'save_outcomes') {
      saveOutcomes = typeof payload === 'string' ? JSON.parse(payload) : payload;
      settleCheckpoint();
      win.__hostGmCheckpointPanel?.refresh();
    } else if (channel === 'metadata') {
      let value = payload;
      if (typeof value === 'string') {
        try { value = JSON.parse(value); } catch (_) { return; }
      }
      if (!value || !Array.isArray(value.gms) || typeof value.phase !== 'string') return;
      const previousPhase = metadata.phase;
      metadata = value;
      win.__hostGmShellMetadata?.(metadata);
      if (metadata.phase === 'Lobby' && previousPhase !== 'Lobby') workspace.reset();
      if (Array.isArray(metadata.role_presets)) win.__hostGmRolePresetsSetAvailable(metadata.role_presets);
      workspace.refreshAdmission();
      renderStart();
    } else if (Object.prototype.hasOwnProperty.call(workspace.handlers, channel)) {
      dispatch(channel, payload);
    }
  });
  renderStart();
  return {
    workspace,
    dispose() {
      win.removeEventListener('phoenix-native-locale-loaded', reloadLanguage);
      disposed = true;
      actionBindings.dispose();
      if (win.__hostLocalGm === getOperator) delete win.__hostLocalGm;
      if (typeof unsubscribe === 'function') unsubscribe();
      readyButton?.removeEventListener('click', setReady);
      forceButton?.removeEventListener('click', forceStart);
      recoveryButton.removeEventListener('click', returnToHostLobby);
      recoveryButton.remove();
      workspace.reset();
      workspace.dispose();
    },
  };
}
