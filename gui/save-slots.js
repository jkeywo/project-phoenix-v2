/**
 * gui/save-slots.js — peer-local save catalogue presentation (issue #865).
 *
 * Rust owns capture timing, storage, and compatibility. This module only
 * normalises the browser objects crossing wasm-bindgen, renders them, and
 * adapts explicit local actions back to the bridge. It never restores a live
 * app: starting a row is delegated to the host page, which reloads into the
 * existing fresh-app resume path.
 */

import { t } from './strings.js';
import { createFocusTrap } from './focus-trap.js';
import {
  armBrowserSaveIdentityHandoff,
  prepareBrowserSaveIdentity,
} from './browser-save-identity.js';

export {
  armBrowserSaveIdentityHandoff,
  prepareBrowserSaveIdentity,
} from './browser-save-identity.js';

const KNOWN_REFUSALS = new Set([
  'empty',
  'unreadable',
  'unparsable',
  'format',
  'rules',
  'content',
  'content-pending',
]);

function text(value) {
  return value == null ? '' : String(value);
}

/** Normalise one wasm-bindgen catalogue object without trusting its shape. */
export function normalizeSaveSlot(row) {
  const source = row && typeof row === 'object' ? row : {};
  const slotId = text(source.slot_id);
  const kind = source.kind === 'autosave' ? 'autosave' : 'manual';
  const refusalKind = KNOWN_REFUSALS.has(source.refusal_kind)
    ? source.refusal_kind
    : source.compatible === true
      ? ''
      : 'unreadable';
  const scenario = text(source.scenario);
  const compatible = source.compatible === true;
  const startable = source.startable === true || compatible;
  return {
    slotId,
    kind,
    displayName: text(source.display_name) || slotId,
    scenario,
    selectedShip: text(source.selected_ship),
    captureTick: text(source.capture_tick),
    compatible,
    startable,
    refusalKind,
    refusal: text(source.refusal),
    metadata: text(source.metadata) || 'none',
    metadataError: text(source.metadata_error),
    canStart: startable && scenario !== '' && text(source.selected_ship) !== '',
    canRename: kind === 'manual',
  };
}

function compatibilityLabelId(row) {
  if (row.compatible) return 'server.save_slots.compatible';
  if (row.refusalKind === 'content-pending') return 'server.save_slots.content_pending';
  if (row.refusalKind === 'format') return 'server.save_slots.incompatible_format';
  if (row.refusalKind === 'rules') return 'server.save_slots.incompatible_rules';
  if (row.refusalKind === 'content') return 'server.save_slots.incompatible_content';
  return 'server.save_slots.unreadable';
}

/** Preserve a post-load staging refusal as data the host must present. */
export function startPreparationResult(raw) {
  const message = text(raw);
  return { ok: message === '', message };
}

/**
 * Consume every URL field that can make the next boot a saved-session boot.
 *
 * Kept pure over a window-shaped value so "New Session" can be tested without
 * navigating jsdom. `scenario` and `ship` belong to the same one-shot resume
 * route: the catalogue adds them only so the fresh page can rebuild the saved
 * world's authored component set before restoration.
 */
export function clearPendingResume(windowLike) {
  if (!windowLike || !windowLike.location) return '';
  const url = new URL(windowLike.location.href);
  url.searchParams.delete('resume');
  url.searchParams.delete('scenario');
  url.searchParams.delete('ship');
  const cleaned = url.toString();
  if (windowLike.history && typeof windowLike.history.replaceState === 'function') {
    windowLike.history.replaceState(null, '', cleaned);
  }
  return cleaned;
}

/** Only an actively running simulation can admit a new fixed-tick capture. */
export function captureAvailableForPhase(phase) {
  return phase === 'InProgress';
}

function metadataLabelId(row) {
  if (row.metadata === 'missing') return 'server.save_slots.metadata_missing';
  if (row.metadata === 'corrupt') return 'server.save_slots.metadata_corrupt';
  if (row.metadata === 'unreadable') return 'server.save_slots.metadata_unreadable';
  return '';
}

/**
 * Pure catalogue model used by the renderer and tests.
 * Rust has already sorted the rows; preserving that order is part of the seam.
 */
export function saveSlotsModel(rows, selectedId = null, confirmingId = null) {
  const slots = Array.from(rows || [], normalizeSaveSlot);
  const selected = slots.find((row) => row.slotId === selectedId) || null;
  return {
    slots: slots.map((row) => ({
      ...row,
      selected: !!selected && row.slotId === selected.slotId,
      confirmingDelete: row.slotId === confirmingId,
      compatibilityLabelId: compatibilityLabelId(row),
      metadataLabelId: metadataLabelId(row),
    })),
    selected,
  };
}

