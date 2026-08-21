// The crew's intelligence files: a compact subject list, the fact sheet behind
// each one (issue #1030), and the entries they gathered themselves (#1031).
//
// Mounted as a sibling overlay to Comms on the destroyer's tactical console —
// the destroyer has no comms console, comms is a toggle there, and this is a
// toggle beside it. The band-B evidence browser grows out of this, so the state
// contract is deliberately small: a list of dossiers in, and one piece of
// local UI state (which sheet is open) that never leaves the component.
//
// NOTHING IS DERIVED HERE. Which facts a crew may see is decided server-side by
// the dossier projection, which has no field for hidden truth at all — see
// src/dossier/projection.rs. This component formats what it was handed. It runs
// no clock, resolves no fact of its own, and there is no filtering step here
// that could be the thing keeping a secret, because there is no secret in the
// payload to keep.
//
// NO ENGLISH CROSSES THE WIRE (AGENTS.md rule 11). Every name, summary, fact
// label and text value is a strings.csv id resolved through t(). A fact's
// *value* is typed rather than pre-formatted so a percentage, a count and a
// yes/no read as themselves.
//
// strings-boot first: its top-level await delays this module's evaluation — and
// therefore this element's registration and upgrade — until the string table is
// loaded, so the constructor's template t() calls never see an empty table.
// No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import { PhElement, phDefine } from './ph-element.js';

/**
 * Provenance code → the strings.csv id for how the crew learned something.
 *
 * A literal table rather than a composed id: a composed id is invisible to the
 * string checker. The
 * codes are the four `EvidenceProvenance` names the host sends (issue #1031),
 * and an unmapped one shows no provenance rather than a machine word — the entry
 * itself still renders, so what the crew learned survives a client that is
 * behind on how they learned it.
 */
export const PROVENANCE_LABELS = Object.freeze({
  scan:      'component.dossier.provenance.scan',
  dialogue:  'component.dossier.provenance.dialogue',
  records:   'component.dossier.provenance.records',
  briefing:  'component.dossier.provenance.briefing',
});

/**
 * Render one typed fact value as the text of its row.
 *
 * The `kind` tag is the server's; a value whose kind this client does not know
 * renders as nothing at all rather than as `[object Object]`, which is the
 * failure mode a untagged pre-formatted string would have hidden.
 * @param {{kind?: string, value?: *}} value
 */
export function formatValue(value) {
  const v = value || {};
  switch (v.kind) {
    case 'text':     return v.value ? t(v.value) : '';
    case 'fraction': return `${Math.round(Math.min(1, Math.max(0, v.value || 0)) * 100)}%`;
    case 'count':    return String(v.value ?? 0);
    case 'flag':     return t(v.value ? 'component.dossier.yes' : 'component.dossier.no');
    default:         return '';
  }
}

export class PhDossierPanel extends PhElement {
  /** UUID of the open fact sheet, or null while the list is showing. */
  #openUuid = null;

  template() {
    return `
  <style>
    :host { display: flex; flex-direction: column; min-height: 0; font-family: 'JetBrains Mono', monospace; color: var(--ink); }
    :host * { box-sizing: border-box; }
    .heading { font-size: var(--text-xs); letter-spacing: 0.2em; color: var(--ink-dim); padding: 0 0.2rem 0.3rem; flex-shrink: 0; }
    .empty { font-size: var(--text-xs); color: var(--ink-dim); text-align: center; padding: 0.5rem 0; letter-spacing: 0.2em; }
    .list { display: flex; flex-direction: column; gap: 0.35rem; overflow-y: auto; min-height: 0; }
    .subject { display: flex; flex-direction: column; gap: 0.1rem; text-align: left; width: 100%;
               font-family: inherit; color: inherit; background: transparent; cursor: pointer;
               border: 1px solid var(--line-soft); border-radius: 2px; padding: 0.35rem 0.4rem; min-height: var(--control-hit-min); }
    .subject .name { font-size: var(--text-sm); letter-spacing: 0.06em; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .subject .count { font-size: var(--text-xs); color: var(--ink-dim); letter-spacing: 0.1em; }
    .sheet { display: flex; flex-direction: column; gap: 0.3rem; overflow-y: auto; min-height: 0; }
    .sheet .name { font-size: var(--text-md); letter-spacing: 0.08em; }
    .sheet .summary { font-size: var(--text-xs); color: var(--ink-dim); line-height: 1.35; }
    .fact { display: flex; align-items: baseline; gap: 0.5rem; font-size: var(--text-sm); line-height: 1.3; padding: 0.1rem 0.2rem; }
    .fact .label { flex: 1; min-width: 0; color: var(--ink-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .fact .value { flex-shrink: 0; color: var(--cyan); font-variant-numeric: tabular-nums; }
    .gathered { border-top: 1px solid var(--edge); margin-top: 0.3rem; padding-top: 0.3rem; }
    .gathered .heading { padding-bottom: 0.2rem; }
    .entry { font-size: var(--text-xs); line-height: 1.35; padding: 0.1rem 0.2rem; }
    .entry .provenance { color: var(--gold); font-size: var(--text-xs); letter-spacing: 0.12em; }
    button.back { align-self: flex-start; font-family: inherit; font-size: var(--text-xs); letter-spacing: 0.15em;
                  padding: 0.2rem 0.5rem; margin-bottom: 0.2rem; background: transparent; color: var(--ink);
                  border: 1px solid var(--line-soft); border-radius: 2px; cursor: pointer; min-height: var(--control-hit-min); }
  </style>
  <div class="heading" id="heading"></div>
  <div id="body"></div>
`;
  }

