// Science scan readout and control (issue #1032).
//
// Sits under <ph-operation-panel> on the captain console: operations are what
// the crew are doing to the world, a scan is what they are finding out about
// it. One ship holds one reading, so this is a header, a short list of rows and
// a button — not a log.
//
// NOTHING HERE KNOWS WHAT A STRUCTURE IS. Every row is a `(label, value)` pair
// the server derived from the subject's own condition track and labelled with a
// `strings.csv` id the scenario authored beside the number. There is no
// per-structure branch, no table of known depots, and — the point of the whole
// slice — no authored scan prose to render: the payload carries no field for
// one (see `system_blackboard_scan_round_trips_and_carries_no_field_for_authored_prose`
// in src/core/codec.rs). A structure invented tomorrow reads out here with no
// change to this file.
//
// THE READING IS NOT LIVE. It is stamped with the tick it was taken on and does
// not update as the structure changes: what the crew have is what they saw when
// they looked. That is why the fidelity line says how precise it is, and why
// the button stays available — scanning again from closer in is the move.
//
// NO ENGLISH CROSSES THE WIRE (AGENTS.md rule 11). The subject name, the band
// name, every row label and the refusal reason all arrive as strings.csv ids
// and are resolved here.
//
// strings-boot first: its top-level await delays this module's evaluation — and
// therefore this element's registration and upgrade — until the string table is
// loaded, so the constructor's template t() calls never see an empty table.
// No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';

/**
 * Whole-percent condition, rendered at the precision the answering band bought.
 *
 * A band that reports in quarters shows `50%`; one that reports whole percent
 * shows `62%`. Digits and a percent sign only, so nothing here needs a
 * strings.csv row.
 * @param {number} fraction
 * @param {number} step
 */
export function formatCondition(fraction, step) {
  const pct = Math.min(100, Math.max(0, (Number(fraction) || 0) * 100));
  // One decimal place only where the band is finer than a whole percent;
  // a quarter-step band would otherwise imply a precision it does not have.
  const decimals = Number(step) > 0 && Number(step) < 0.01 ? 1 : 0;
  return `${pct.toFixed(decimals)}%`;
}

/**
 * The `±` half-width of a reading taken at `step`, as whole percent, or `null`
 * when the step is too fine to be worth stating.
 * @param {number} step
 */
export function formatTolerance(step) {
  const half = (Number(step) || 0) * 100 / 2;
  return half >= 1 ? `±${Math.round(half)}%` : null;
}

export class PhScanReadout extends HTMLElement {
  #state = null;

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    const tpl = document.createElement('template');
    tpl.innerHTML = `
  <style>
    :host { display: block; font-family: 'JetBrains Mono', monospace; color: var(--ink); }
    :host * { box-sizing: border-box; }
    .heading { font-size: 0.6rem; letter-spacing: 0.2em; color: var(--ink-dim); padding: 0 0.2rem 0.3rem; }
    .empty { font-size: 0.65rem; color: var(--ink-dim); text-align: center; padding: 0.5rem 0; letter-spacing: 0.2em; }
    .subject { display: flex; align-items: baseline; gap: 0.5rem; font-size: 0.7rem; padding: 0.1rem 0.2rem; }
    .subject .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .subject .band { flex-shrink: 0; letter-spacing: 0.1em; color: var(--cyan); }
    .rows { display: flex; flex-direction: column; gap: 0.2rem; padding: 0.15rem 0.2rem 0; }
    .row { display: flex; align-items: baseline; gap: 0.5rem; font-size: 0.66rem; line-height: 1.3; }
    .row .label { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--ink-dim); }
    .row .value { flex-shrink: 0; font-variant-numeric: tabular-nums; }
    .row.down .value { color: var(--warn, var(--ink-dim)); }
    .tolerance { color: var(--ink-dim); font-size: 0.9em; margin-left: 0.3rem; }
    .reason { font-size: 0.62rem; color: var(--warn, var(--ink-dim)); padding: 0.2rem; letter-spacing: 0.05em; }
    button { width: 100%; font-family: inherit; font-size: 0.65rem; letter-spacing: 0.15em;
             padding: 0.35rem; margin-top: 0.25rem; background: transparent; color: var(--ink);
             border: 1px solid var(--line-soft); border-radius: 2px; cursor: pointer; }
    button:disabled { color: var(--ink-dim); cursor: default; }
  </style>
  <div class="heading" id="heading"></div>
  <div id="body"></div>
  <div class="reason" id="reason" hidden></div>
  <button id="action" type="button" hidden></button>
`;
    this.shadowRoot.appendChild(tpl.content.cloneNode(true));
    this.shadowRoot.getElementById('heading').textContent = t('component.scan.heading');
    this.shadowRoot.getElementById('action').addEventListener('click', () => this.#onAction());
  }