function node(doc, tag, className = '') {
  const el = doc.createElement(tag);
  if (className) el.className = className;
  return el;
}

function errorDetail(error) {
  if (error && typeof error.message === 'string' && error.message) return error.message;
  return text(error);
}

/**
 * Mount the thin save catalogue and return its refresh/status adapter.
 *
 * @param {{
 *   root: HTMLElement,
 *   api: {
 *     list: Function, create: Function, rename: Function, export: Function,
 *     takeExport: Function, exportName: Function, delete: Function,
 *   },
 *   doc?: Document,
 *   canCapture?: () => boolean,
 *   download?: (doc: Document, name: string, text: string) => boolean,
 *   onStart?: (row: ReturnType<typeof normalizeSaveSlot>) =>
 *     (boolean | void | Promise<boolean | void>),
 *   onNewSession?: () => void,
 *   onPendingCaptureDropped?: (message: string) => void,
 *   mode?: 'combined' | 'catalogue' | 'manual',
 *   idPrefix?: string,
 *   headerAction?: Element,
 * }} options
 *
 * `headerAction` is a LIVE element moved into the panel's header row, beside
 * the heading (issue #1363). It exists for the save importer: importing a
 * portable save is an action ON this catalogue — it reaches the same World,
 * the same version gate and the same staged restore a row's Start reaches — so
 * it belongs in this panel's header rather than as a separate block of the
 * boot panel, which is where it sat while the catalogue was a column of that
 * panel and the two were only neighbours.
 *
 * The node is MOVED rather than rebuilt, for the reason
 * `gui/host-landing-render.js` moves `#scenario-panel`: the importer is static
 * markup with a page-lifetime click handler and a hidden `<input type=file>`,
 * and a listener survives a re-parent where a re-render would drop it. It is
 * also why the option takes an element and not a string id — this module does
 * not know, and must not learn, which document its caller assembled.
 */