  onTemplate() {
    this.$('heading').textContent = t('component.dossier.heading');
  }

  /** The subject list as it arrived, always an array. */
  get #dossiers() {
    const s = this.state || {};
    return Array.isArray(s.dossiers) ? s.dossiers : [];
  }

  /**
   * The open fact sheet, or `null` when the list is showing.
   *
   * Resolved by UUID on every render rather than held as an object, so a sheet
   * left open while the world moves shows the current facts — and a subject that
   * leaves the world drops the operator back to the list rather than freezing a
   * stale sheet in front of them.
   */
  get open() {
    if (!this.#openUuid) return null;
    return this.#dossiers.find((d) => d.uuid === this.#openUuid) || null;
  }

  /** Open a subject's fact sheet by UUID; `null` returns to the list. */
  select(uuid) {
    this.#openUuid = uuid || null;
    this.render();
  }

  render() {
    const body = this.shadowRoot.getElementById('body');
    body.innerHTML = '';
    const open = this.open;
    if (open) {
      this.#renderSheet(body, open);
    } else {
      this.#openUuid = null;
      this.#renderList(body, this.#dossiers);
    }
  }

  /** @param {HTMLElement} body @param {Array} dossiers */
  #renderList(body, dossiers) {
    if (dossiers.length === 0) {
      const empty = document.createElement('div');
      empty.className = 'empty';
      empty.textContent = t('component.dossier.empty');
      body.appendChild(empty);
      return;
    }
    const list = document.createElement('div');
    list.className = 'list';
    dossiers.forEach((d) => {
      const row = document.createElement('button');
      row.type = 'button';
      row.className = 'subject';
      row.dataset.uuid = d.uuid || '';
      row.innerHTML = '<span class="name"></span><span class="count"></span>';
      // The name is a strings.csv id. An id with no row renders as ⟨id⟩ via
      // t()'s own miss reporting; a subject with no name id at all falls back to
      // its uuid, the way the weapons console does, so it is never invisible.
      row.children[0].textContent = d.name ? t(d.name) : (d.uuid || '');
      row.children[1].textContent = this.#countLabel(d);
      row.addEventListener('click', () => this.select(d.uuid));
      list.appendChild(row);
    });
    body.appendChild(list);
  }

  /**
   * The one-line hint under a subject's name in the list: how much is on file.
   * "Empty, not missing" is the whole point of a subject with nothing known, so
   * it says so rather than showing a bare name.
   */
  #countLabel(dossier) {
    const facts = (dossier.facts || []).length + (dossier.evidence || []).length;
    return facts === 0 ? t('component.dossier.nothing_on_file') : String(facts);
  }

  /** @param {HTMLElement} body @param {object} dossier */
  #renderSheet(body, dossier) {
    const back = document.createElement('button');
    back.type = 'button';
    back.className = 'back';
    back.id = 'dossier-back';
    back.textContent = t('component.dossier.back');
    back.addEventListener('click', () => this.select(null));
    body.appendChild(back);

    const sheet = document.createElement('div');
    sheet.className = 'sheet';

    const name = document.createElement('div');
    name.className = 'name';
    name.textContent = dossier.name ? t(dossier.name) : (dossier.uuid || '');
    sheet.appendChild(name);

    if (dossier.summary) {
      const summary = document.createElement('div');
      summary.className = 'summary';
      summary.textContent = t(dossier.summary);
      sheet.appendChild(summary);
    }

    const facts = dossier.facts || [];
    if (facts.length === 0) {
      const empty = document.createElement('div');
      empty.className = 'empty';
      empty.textContent = t('component.dossier.nothing_on_file');
      sheet.appendChild(empty);
    }
    facts.forEach((fact) => {
      const row = document.createElement('div');
      row.className = 'fact';
      row.innerHTML = '<span class="label"></span><span class="value"></span>';
      row.children[0].textContent = fact.label ? t(fact.label) : '';
      row.children[1].textContent = formatValue(fact.value);
      sheet.appendChild(row);
    });

    // What this crew GATHERED, kept visibly apart from what they were given —
    // the separation is the readout, so these never merge into the fact rows
    // above. Absent rather than empty when there is nothing: an evidence heading
    // over no entries reads as a loss.
    //
    // Order is the host's, which is gather order (issue #1031). Nothing here
    // sorts or groups by provenance: the block is the story of what the crew
    // did, in the order they did it.
    const evidence = dossier.evidence || [];
    if (evidence.length > 0) {
      const block = document.createElement('div');
      block.className = 'gathered';
      const heading = document.createElement('div');
      heading.className = 'heading';
      heading.textContent = t('component.dossier.gathered');
      block.appendChild(heading);
      evidence.forEach((entry) => {
        const row = document.createElement('div');
        row.className = 'entry';
        row.innerHTML = '<div class="text"></div><div class="provenance"></div>';
        row.children[0].textContent = entry.text ? t(entry.text) : '';
        const provenance = PROVENANCE_LABELS[entry.provenance];
        row.children[1].textContent = provenance ? t(provenance) : '';
        block.appendChild(row);
      });
      sheet.appendChild(block);
    }

    body.appendChild(sheet);
  }
}

phDefine('ph-dossier-panel', PhDossierPanel);
