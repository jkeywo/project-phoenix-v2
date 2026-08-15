// External-operation readout and control (issues #1026, #1027).
//
// Sits under <ph-deadline-list> on the captain console: deadlines are what the
// mission is doing to the crew, operations are what the crew are doing to the
// world. One ship runs at most one operation, so this is a single row plus a
// control, not a list.
//
// NOTHING HERE KNOWS WHAT A VERB IS. The five verbs (#1027 completed the set)
// reach this component as `{verb, label}` pairs on the blackboard and are
// rendered by walking that list — there is no per-verb branch, no verb-specific
// icon table and no special case for the one that tows. A sixth verb authored
// tomorrow appears in the picker with no change here, which is the test at the
// bottom of tests/client/ph-operation-panel.test.js.
//
// THE PICKER EXISTS BECAUSE THERE ARE NOW FIVE. Up to #1026 a hull offered one
// verb and the button ordered it; a tender that can tow, escort, transfer and
// field-repair needs the crew to say which. A hull offering exactly one verb
// still shows no picker, so the single-capability console is unchanged.
//
// THE BAR IS NOT CLIENT-SIDE. `progress` arrives already computed server-side
// off ELIGIBLE ticks and is re-published every tick, so this component paints a
// number it was handed and runs no timer of its own. That matters more here
// than it does for a countdown: a stalled operation's bar is meant to sit
// visibly still — that stillness is how the crew notice they have drifted out
// of range — and a client interpolating its own progress would animate straight
// through the interruption.
//
// NO ENGLISH CROSSES THE WIRE (AGENTS.md rule 11). The verb, the target and the
// stall/failure reason all arrive as strings.csv ids and are resolved here. The
// state word is the one exception, and only in shape: the payload carries a
// machine code and the map below turns it into a literal id, so
// scripts/check-strings.mjs can still see every id this file can render.
//
// strings-boot first: its top-level await delays this module's evaluation — and
// therefore this element's registration and upgrade — until the string table is
// loaded, so the constructor's template t() calls never see an empty table.
// No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import { phAdoptConsoleStyles } from './ph-console-styles.js';

/**
 * Machine state code → the strings.csv id for the word shown beside the bar.
 *
 * A literal table rather than a composed `'component.operations.state.' + code`
 * because a composed id is invisible to the string checker, and a state that
 * shipped with no row behind it would render as a raw `⟨id⟩` to the crew.
 */
export const STATE_LABELS = Object.freeze({
  holding:   'component.operations.state.holding',
  stalled:   'component.operations.state.stalled',
  completed: 'component.operations.state.completed',
  aborted:   'component.operations.state.aborted',
  failed:    'component.operations.state.failed',
});

/** States in which the operation is over and the bar should stop reading live. */
const SETTLED = new Set(['completed', 'aborted', 'failed']);

export class PhOperationPanel extends HTMLElement {
  #state = null;