  set state(val) {
    this.#state = val;
    this.#render();
  }

  get state() { return this.#state; }

  /**
   * The console action this panel would send, or `null` when its button is not
   * live. Exposed so the host page forwards exactly what the panel decided
   * rather than re-deriving it.
   */
  get action() {
    const scan = (this.#state && this.#state.scan) || {};
    const uuid = (this.#state && this.#state.target_uuid) || null;
    if (!scan.capable || !uuid) return null;
    return { action: 'scan_target', uuid };
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
    const scan = (this.#state && this.#state.scan) || {};
    const reading = scan.reading || null;
    const body = this.shadowRoot.getElementById('body');
    const reasonEl = this.shadowRoot.getElementById('reason');
    const button = this.shadowRoot.getElementById('action');

    // A hull whose sensor suite cannot take a reading says so, rather than
    // showing an empty box the crew have to interpret.
    if (!scan.capable) {
      body.innerHTML = `<div class="empty">${t('component.scan.none')}</div>`;
      reasonEl.hidden = true;
      button.hidden = true;
      return;
    }

    if (!reading) {
      body.innerHTML = `<div class="empty">${t('component.scan.idle')}</div>`;
    } else {
      body.textContent = '';
      const subject = document.createElement('div');
      subject.className = 'subject';
      subject.innerHTML = '<span class="name"></span><span class="band"></span>';
      subject.firstChild.textContent = reading.subject_name
        ? t(reading.subject_name)
        : (reading.subject_uuid || '');
      subject.lastChild.textContent = reading.band_label
        ? t(reading.band_label)
        : (reading.band || '');
      body.appendChild(subject);

      const rows = document.createElement('div');
      rows.className = 'rows';
      // Condition first — it is the number the whole reading is about — then
      // the operational flags, then the capacities, in the order the server
      // derived them. Nothing here re-sorts: the order is the subject's own
      // authored order, so two consoles read the same sheet.
      rows.appendChild(this.#row(
        t('component.scan.condition'),
        formatCondition(reading.condition_fraction, reading.condition_step),
        false,
        formatTolerance(reading.condition_step),
      ));
      for (const entry of reading.flags || []) {
        const [label, held] = entry;
        rows.appendChild(this.#row(
          t(label),
          t(held ? 'component.scan.flag.held' : 'component.scan.flag.down'),
          !held,
        ));
      }
      for (const entry of reading.capacities || []) {
        const [label, amount] = entry;
        rows.appendChild(this.#row(t(label), String(amount), false));
      }
      body.appendChild(rows);
    }

    // A refusal and a reading are never both current: the server clears one
    // when it sets the other, so this shows whichever arrived.
    reasonEl.hidden = !scan.refusal;
    if (scan.refusal) reasonEl.textContent = t(scan.refusal);

    const action = this.action;
    button.hidden = false;
    button.disabled = !action;
    button.textContent = t('component.scan.take');
  }

  /**
   * One `(label, value)` line.
   * @param {string} label  already-resolved text
   * @param {string} value  already-resolved text
   * @param {boolean} down  render as a fault
   * @param {string|null} [tolerance]  already-resolved `±n%`, or null
   */
  #row(label, value, down, tolerance) {
    const el = document.createElement('div');
    el.className = down ? 'row down' : 'row';
    const labelEl = document.createElement('span');
    labelEl.className = 'label';
    labelEl.textContent = label;
    const valueEl = document.createElement('span');
    valueEl.className = 'value';
    valueEl.textContent = value;
    if (tolerance) {
      const tol = document.createElement('span');
      tol.className = 'tolerance';
      tol.textContent = tolerance;
      valueEl.appendChild(tol);
    }
    el.appendChild(labelEl);
    el.appendChild(valueEl);
    return el;
  }
}

if (typeof window !== 'undefined' && !customElements.get('ph-scan-readout')) {
  customElements.define('ph-scan-readout', PhScanReadout);
}
