/**
 * Named GM checkpoints (issue #1445).
 *
 * A Bookmark is a NAME on the ordinary save machinery. This panel asks for a
 * manual capture through exactly the API `gui/save-slots.js` uses — the same
 * `CaptureSlot::Manual` request, the same fixed-tick boundary, the same
 * peer-private browser Store — and then does one thing that surface does not:
 * it re-reads the catalogue and only claims a checkpoint exists when a row with
 * that slot id has come back carrying a capture tick.
 *
 * That read-back is the whole honesty rule. A capture tick is never taken from
 * the request, from an optimistic guess, or from the status sentence: no row,
 * no tick, and an explicit failure instead. A storage refusal, a dropped
 * boundary and a write that reported success but left nothing behind are three
 * different failures and read as three different sentences.
 *
 * The candidate list below is the shared preflight (`src/gm_checkpoint.rs`,
 * `gui/gm-checkpoint-preflight.js`) applied to this GM's OWN catalogue. It
 * restores nothing — issue #1446 owns live restore and revalidates for itself —
 * and it never reaches another peer's saves, because the browser Store it reads
 * is this browser's alone.
 *
 * Reading discipline (PRD #1418): every status is a sentence as well as a
 * `data-*` attribute, rows are buttons with arrow/Home/End movement and a
 * `--control-hit-min` floor, the selection and the name being typed survive
 * every refresh, and nothing here opens a dialog — a bookmark is routine work,
 * and a failure is a persistent, readable line, not an interruption.
 */
import {
  normalizeCandidatePreflight,
  paintCandidatePreflight,
} from './gm-checkpoint-preflight.js';
import { captureAvailableForPhase, slotName } from './save-slots.js';

const text = (value) => (value == null ? '' : String(value));

/**
 * Whether a new fixed-tick capture may be admitted, re-exported rather than
 * restated.
 *
 * A bookmark IS the save catalogue's manual capture, so "may I capture now?"
 * has to be one answer. A second copy of the rule here could disagree with the
 * one `window.__setSaveSlotsPhase` already computes from
 * `save-slots.js` — both are live on the real page, one behind this panel's
 * `setPhase` and one behind its `canCapture` — and the Bookmark control would
 * then offer a capture the catalogue knows is refused.
 */
export { captureAvailableForPhase as checkpointCaptureAvailableForPhase };

/**
 * Normalise one catalogue row into the fields a checkpoint surface needs.
 *
 * Deliberately a narrow read of the same objects `normalizeSaveSlot` reads: a
 * checkpoint panel has no business with rename, export or the delete
 * confirmation, and reading fewer fields is what keeps it from growing a second
 * catalogue UI.
 */
export function normalizeCheckpointRow(row) {
  const source = row && typeof row === 'object' ? row : {};
  const slotId = text(source.slot_id);
  return {
    slotId,
    kind: source.kind === 'autosave' ? 'autosave' : 'manual',
    displayName: text(source.display_name) || slotId,
    scenario: text(source.scenario),
    captureTick: text(source.capture_tick),
    preflight: normalizeCandidatePreflight(source.preflight),
  };
}

/**
 * Build the confirmed-capture fact for one requested bookmark.
 *
 * Mirrors `gm_checkpoint::confirmed_checkpoint`: present in the catalogue AND
 * carrying a capture tick, or nothing at all.
 */
export function confirmedCheckpoint(rows, slotId) {
  if (!slotId) return null;
  for (const raw of rows || []) {
    const row = normalizeCheckpointRow(raw);
    if (row.slotId !== slotId) continue;
    if (row.captureTick === '') return null;
    return { slotId: row.slotId, displayName: row.displayName, captureTick: row.captureTick };
  }
  return null;
}