export function mountSaveSlots(options) {
  const root = options && options.root;
  const doc = (options && options.doc) || (root && root.ownerDocument);
  const api = options && options.api;
  if (!root || !doc || !api) throw new TypeError('save slot mount requires root, document, and api');

  // Establish the peer-local namespace before the first catalogue read. The
  // second mount on server.html (the compact in-session surface) reuses the
  // already-installed identity and cannot fork this peer's Store.
  const saveIdentityReady = prepareBrowserSaveIdentity(doc.defaultView || undefined);

  const canCapture = options.canCapture || (() => false);
  const download = options.download || (() => false);
  const onStart = options.onStart || (() => {});
  const onNewSession = options.onNewSession || (() => {});
  const onPendingCaptureDropped = options.onPendingCaptureDropped || (() => {});
  const mode = ['catalogue', 'manual'].includes(options.mode) ? options.mode : 'combined';
  const idPrefix = text(options.idPrefix).trim();
  const domId = (base) => idPrefix ? `${idPrefix}-${base}` : base;
  const showsCatalogue = mode !== 'manual';
  const showsCapture = mode !== 'catalogue';
  const state = {
    rows: [],
    selectedId: null,
    confirmingId: null,
    statusTone: '',
    statusText: '',
    captureAvailable: null,
    pendingCapture: false,
    pendingStart: false,
  };

  root.replaceChildren();
  root.dataset.saveSlotsMode = mode;
  // The header row: the panel's name, and whatever acts on the panel as a
  // whole. A row rather than a bare `<h2>` since issue #1363, because the save
  // importer now sits here — see `headerAction` above.
  const head = node(doc, 'div', 'save-slots-head');
  const heading = node(doc, 'h2', 'save-slots-heading');
  heading.id = domId('save-slots-heading');
  heading.tabIndex = -1;
  heading.textContent = t(mode === 'manual'
    ? 'server.save_slots.manual_heading'
    : 'server.save_slots.heading');
  const headActions = node(doc, 'div', 'save-slots-head-actions');
  head.append(heading, headActions);
  // Guarded on the caller actually handing one over: the native viewscreen
  // carries no file chooser (importing an arbitrary session is host tooling
  // its document strips), and a demo build removes the importer outright. An
  // absent action leaves an empty slot the stylesheet collapses, never a hole
  // where a control should be.
  if (options.headerAction) headActions.appendChild(options.headerAction);
  root.setAttribute('aria-labelledby', heading.id);

  const intro = node(doc, 'p', 'save-slots-intro');
  intro.textContent = t('server.save_slots.intro');

  const newSession = node(doc, 'button', 'save-slots-button');
  newSession.type = 'button';
  newSession.dataset.saveAction = 'new-session';
  newSession.textContent = t('server.save_slots.new_session');

  const createForm = node(doc, 'div', 'save-slots-create');
  const createLabel = node(doc, 'label', 'save-slots-label');
  createLabel.htmlFor = domId('save-slot-name');
  createLabel.textContent = t('server.save_slots.manual_name');
  const createInput = node(doc, 'input', 'save-slots-input');
  createInput.id = domId('save-slot-name');
  createInput.type = 'text';
  createInput.placeholder = t('server.save_slots.manual_placeholder');
  const createButton = node(doc, 'button', 'save-slots-button');
  createButton.type = 'button';
  createButton.dataset.saveAction = 'create';
  createButton.textContent = t('server.save_slots.create');
  const captureHint = node(doc, 'p', 'save-slots-hint');
  captureHint.id = domId('save-slot-capture-hint');
  captureHint.textContent = t('server.save_slots.capture_unavailable');
  createButton.setAttribute('aria-describedby', captureHint.id);
  createForm.append(createLabel, createInput, createButton, captureHint);

  const list = node(doc, 'ul', 'save-slots-list');
  list.dataset.saveList = '';
  list.tabIndex = -1;

  const actions = node(doc, 'div', 'save-slots-actions');
  actions.hidden = true;
  const renameLabel = node(doc, 'label', 'save-slots-label');
  renameLabel.htmlFor = domId('save-slot-rename');
  renameLabel.textContent = t('server.save_slots.rename_label');
  const renameInput = node(doc, 'input', 'save-slots-input');
  renameInput.id = domId('save-slot-rename');
  renameInput.type = 'text';
  const renameButton = node(doc, 'button', 'save-slots-button');
  renameButton.type = 'button';
  renameButton.dataset.saveAction = 'rename';
  renameButton.textContent = t('server.save_slots.rename');
  const exportButton = node(doc, 'button', 'save-slots-button');
  exportButton.type = 'button';
  exportButton.dataset.saveAction = 'export';
  exportButton.textContent = t('server.save_slots.export');
  const startButton = node(doc, 'button', 'save-slots-button primary');
  startButton.type = 'button';
  startButton.dataset.saveAction = 'start';
  startButton.textContent = t('server.save_slots.start');
  const deleteButton = node(doc, 'button', 'save-slots-button danger');
  deleteButton.type = 'button';
  deleteButton.dataset.saveAction = 'delete';
  deleteButton.textContent = t('server.save_slots.delete');
  actions.append(renameLabel, renameInput, renameButton, exportButton, startButton, deleteButton);

  const confirmation = node(doc, 'div', 'save-slots-confirm');
  confirmation.hidden = true;
  confirmation.setAttribute('role', 'alertdialog');
  confirmation.setAttribute('aria-modal', 'true');
  confirmation.setAttribute('aria-labelledby', domId('save-slots-confirm-text'));
  const confirmText = node(doc, 'p');
  confirmText.id = domId('save-slots-confirm-text');
  const cancelButton = node(doc, 'button', 'save-slots-button');
  cancelButton.type = 'button';
  cancelButton.dataset.saveAction = 'cancel-delete';
  cancelButton.textContent = t('server.save_slots.cancel');
  const confirmButton = node(doc, 'button', 'save-slots-button danger');
  confirmButton.type = 'button';
  confirmButton.dataset.saveAction = 'confirm-delete';
  confirmButton.textContent = t('server.save_slots.confirm_delete');
  confirmation.append(confirmText, cancelButton, confirmButton);

  const status = node(doc, 'div', 'save-slots-status');
  status.setAttribute('role', 'status');
  status.setAttribute('aria-live', 'polite');
  status.setAttribute('aria-atomic', 'true');

  if (mode === 'combined') {
    root.append(head, intro, newSession, createForm, list, actions, confirmation, status);
  } else if (showsCatalogue) {
    root.append(head, intro, newSession, list, actions, confirmation, status);
  } else {
    root.append(head, createForm, status);
  }

  const confirmationFocusTrap = createFocusTrap(confirmation, {
    doc,
    initialFocus: cancelButton,
    onEscape: () => closeDeleteConfirmation(),
  });

  function selectedRow() {
    return saveSlotsModel(state.rows, state.selectedId, state.confirmingId).selected;
  }

  function setStatus(tone, message) {
    state.statusTone = tone;
    state.statusText = message;
    status.textContent = message;
    status.className = 'save-slots-status' + (tone ? ' ' + tone : '');
    status.hidden = !message;
  }

  function closeDeleteConfirmation() {
    state.confirmingId = null;
    render();
    confirmationFocusTrap.release();
  }

  function captureAvailable() {
    return state.captureAvailable == null
      ? !!canCapture()
      : state.captureAvailable;
  }

  function render() {
    const model = saveSlotsModel(state.rows, state.selectedId, state.confirmingId);
    list.replaceChildren();
    if (model.slots.length === 0) {
      const empty = node(doc, 'li', 'save-slots-empty');
      empty.textContent = t('server.save_slots.empty');
      list.appendChild(empty);
    } else {
      for (const row of model.slots) {
        const displayName = row.kind === 'autosave'
          ? t('server.save_slots.autosave')
          : row.displayName;
        const item = node(doc, 'li', 'save-slot-row');
        const button = node(doc, 'button', 'save-slot-select' + (row.selected ? ' selected' : ''));
        button.type = 'button';
        button.dataset.saveAction = 'select';
        button.dataset.slotId = row.slotId;
        button.setAttribute('aria-pressed', row.selected ? 'true' : 'false');
        button.setAttribute('aria-label', t('server.save_slots.select', { name: displayName }));

        const name = node(doc, 'span', 'save-slot-name');
        name.textContent = displayName;
        const detail = node(doc, 'span', 'save-slot-detail');
        detail.textContent = t('server.save_slots.row_detail', {
          scenario: row.scenario || t('server.save_slots.unknown_scenario'),
          tick: row.captureTick || t('server.save_slots.unknown_tick'),
        });
        const compatibilityTone = row.refusalKind === 'content-pending'
          ? 'pending'
          : row.compatible
            ? 'compatible'
            : 'incompatible';
        const compatibility = node(
          doc,
          'span',
          'save-slot-compatibility ' + compatibilityTone,
        );
        compatibility.textContent = t(row.compatibilityLabelId);
        button.append(name, detail, compatibility);
        if (row.metadataLabelId) {
          const metadata = node(doc, 'span', 'save-slot-metadata');
          metadata.textContent = t(row.metadataLabelId);
          button.appendChild(metadata);
        }
        item.appendChild(button);
        list.appendChild(item);
      }
    }

    const selected = model.selected;
    actions.hidden = !selected;
    confirmation.hidden = !state.confirmingId;
    if (selected) {
      renameInput.value = selected.kind === 'autosave'
        ? t('server.save_slots.autosave')
        : selected.displayName;
      renameInput.disabled = !selected.canRename;
      renameButton.disabled = !selected.canRename;
      exportButton.disabled = false;
      startButton.disabled = !selected.canStart || state.pendingStart;
      deleteButton.disabled = false;
    }
    actions.setAttribute('aria-busy', state.pendingStart ? 'true' : 'false');
    if (state.confirmingId) {
      const deleting = model.slots.find((row) => row.slotId === state.confirmingId);
      confirmText.textContent = t('server.save_slots.delete_confirm', {
        name: deleting
          ? (deleting.kind === 'autosave' ? t('server.save_slots.autosave') : deleting.displayName)
          : state.confirmingId,
      });
    }

    const capture = captureAvailable();
    createForm.setAttribute('aria-busy', state.pendingCapture ? 'true' : 'false');
    createInput.disabled = !capture || state.pendingCapture;
    createButton.disabled = !capture || state.pendingCapture;
    captureHint.hidden = capture;
    setStatus(state.statusTone, state.statusText);
  }

  async function refresh() {
    await saveIdentityReady;
    if (!showsCatalogue) {
      render();
      return true;
    }
    try {
      state.rows = Array.from(await Promise.resolve(api.list()) || []);
      if (!state.rows.some((row) => normalizeSaveSlot(row).slotId === state.selectedId)) {
        state.selectedId = null;
        state.confirmingId = null;
        confirmationFocusTrap.release();
      }
      render();
      return true;
    } catch (error) {
      state.rows = [];
      state.selectedId = null;
      state.confirmingId = null;
      confirmationFocusTrap.release();
      render();
      setStatus('failed', t('server.save_slots.list_failed', { detail: errorDetail(error) }));
      return false;
    }
  }

  function actionFailure(detail) {
    setStatus('failed', t('server.save_slots.action_failed', { detail: detail || t('server.save_slots.local_failure') }));
  }

  newSession.addEventListener('click', function () {
    state.selectedId = null;
    state.confirmingId = null;
    confirmationFocusTrap.release();
    render();
    setStatus('ok', t('server.save_slots.new_session_ready'));
    onNewSession();
  });

  createButton.addEventListener('click', function () {
    const displayName = text(createInput.value).trim();
    if (!displayName) {
      setStatus('failed', t('server.save_slots.name_required'));
      createInput.focus();
      return;
    }
    try {
      const slotId = text(api.create(displayName));
      if (!slotId) {
        actionFailure('');
        return;
      }
      createInput.value = '';
      state.pendingCapture = true;
      render();
      setStatus('pending', t('server.save_slots.pending'));
    } catch (error) {
      actionFailure(errorDetail(error));
    }
  });

  list.addEventListener('click', function (event) {
    const button = event.target.closest('[data-save-action="select"]');
    if (!button || !list.contains(button)) return;
    state.selectedId = button.dataset.slotId || null;
    state.confirmingId = null;
    confirmationFocusTrap.release();
    render();
  });

  renameButton.addEventListener('click', async function () {
    const selected = selectedRow();
    if (!selected || !selected.canRename) return;
    const displayName = text(renameInput.value).trim();
    if (!displayName) {
      setStatus('failed', t('server.save_slots.name_required'));
      renameInput.focus();
      return;
    }
    try {
      const refusal = text(api.rename(selected.slotId, displayName));
      if (refusal) {
        actionFailure(refusal);
        return;
      }
      setStatus('ok', t('server.save_slots.renamed'));
      await refresh();
    } catch (error) {
      actionFailure(errorDetail(error));
    }
  });

  exportButton.addEventListener('click', function () {
    const selected = selectedRow();
    if (!selected) return;
    try {
      const refusal = text(api.export(selected.slotId));
      if (refusal) {
        actionFailure(refusal);
        return;
      }
      const artifact = text(api.takeExport());
      const name = text(api.exportName());
      if (!artifact || !download(doc, name, artifact)) {
        actionFailure(t('server.save_slots.export_missing'));
        return;
      }
      setStatus('ok', t('server.save_slots.exported'));
    } catch (error) {
      actionFailure(errorDetail(error));
    }
  });

  startButton.addEventListener('click', async function () {
    const selected = selectedRow();
    if (!selected || !selected.canStart || state.pendingStart) return;
    state.pendingStart = true;
    render();
    setStatus('pending', t('server.save_slots.starting'));
    try {
      const started = await Promise.resolve(onStart(selected));
      // Success navigates away, so keep Start disabled. Preserve the original
      // fire-and-navigate adapter contract: only explicit false is a refusal.
      if (started === false) {
        state.pendingStart = false;
        render();
        actionFailure(t('server.save_slots.local_failure'));
      }
    } catch (error) {
      state.pendingStart = false;
      render();
      actionFailure(errorDetail(error));
    }
  });

  deleteButton.addEventListener('click', function () {
    const selected = selectedRow();
    if (!selected) return;
    state.confirmingId = selected.slotId;
    render();
    confirmationFocusTrap.activate();
  });

  cancelButton.addEventListener('click', function () {
    closeDeleteConfirmation();
  });

  confirmButton.addEventListener('click', async function () {
    const slotId = state.confirmingId;
    if (!slotId) return;
    try {
      const refusal = text(api.delete(slotId, true));
      if (refusal) {
        closeDeleteConfirmation();
        actionFailure(refusal);
        return;
      }
      closeDeleteConfirmation();
      state.selectedId = null;
      setStatus('ok', t('server.save_slots.deleted'));
      await refresh();
      heading.focus();
    } catch (error) {
      closeDeleteConfirmation();
      actionFailure(errorDetail(error));
    }
  });

  async function reportOutcome(ok, detail = '') {
    state.pendingCapture = false;
    render();
    if (ok) {
      setStatus('ok', t('server.save_slots.created'));
      if (showsCatalogue) await refresh();
    } else {
      actionFailure(detail);
    }
  }

  function setCaptureAvailable(available) {
    state.captureAvailable = !!available;
    let refusal = '';
    if (!state.captureAvailable && state.pendingCapture) {
      state.pendingCapture = false;
      refusal = t('server.save_slots.capture_dropped');
      actionFailure(refusal);
      onPendingCaptureDropped(refusal);
    }
    render();
    return refusal;
  }

  function setPhase(phase) {
    return setCaptureAvailable(captureAvailableForPhase(phase));
  }

  render();
  const ready = refresh();
  return {
    refresh,
    reportOutcome,
    setCaptureAvailable,
    setPhase,
    ready,
    model: () => saveSlotsModel(state.rows, state.selectedId, state.confirmingId),
  };
}
