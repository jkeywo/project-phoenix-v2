/**
 * The live-restore control (issues #1446 and #1447).
 *
 * A GM has already previewed and selected a candidate in the checkpoint panel
 * (#1445). This control is the one place that asks for that candidate to be
 * loaded, and it is deliberately thin: it submits the typed, attributed
 * `request_live_restore` action and then READS the answer off the projection
 * every other technical condition arrives on. It decides nothing.
 *
 * # Waiting on the room (issue #1447)
 *
 * A rewind of a session with more than one simulation peer has two waits in it,
 * and a facilitator standing at the desk has to be able to plan around both. So
 * the status line carries a COUNTDOWN in whole seconds and a count of the peers
 * still to answer, and the peer rows on the health panel carry
 * `restore_waiting` so the room can be named rather than counted. Neither is a
 * pop-up and neither is a spinner: a wait a GM cannot read is a wait that feels
 * like a hang.
 *
 * # Why the state comes from the health projection
 *
 * `src/gm_restore.rs` publishes the phase through `gm_health` (#1437) —
 * the same banner region no filter, snooze or reading hold can touch. A
 * facilitator who did not press the button still has to see that the world is
 * being replaced under them, and a second restore-only panel with its own
 * protocol vocabulary is exactly what PRD #1420 says not to build. So this
 * control shows the phase in words, and the unfilterable banner shows the same
 * fact with the same sentence.
 *
 * # Why Restore is a preview confirmation and Resume is not
 *
 * A restore discards everything that happened after the checkpoint. That is the
 * one GM decision whose consequences a crew cannot un-see, so it defaults to
 * `confirm-preview` and the preview names the concrete candidate, its tick, and
 * that current assignments are kept. Resume is the ordinary session-pause
 * action under its own existing category; there is no second resume.
 *
 * # Reading discipline (PRD #1418)
 *
 * Every phase is a sentence as well as a `data-phase`; the two buttons keep
 * their identity across every refresh so a keyboard operator is never holding a
 * replaced node; nothing here opens a dialog for routine progress, and a
 * failure is a persistent readable line rather than an interruption.
 */
import { createActionCorrelation, DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS } from './action-feedback.js';
import { GM_ACTION_REFUSAL_REASON_LABELS } from './gm-action-reasons.js';
import { candidateBlockTexts } from './gm-checkpoint-preflight.js';
import { parseGmHealthProjection } from './gm-health-banner.js';
import { slotName } from './save-slots.js';

/** The confirmation category this control selects. Registered centrally. */
export const GM_RESTORE_CONFIRMATION = Object.freeze({
  category: 'world.restore',
  defaultMode: 'confirm-preview',
});

/** Every phase `GmRestorePhase::as_wire` may report. */
export const GM_RESTORE_PHASES = Object.freeze([
  'idle', 'accepted', 'capturing-recovery', 'awaiting-readiness', 'loading',
  'awaiting-agreement', 'restored', 'rolled-back', 'failed',
]);

/** Phases in which the world is being changed and no new request may be made. */
const WORKING_PHASES = new Set([
  'accepted', 'capturing-recovery', 'awaiting-readiness', 'loading', 'awaiting-agreement',
]);

/** The String Table id naming one phase. */
export function restorePhaseLabelId(phase) {
  const known = GM_RESTORE_PHASES.includes(phase) ? phase : 'idle';
  return `server.gm.restore.phase.${known.replace(/-/g, '_')}`;
}

/**
 * Read the live-restore state out of one `gm_health` payload.
 *
 * `null` when the payload is not a health projection at all, so a malformed
 * frame leaves the last honest picture on screen. An absent `restore` field is
 * the ordinary "no restore on this desk" answer and comes back as the idle
 * state rather than as nothing, so the control can distinguish "no restore" from
 * "no news".
 */