export function createGmCheckpointPanel({
  doc = globalThis.document,
  t = (id) => id,
  api = null,
  canCapture = () => false,
  now = () => new Date(),
  onSelect = () => {},
} = {}) {
  const el = (suffix) => doc && doc.getElementById(`gm-checkpoint-${suffix}`);
  const region = doc && doc.getElementById('gm-checkpoint');
  const heading = el('heading');
  const nameInput = el('name');
  const bookmarkButton = el('bookmark');
  const hint = el('hint');
  const status = el('status');
  const list = el('list');
  const empty = el('empty');
  const summary = el('summary');
  const detail = el('detail');
  const detailName = el('detail-name');
  const detailRecord = el('detail-record');
  const detailPreflight = el('detail-preflight');

  const state = {
    rows: [],
    selectedId: null,
    pending: false,
    pendingSlotId: null,
    phaseCapturable: null,
    statusTone: '',
    statusText: '',
  };

  if (region && heading) region.setAttribute('aria-labelledby', heading.id);
  if (status) {
    status.setAttribute('role', 'status');
    status.setAttribute('aria-live', 'polite');
    status.setAttribute('aria-atomic', 'true');
  }
  if (list) list.setAttribute('role', 'list');
  if (empty) empty.textContent = t('server.gm.checkpoint.empty');
  if (hint) hint.textContent = t('server.gm.checkpoint.unavailable');

  function setStatus(tone, message) {
    state.statusTone = tone;
    state.statusText = message;
    if (!status) return;
    status.textContent = message;
    status.dataset.tone = tone;
    status.hidden = message === '';
  }

  function captureAvailable() {
    if (state.phaseCapturable != null) return state.phaseCapturable;
    try { return !!canCapture(); } catch (_) { return false; }
  }

  const rows = () => state.rows.map(normalizeCheckpointRow);
  const selected = () => rows().find((row) => row.slotId === state.selectedId) || null;

  /** slot id → the row element that has been on screen since it first appeared. */
  const rowNodes = new Map();

  function moveFocus(from, step) {
    const buttons = [...list.querySelectorAll('.gm-checkpoint-row')];
    const index = buttons.indexOf(from);
    if (index < 0) return;
    const next = step === 'home' ? 0
      : step === 'end' ? buttons.length - 1
        : Math.min(buttons.length - 1, Math.max(0, index + step));
    buttons[next]?.focus();
  }

  /**
   * Build one candidate row ONCE, keyed by its slot id.
   *
   * The catalogue is re-read on every admission refresh, which on a live GM
   * console is continuous. Rebuilding the list each time destroyed the very
   * node the operator was reaching for: a pointer press that began on a row
   * landed on nothing, because the element under the finger had been replaced
   * between the press and the release. So the node is created once and
   * repainted in place — #1418's stable target, in the only form that holds
   * under a refresh nobody asked for.
   */
  function createRow(slotId) {
    const item = doc.createElement('li');
    item.className = 'gm-checkpoint-entry';
    const button = doc.createElement('button');
    button.type = 'button';
    button.className = 'gm-checkpoint-row';
    // NOT `data-slot-id`: the landing save catalogue's rows already carry
    // that attribute for the same slot ids, and both lists are in the one
    // server.html document at once. A shared attribute made every unscoped
    // `[data-slot-id=...]` selector ambiguous — which is exactly how this
    // panel broke two existing save-catalogue smoke tests.
    button.dataset.checkpointSlotId = slotId;
    for (const className of ['gm-checkpoint-name', 'gm-checkpoint-record', 'gm-checkpoint-verdict']) {
      const span = doc.createElement('span');
      span.className = className;
      button.appendChild(span);
    }
    button.addEventListener('click', () => select(slotId));
    button.addEventListener('keydown', (event) => {
      const step = event.key === 'ArrowDown' ? 1
        : event.key === 'ArrowUp' ? -1
          : event.key === 'Home' ? 'home'
            : event.key === 'End' ? 'end' : null;
      if (step === null) return;
      event.preventDefault();
      moveFocus(button, step);
    });
    item.appendChild(button);
    return { item, button };
  }

  function paintRow(button, row) {
    // Three answers, not two. A row whose preflight is absent has not been
    // CHECKED — the landing catalogue carries none, because there is no live
    // session yet for a row to be a candidate for — and painting that as
    // "cannot hold the current assignments" states a refusal nothing decided.
    const checked = !!row.preflight;
    const eligible = checked && row.preflight.eligible;
    const verdict = t(checked
      ? (eligible ? 'server.gm.checkpoint.eligible' : 'server.gm.checkpoint.ineligible')
      : 'server.gm.checkpoint.verdict_unknown');
    button.dataset.eligible = checked ? String(eligible) : 'unknown';
    button.setAttribute('aria-pressed', String(row.slotId === state.selectedId));
    const displayName = row.kind === 'autosave'
      ? t('server.gm.checkpoint.autosave')
      : slotName(row.displayName);
    button.setAttribute('aria-label', t('server.gm.checkpoint.select', {
      name: displayName,
      verdict,
    }));
    for (const [className, value] of [
      ['gm-checkpoint-name', displayName],
      ['gm-checkpoint-record', t('server.gm.checkpoint.row_detail', {
        scenario: row.scenario || t('server.gm.checkpoint.unknown_scenario'),
        tick: row.captureTick || t('server.gm.checkpoint.unknown_tick'),
      })],
      // Words, not only `data-eligible`: the verdict has to survive a forced
      // colour mode that flattens the row's styling.
      ['gm-checkpoint-verdict', verdict],
    ]) {
      const span = button.querySelector(`.${className}`);
      if (span && span.textContent !== value) span.textContent = value;
    }
  }

  function paintDetail() {
    const row = selected();
    if (detail) detail.hidden = !row;
    if (!row) {
      for (const node of [detailName, detailRecord]) if (node) node.textContent = '';
      if (detailPreflight) {
        detailPreflight.replaceChildren();
        detailPreflight.removeAttribute('data-eligible');
      }
      return;
    }
    if (detailName) {
      detailName.textContent = row.kind === 'autosave'
        ? t('server.gm.checkpoint.autosave')
        : slotName(row.displayName);
    }
    if (detailRecord) {
      detailRecord.textContent = t('server.gm.checkpoint.row_detail', {
        scenario: row.scenario || t('server.gm.checkpoint.unknown_scenario'),
        tick: row.captureTick || t('server.gm.checkpoint.unknown_tick'),
      });
    }
    paintCandidatePreflight(detailPreflight, row.preflight, { doc, t });
  }

  function render() {
    const model = rows();
    if (list) {
      // Reading position and focus belong to the operator, not to the refresh:
      // a row that is still in the catalogue keeps the element it already had,
      // so focus, the pointer's target and any in-flight press all survive.
      const present = new Set();
      model.forEach((row, index) => {
        present.add(row.slotId);
        let node = rowNodes.get(row.slotId);
        if (!node) {
          node = createRow(row.slotId);
          rowNodes.set(row.slotId, node);
        }
        paintRow(node.button, row);
        if (list.children[index] !== node.item) {
          list.insertBefore(node.item, list.children[index] || null);
        }
      });
      for (const [slotId, node] of [...rowNodes]) {
        if (present.has(slotId)) continue;
        node.item.remove();
        rowNodes.delete(slotId);
      }
    }
    if (empty) empty.hidden = model.length !== 0;
    if (summary) {
      // The count is over rows that were actually CHECKED. An unchecked row is
      // neither eligible nor refused, so folding it into the denominator would
      // report a shortfall the preflight never found; it is counted in its own
      // sentence instead, and only when there is one to count.
      const checked = model.filter((row) => !!row.preflight);
      const unchecked = model.length - checked.length;
      const counted = t('server.gm.checkpoint.summary', {
        eligible: String(checked.filter((row) => row.preflight.eligible).length),
        total: String(checked.length),
      });
      summary.textContent = unchecked === 0
        ? counted
        : `${counted} ${t('server.gm.checkpoint.summary_unchecked', {
          unchecked: String(unchecked),
        })}`;
    }
    if (state.selectedId && !model.some((row) => row.slotId === state.selectedId)) {
      state.selectedId = null;
    }
    const capture = captureAvailable();
    if (nameInput) nameInput.disabled = !capture || state.pending;
    if (bookmarkButton) bookmarkButton.disabled = !capture || state.pending;
    if (hint) hint.hidden = capture;
    if (region) region.setAttribute('aria-busy', state.pending ? 'true' : 'false');
    paintDetail();
  }

  function select(slotId) {
    const next = rows().some((row) => row.slotId === slotId) ? slotId : null;
    if (next === state.selectedId) return next !== null;
    state.selectedId = next;
    render();
    // The live-restore control (issue #1446) reads THIS selection, so it is
    // told when it moves rather than polling a panel it does not own.
    try { onSelect(selected()); } catch (_) { /* a listener must not break selection. */ }
    return next !== null;
  }

  async function refresh() {
    if (!api || typeof api.list !== 'function') {
      state.rows = [];
      render();
      return false;
    }
    try {
      state.rows = Array.from(await Promise.resolve(api.list()) || []);
      render();
      return true;
    } catch (error) {
      state.rows = [];
      render();
      setStatus('failed', t('server.gm.checkpoint.list_failed', {
        detail: errorDetail(error),
      }));
      return false;
    }
  }

  function errorDetail(error) {
    if (error && typeof error.message === 'string' && error.message) return error.message;
    return text(error) || t('server.gm.checkpoint.local_failure');
  }

  function bookmark() {
    if (!api || typeof api.create !== 'function') return false;
    const displayName = text(nameInput && nameInput.value).trim();
    if (!displayName) {
      setStatus('failed', t('server.gm.checkpoint.name_required'));
      nameInput?.focus();
      return false;
    }
    if (!captureAvailable() || state.pending) return false;
    let slotId = '';
    try {
      slotId = text(api.create(displayName));
    } catch (error) {
      setStatus('failed', t('server.gm.checkpoint.failed', { detail: errorDetail(error) }));
      return false;
    }
    if (!slotId) {
      setStatus('failed', t('server.gm.checkpoint.failed', {
        detail: t('server.gm.checkpoint.local_failure'),
      }));
      return false;
    }
    state.pending = true;
    state.pendingSlotId = slotId;
    if (nameInput) nameInput.value = '';
    render();
    // Pending is pending. No tick, no time, no name in past tense.
    setStatus('pending', t('server.gm.checkpoint.pending', { name: displayName }));
    return true;
  }

  /**
   * Settle a requested bookmark against what actually reached storage.
   *
   * `ok` is the Store's own answer for THIS capture; the catalogue read that
   * follows is what turns it into a confirmed checkpoint. A success that leaves
   * no readable row is reported as a failure, because a GM who is told a
   * checkpoint exists must be able to select it afterwards.
   */
  async function reportOutcome(ok, detail = '') {
    if (!state.pending) return false;
    const slotId = state.pendingSlotId;
    state.pending = false;
    state.pendingSlotId = null;
    if (!ok) {
      render();
      setStatus('failed', t('server.gm.checkpoint.failed', {
        detail: text(detail) || t('server.gm.checkpoint.local_failure'),
      }));
      return false;
    }
    await refresh();
    const confirmed = confirmedCheckpoint(state.rows, slotId);
    if (!confirmed) {
      setStatus('failed', t('server.gm.checkpoint.failed', {
        detail: t('server.gm.checkpoint.unconfirmed'),
      }));
      return false;
    }
    state.selectedId = confirmed.slotId;
    render();
    setStatus('ok', t('server.gm.checkpoint.confirmed', {
      name: confirmed.displayName,
      tick: confirmed.captureTick,
      time: localTime(),
    }));
    return true;
  }

  /** Private, presentation-only: never enters a save, a digest or the wire. */
  function localTime() {
    let stamp = null;
    try { stamp = now(); } catch (_) { stamp = null; }
    if (!(stamp instanceof Date) || Number.isNaN(stamp.getTime())) {
      return t('server.gm.checkpoint.unknown_time');
    }
    try { return stamp.toLocaleTimeString(); } catch (_) { return stamp.toISOString(); }
  }

  function setCaptureAvailable(available) {
    state.phaseCapturable = !!available;
    let refusal = '';
    if (!state.phaseCapturable && state.pending) {
      state.pending = false;
      state.pendingSlotId = null;
      refusal = t('server.gm.checkpoint.capture_dropped');
      setStatus('failed', t('server.gm.checkpoint.failed', { detail: refusal }));
    }
    render();
    return refusal;
  }

  function setPhase(phase) {
    return setCaptureAvailable(captureAvailableForPhase(phase));
  }

  function reset() {
    state.rows = [];
    state.selectedId = null;
    state.pending = false;
    state.pendingSlotId = null;
    if (nameInput) nameInput.value = '';
    render();
    setStatus('', '');
  }

  const onBookmark = () => bookmark();
  bookmarkButton?.addEventListener('click', onBookmark);
  render();
  const ready = refresh();

  return {
    bookmark,
    refresh,
    reportOutcome,
    select,
    setCaptureAvailable,
    setPhase,
    reset,
    ready,
    state: () => ({
      rows: rows(),
      selected: selected(),
      pending: state.pending,
      statusTone: state.statusTone,
      statusText: state.statusText,
    }),
    destroy: () => bookmarkButton?.removeEventListener('click', onBookmark),
  };
}