  /**
   * Which of the hull's verbs the crew have selected, or `null` for "whichever
   * the hull offers first". Purely local: the server is never told what a
   * console is *about* to order.
   */
  #chosenVerb = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    // Every component adopts the shared control family (module 1 of PRD
    // #1023): custom properties cross a shadow boundary, class rules do not.
    phAdoptConsoleStyles(this.shadowRoot);
    const tpl = document.createElement('template');
    tpl.innerHTML = `
  <style>
    :host { display: block; font-family: 'JetBrains Mono', monospace; color: var(--ink); }
    :host * { box-sizing: border-box; }
    .heading { font-size: var(--text-xs); letter-spacing: 0.2em; color: var(--ink-dim); padding: 0 0.2rem 0.3rem; }
    .empty { font-size: var(--text-xs); color: var(--ink-dim); text-align: center; padding: 0.5rem 0; letter-spacing: 0.2em; }
    .row { display: flex; align-items: baseline; gap: 0.5rem; font-size: var(--text-sm); line-height: 1.3; padding: 0.1rem 0.2rem; }
    .row .verb { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .row .state { flex-shrink: 0; letter-spacing: 0.1em; color: var(--cyan); }
    .track { height: 6px; border-radius: 3px; background: var(--line-soft); overflow: hidden; margin: 0.25rem 0.2rem; }
    .fill { height: 100%; width: 0; background: var(--cyan); }
    .reason { font-size: var(--text-xs); color: var(--reloading); padding: 0 0.2rem 0.2rem; letter-spacing: 0.05em; }
    .slowed { font-size: var(--text-xs); color: var(--reloading); padding: 0 0.2rem 0.2rem; letter-spacing: 0.05em; }
    select { width: 100%; font-family: inherit; font-size: var(--text-xs); letter-spacing: 0.1em;
             padding: 0.3rem; margin-top: 0.2rem; background: transparent; color: var(--ink);
             border: 1px solid var(--line-soft); border-radius: 2px; }
    :host([data-state="stalled"]) .fill { background: var(--reloading); }
    :host([data-state="stalled"]) .state { color: var(--reloading); }
    :host([data-state="failed"]) .fill,
    :host([data-state="failed"]) .state { background: none; color: var(--ink-dim); }
    :host([data-state="failed"]) .fill { background: var(--ink-dim); }
    :host([data-state="completed"]) .fill { background: var(--loaded); }
    :host([data-state="aborted"]) .fill { background: var(--ink-dim); }
    button { width: 100%; font-family: inherit; font-size: var(--text-xs); letter-spacing: 0.15em;
             padding: 0.35rem; margin-top: 0.2rem; background: transparent; color: var(--ink);
             border: 1px solid var(--line-soft); border-radius: 2px; cursor: pointer; }
    button:disabled { color: var(--ink-dim); cursor: default; }
  </style>
  <div class="heading" id="heading"></div>
  <div id="body"></div>
  <div class="slowed" id="slowed" hidden></div>
  <div class="reason" id="reason" hidden></div>
  <select id="verb" hidden></select>
  <button id="action" type="button" hidden></button>
`;
    this.shadowRoot.appendChild(tpl.content.cloneNode(true));
    this.shadowRoot.getElementById('heading').textContent = t('component.operations.heading');
    this.shadowRoot.getElementById('action').addEventListener('click', () => this.#onAction());
    const verb = this.shadowRoot.getElementById('verb');
    verb.setAttribute('aria-label', t('component.operations.verb_picker'));
    // The chosen verb is UI state, not sim state: it is what the crew are about
    // to order, and nothing on the wire has an opinion about it until they
    // press the button. Re-rendering on change keeps the button's label and the
    // action it would send in step with the selection.
    verb.addEventListener('change', () => {
      this.#chosenVerb = verb.value || null;
      this.#render();
    });
  }

  set state(val) {
    this.#state = val;
    this.#render();
  }