export function parseGmRestoreState(payload) {
  const projection = parseGmHealthProjection(payload);
  if (!projection) return null;
  const row = projection.restore;
  if (!row || typeof row !== 'object') {
    return { phase: 'idle', operator: '', working: false, restoredTick: null, failure: null, paused: projection.paused };
  }
  const phase = GM_RESTORE_PHASES.includes(row.phase) ? row.phase : 'idle';
  const counted = (value) => (Number.isSafeInteger(value) && value >= 0 ? value : 0);
  return {
    phase,
    operator: typeof row.operator === 'string' ? row.operator : '',
    // The host's own answer, but never wider than this build's own reading of
    // the phase: a payload claiming a settled phase is still working would
    // otherwise disable a Resume the GM needs.
    working: row.working === true && WORKING_PHASES.has(phase),
    restoredTick: Number.isSafeInteger(row.restored_tick) ? row.restored_tick : null,
    failure: typeof row.failure === 'string' && row.failure.length > 0 ? row.failure : null,
    // The `{peers}` that failure's own sentence takes, when it has one.
    failurePeers: counted(row.failure_peers),
    // The two numbers a wait is readable by (issue #1447). A countdown is only
    // meaningful while something is actually being waited for, so an absent or
    // nonsense value is `null` rather than a zero that would read as "now".
    countdown: Number.isSafeInteger(row.countdown_seconds) && row.countdown_seconds >= 0
      ? row.countdown_seconds
      : null,
    waitingPeers: counted(row.waiting_peers),
    excludedPeers: counted(row.excluded_peers),
    paused: projection.paused,
    // Which peers, named the way the health panel names them. A GM waiting on
    // the room should not have to count rows to find out who is missing.
    waitingFor: Array.isArray(projection.peers)
      ? projection.peers.filter((peer) => peer && peer.restore_waiting === true)
        .map((peer) => peer.id).filter((id) => typeof id === 'string' && id.length > 0)
      : [],
    excludedFor: Array.isArray(projection.peers)
      ? projection.peers.filter((peer) => peer && peer.restore_excluded === true)
        .map((peer) => peer.id).filter((id) => typeof id === 'string' && id.length > 0)
      : [],
  };
}

/** The empty state, so a reset and a first paint say the same thing. */
const IDLE_STATE = Object.freeze({
  phase: 'idle',
  operator: '',
  working: false,
  restoredTick: null,
  failure: null,
  failurePeers: 0,
  countdown: null,
  waitingPeers: 0,
  excludedPeers: 0,
  paused: false,
  waitingFor: [],
  excludedFor: [],
});

export function createGmRestoreControl({
  doc = globalThis.document,
  t = (id) => id,
  getCandidate = () => null,
  getOperator = () => null,
  submitRestore = () => false,
  submitResume = () => false,
  confirmAction = (request) => request.accept(),
  correlation = createActionCorrelation,
  schedule = globalThis.setTimeout,
  cancelSchedule = globalThis.clearTimeout,
} = {}) {
  const el = (suffix) => doc && doc.getElementById(`gm-restore-${suffix}`);
  const region = doc && doc.getElementById('gm-restore');
  const heading = el('heading');
  const summary = el('summary');
  const status = el('status');
  const restoreButton = el('apply');
  const resumeButton = el('resume');

  let state = { ...IDLE_STATE };
  let pending = null;
  let timer = null;
  let localRefusal = '';

  if (region && heading) region.setAttribute('aria-labelledby', heading.id);
  if (status) {
    status.setAttribute('role', 'status');
    status.setAttribute('aria-live', 'polite');
    status.setAttribute('aria-atomic', 'true');
  }

  const candidate = () => {
    const row = getCandidate();
    return row && typeof row === 'object' && row.slotId ? row : null;
  };
  const eligible = () => {
    const row = candidate();
    return !!row && !!row.preflight && row.preflight.eligible === true;
  };
  const canRequest = () => !!getOperator() && !pending && !state.working && eligible();
  // Resume is the GM's own explicit end of a restore. It is offered exactly
  // while a restore is reported and the world is still held: never while one is
  // working (the host refuses that by name), and never as an automatic step.
  const canResume = () => !!getOperator() && !pending
    && !state.working && state.phase !== 'idle' && state.paused;

  /**
   * Mark one control pressable or not, WITHOUT taking focus off the operator.
   *
   * `aria-disabled` always tells the truth. The native `disabled` attribute is
   * withheld from a button that currently holds focus, because disabling the
   * focused node drops focus to the document body - and a rewind has five
   * working steps, so a GM who tabbed to Resume and then watched the restore
   * work would silently lose their place mid-operation (PRD #1418 story 29).
   * Both handlers already refuse when the control is not pressable, so a
   * focused, aria-disabled button is inert rather than a trap; the next render
   * after focus moves away disables it natively like any other.
   */
  function setPressable(button, pressable) {
    if (!button) return;
    button.setAttribute('aria-disabled', String(!pressable));
    const focused = doc && doc.activeElement === button;
    button.disabled = !pressable && !focused;
  }

  function setStatus(tone, message) {
    if (!status) return;
    status.dataset.tone = tone;
    status.textContent = message;
    status.hidden = message === '';
  }

  /**
   * The phase sentence, plus whatever the phase itself has to add.
   *
   * The wait sentences are appended rather than folded into the phase string so
   * that the phase a screen reader hears first is always the same words for the
   * same step, and the countdown that follows is the part that changes.
   */
  function phaseText() {
    const base = t(restorePhaseLabelId(state.phase), {
      operator: state.operator,
      tick: state.restoredTick === null ? '' : String(state.restoredTick),
    });
    const parts = [base];
    if (state.working && state.countdown !== null && state.waitingPeers > 0) {
      parts.push(t('server.gm.restore.waiting', {
        peers: String(state.waitingPeers),
        seconds: String(state.countdown),
        names: state.waitingFor.join(', '),
      }));
    }
    if (state.excludedPeers > 0) {
      parts.push(t('server.gm.restore.excluded', {
        peers: String(state.excludedPeers),
        names: state.excludedFor.join(', '),
      }));
    }
    if (state.failure) parts.push(t(state.failure, { peers: String(state.failurePeers) }));
    return parts.join(' ');
  }

  /**
   * How this control names a candidate, in the words the picker used.
   *
   * A row the ENGINE named carries a String Table id in `display_name` — the
   * recovery checkpoint a restore takes for itself is the one this control can
   * meet — so the name is resolved through the same `slotName` seam the
   * catalogue and the picker render with. A row with no name at all falls back
   * to its slot id, which is at least a handle the GM can match to the list.
   */
  const candidateName = (row) => slotName(row.displayName) || row.slotId;

  function candidateSummary() {
    const row = candidate();
    if (!row) return t('server.gm.restore.no_candidate');
    if (eligible()) {
      return t('server.gm.restore.candidate', {
        name: candidateName(row),
        tick: row.captureTick || t('server.gm.checkpoint.unknown_tick'),
      });
    }
    // Words, not a disabled button alone (#1418 story 27): a control a GM
    // cannot press must say why, in the same vocabulary the picker used.
    const reasons = candidateBlockTexts(row.preflight, t);
    return reasons.length === 0
      ? t('server.gm.restore.candidate_unchecked', { name: candidateName(row) })
      : t('server.gm.restore.candidate_blocked', {
        name: candidateName(row),
        reasons: reasons.join(' '),
      });
  }

  function render() {
    if (region) {
      region.dataset.phase = state.phase;
      // Read by the 200% layout and by tests: a countdown that exists only
      // inside a sentence cannot be styled, and a value of `0` is a real
      // countdown reading rather than an absent one.
      if (state.working && state.countdown !== null) {
        region.dataset.countdown = String(state.countdown);
        region.dataset.waiting = String(state.waitingPeers);
      } else {
        delete region.dataset.countdown;
        delete region.dataset.waiting;
      }
    }
    if (summary) summary.textContent = candidateSummary();
    setPressable(restoreButton, canRequest());
    if (resumeButton) resumeButton.hidden = false;
    setPressable(resumeButton, canResume());
    if (localRefusal) {
      setStatus('failed', localRefusal);
      return;
    }
    if (pending) {
      setStatus('pending', t('server.gm.restore.requesting'));
      return;
    }
    const tone = state.failure ? 'failed'
      : state.phase === 'restored' ? 'ok'
        : state.working ? 'pending' : '';
    setStatus(tone, state.phase === 'idle' ? '' : phaseText());
  }

  function clearPending() {
    if (timer !== null) cancelSchedule(timer);
    timer = null;
    pending = null;
  }

  /**
   * Ask for the selected candidate to be restored.
   *
   * The intent is CAPTURED before the confirmation opens and submitted
   * verbatim: the catalogue is re-read continuously, and rewriting the request
   * to whatever is selected when the dialog closes would restore a session the
   * GM never agreed to.
   */
  function request() {
    if (!canRequest()) return false;
    const operator = getOperator();
    const row = candidate();
    const captured = Object.freeze({
      action: 'request_live_restore',
      operator_id: operator.id,
      candidate: row.slotId,
      // Presentation only: the submission below carries the slot id, and this
      // name exists so the confirmation preview can say which row it means.
      name: candidateName(row),
      tick: row.captureTick || '',
    });
    const description = t('server.gm.restore.confirm', {
      name: captured.name,
      tick: captured.tick || t('server.gm.checkpoint.unknown_tick'),
    });
    let consumed = false;
    return confirmAction({
      ...GM_RESTORE_CONFIRMATION,
      intent: captured,
      description,
      // The preview separates the TECHNICAL change from what the crew have
      // already witnessed, which is the distinction PRD #1420 story 6 asks a
      // GM to weigh and which no restored value can un-see.
      preview: () => `${description} ${t('server.gm.restore.confirm_consequences')}`,
      onCancel() { consumed = true; },
      accept() {
        if (consumed) return false;
        consumed = true;
        if (pending || getOperator()?.id !== captured.operator_id) return false;
        const submission = {
          action: captured.action,
          operator_id: captured.operator_id,
          candidate: captured.candidate,
          correlation: correlation(),
        };
        let accepted = false;
        try { accepted = submitRestore(submission) !== false; } catch (_) { accepted = false; }
        if (!accepted) {
          localRefusal = t('server.gm.restore.local_refusal');
          render();
          return false;
        }
        localRefusal = '';
        pending = submission;
        render();
        timer = schedule(() => {
          timer = null;
          pending = null;
          localRefusal = t('server.gm.restore.timed_out');
          render();
        }, DEFAULT_ACTION_FEEDBACK_TIMEOUT_MS);
        return true;
      },
    }) !== false;
  }

  /** End the hold a reported restore left. The ordinary pause action. */
  function resume() {
    if (!canResume()) return false;
    let accepted = false;
    try { accepted = submitResume(correlation()) !== false; } catch (_) { accepted = false; }
    if (!accepted) {
      localRefusal = t('server.gm.restore.local_refusal');
      render();
      return false;
    }
    localRefusal = '';
    render();
    return true;
  }

  /**
   * Fold one `gm_health` payload.
   *
   * @returns whether it was accepted.
   */
  function update(payload) {
    const next = parseGmRestoreState(payload);
    if (!next) return false;
    // The host has taken the request: the local pending line hands over to the
    // authoritative phase rather than both being on screen at once.
    if (pending && next.phase !== 'idle') clearPending();
    state = next;
    if (next.phase !== 'idle') localRefusal = '';
    render();
    return true;
  }

  /**
   * Settle the request against the ONE canonical journal.
   *
   * A refusal never reaches the health projection — the request was refused, so
   * no restore ever started — and this is where the GM finds out why, in the
   * shared refusal vocabulary every other GM surface uses.
   */
  function settleJournal(entries) {
    if (!pending || !Array.isArray(entries)) return false;
    const terminal = entries.find((entry) => entry
      && entry.operator_id === pending.operator_id
      && entry.correlation === pending.correlation
      && entry.outcome !== 'pending');
    if (!terminal) return false;
    clearPending();
    if (terminal.outcome === 'refused') {
      const id = GM_ACTION_REFUSAL_REASON_LABELS[terminal.reason];
      localRefusal = t('server.gm.restore.refused', {
        reason: id ? t(id) : t('server.gm.restore.reason_unknown'),
      });
    } else {
      localRefusal = '';
    }
    render();
    return true;
  }

  function reset() {
    clearPending();
    localRefusal = '';
    state = { ...IDLE_STATE };
    render();
  }

  const onRestore = () => request();
  const onResume = () => resume();
  restoreButton?.addEventListener('click', onRestore);
  resumeButton?.addEventListener('click', onResume);
  render();

  return {
    request,
    resume,
    update,
    settleJournal,
    /** Repaint after the candidate selection moved. */
    refresh: render,
    reset,
    state: () => ({ ...state, pending: !!pending, refusal: localRefusal }),
    destroy() {
      clearPending();
      restoreButton?.removeEventListener('click', onRestore);
      resumeButton?.removeEventListener('click', onResume);
    },
  };
}