  get state() { return this.#state; }

  /**
   * The console action this panel would send for its current state, or `null`
   * when its button is not live. Exposed so the host page forwards exactly what
   * the panel decided rather than re-deriving it.
   */
  get action() {
    const ops = (this.#state && this.#state.operations) || {};
    const active = ops.active || null;
    if (active && !SETTLED.has(active.state || '')) {
      return { action: 'abort_operation' };
    }
    const capabilities = Array.isArray(ops.capabilities) ? ops.capabilities : [];
    // The selection, or the hull's first verb when nothing has been chosen —
    // which is exactly the #1026 behaviour for a hull that offers one verb. A
    // selection the hull no longer offers (a save resumed onto a different
    // ship, a capability withdrawn) falls back rather than sending a verb the
    // server would refuse by name.
    const chosen =
      capabilities.find((c) => c && c.verb === this.#chosenVerb) || capabilities[0];
    const target = (this.#state && this.#state.target_uuid) || null;
    if (!chosen || !target) return null;
    return { action: 'start_operation', verb: chosen.verb, target_uuid: target };
  }

  connectedCallback() {
    // The console-core wiring pattern every ph-* control uses: the host page
    // publishes `window.sendAction` and repairs this reference after upgrade.
    this.sendAction ??= window.sendAction;
  }

  #onAction() {
    const action = this.action;
    if (!action || !this.sendAction) return;
    const { action: name, ...payload } = action;
    this.sendAction(name, payload);
  }

  #render() {
    const ops = (this.#state && this.#state.operations) || {};
    const capabilities = Array.isArray(ops.capabilities) ? ops.capabilities : [];
    const active = ops.active || null;
    const body = this.shadowRoot.getElementById('body');
    const reasonEl = this.shadowRoot.getElementById('reason');
    const slowedEl = this.shadowRoot.getElementById('slowed');
    const picker = this.shadowRoot.getElementById('verb');
    const button = this.shadowRoot.getElementById('action');

    // A hull that can perform nothing gets a panel that says so, rather than an
    // empty box the crew have to interpret.
    if (capabilities.length === 0 && !active) {
      this.removeAttribute('data-state');
      body.innerHTML = `<div class="empty">${t('component.operations.none')}</div>`;
      reasonEl.hidden = true;
      slowedEl.hidden = true;
      picker.hidden = true;
      button.hidden = true;
      return;
    }

    if (!active) {
      this.removeAttribute('data-state');
      body.innerHTML = `<div class="empty">${t('component.operations.idle')}</div>`;
    } else {
      const state = active.state || 'holding';
      this.setAttribute('data-state', state);
      const pct = Math.round(Math.min(1, Math.max(0, active.progress ?? 0)) * 100);
      const verb = active.verb_label ? t(active.verb_label) : (active.verb || '');
      const target = active.target_name ? t(active.target_name) : (active.target_uuid || '');
      const stateLabel = STATE_LABELS[state]
        ? t(STATE_LABELS[state])
        : state.toUpperCase();
      body.innerHTML = `
        <div class="row">
          <span class="verb"></span>
          <span class="state"></span>
        </div>
        <div class="track"><div class="fill"></div></div>`;
      body.querySelector('.verb').textContent = target ? `${verb} — ${target}` : verb;
      body.querySelector('.state').textContent = stateLabel;
      body.querySelector('.fill').style.width = `${pct}%`;
    }

    // A hazard band stretching the work (issue #1027). Shown only when the rate
    // is below normal, because a bar labelled "100%" on every ordinary
    // operation is noise — and a bar crawling with NOTHING beside it reads as a
    // bug rather than as the storm, which is the whole reason the rate is on
    // the wire at all.
    const rate = active && Number.isFinite(active.rate_percent)
      ? active.rate_percent
      : 100;
    const slowed = !!active && !SETTLED.has(active.state || '') && rate < 100;
    slowedEl.hidden = !slowed;
    if (slowed) {
      slowedEl.textContent = t('component.operations.slowed').replace('{rate}', String(rate));
    }

    // A refusal (no operation was ever opened) and a stall reason (one was, and
    // it is not advancing) are different things; the crew act on them
    // differently, so only one is ever shown and the live one wins.
    const reason = (active && active.reason) || ops.refusal || null;
    reasonEl.hidden = !reason;
    if (reason) reasonEl.textContent = t(reason);

    const action = this.action;
    button.hidden = !action;
    if (action) {
      button.textContent = action.action === 'abort_operation'
        ? t('component.operations.abort')
        : t('component.operations.start');
    }

    // The verb picker. Offered only when there is a choice to make and only
    // when the button would start something — a running operation has one verb
    // and it is already decided. Rebuilt from the wire's own capability list,
    // so a hull that gains a verb gains an option here with no change to this
    // file.
    const choosing = capabilities.length > 1 && action
      && action.action === 'start_operation';
    picker.hidden = !choosing;
    if (choosing) {
      const selected = action.verb;
      const rendered = capabilities.map((c) => c && c.verb).join(' ');
      if (picker.dataset.rendered !== rendered) {
        picker.textContent = '';
        for (const capability of capabilities) {
          if (!capability || !capability.verb) continue;
          const option = document.createElement('option');
          option.value = capability.verb;
          option.textContent = capability.label ? t(capability.label) : capability.verb;
          picker.appendChild(option);
        }
        picker.dataset.rendered = rendered;
      }
      picker.value = selected;
    }
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-operation-panel')) {
  customElements.define('ph-operation-panel', PhOperationPanel);
}
