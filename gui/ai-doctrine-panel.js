/**
 * gui/ai-doctrine-panel.js — the debug dock's AI doctrine-pool panel
 * (issue #1149, PRD #1144).
 *
 * A JSON-driven renderer on the structured debug pipeline, following the pattern
 * `gui/station-activity-chart.js` set: a `parse*` guard, a pure
 * `build*(payload)` that returns detached DOM, and a `render*(container, json)`
 * wrapper the settings cog wires in. It parses the `AiStatePayload` the WASM
 * bridge publishes (`wasm_get_ai_doctrine`) and draws, per AI ship, the
 * scored-objective pool with every candidate's score, chosen directive and
 * resolved target — the evidence an AI tuner needs to see *why* the AI picked
 * what it picked. Nothing here talks to the simulation, so it is unit-tested in
 * jsdom without a browser or a WASM bundle.
 */

import { t } from './strings.js';

/**
 * Parse the raw bridge JSON into a payload, or `null` when there is nothing
 * renderable yet (empty string before the first publish, or malformed input).
 *
 * @param {string} json
 * @returns {object|null}
 */
export function parseAiDoctrine(json) {
  if (typeof json !== 'string' || json.length === 0) return null;
  let payload;
  try {
    payload = JSON.parse(json);
  } catch {
    return null;
  }
  if (!payload || !Array.isArray(payload.ships)) return null;
  return payload;
}

/** Format a score to one decimal place, defensively (a non-number reads `0.0`). */
function fmtScore(score) {
  const n = Number(score);
  return Number.isFinite(n) ? n.toFixed(1) : '0.0';
}

/** Build one candidate row. Pure: returns a detached `<tr>`. */
function candidateRow(candidate, chosenId, doc) {
  const row = doc.createElement('tr');
  row.className = 'ad-candidate';
  row.setAttribute('data-objective', String(candidate.id ?? ''));
  if (chosenId != null && candidate.id === chosenId) row.classList.add('chosen');
  row.setAttribute('data-score', fmtScore(candidate.score));

  const cells = [
    { cls: 'ad-c-objective', text: String(candidate.id ?? '') },
    { cls: 'ad-c-score', text: fmtScore(candidate.score) },
    { cls: 'ad-c-directive', text: String(candidate.directive ?? '') },
    { cls: 'ad-c-target', text: candidate.target != null ? String(candidate.target) : '—' },
    { cls: 'ad-c-status', text: String(candidate.status ?? '') },
  ];
  for (const { cls, text } of cells) {
    const td = doc.createElement('td');
    td.className = cls;
    td.textContent = text;
    row.appendChild(td);
  }
  // A mandatory objective is flagged for the tuner without spending a column.
  if (candidate.mandatory) row.classList.add('mandatory');
  return row;
}

/** Build one ship's section — its chosen directive plus its candidate table. */
function shipSection(ship, doc) {
  const section = doc.createElement('div');
  section.className = 'ad-ship';
  section.setAttribute('data-ship', String(ship.ship ?? ''));

  const header = doc.createElement('div');
  header.className = 'ad-ship-header';

  const name = doc.createElement('span');
  name.className = 'ad-ship-name';
  name.textContent = String(ship.ship ?? '');
  header.appendChild(name);

  const chosen = doc.createElement('span');
  chosen.className = 'ad-chosen';
  if (ship.chosen && typeof ship.chosen === 'object') {
    chosen.textContent = t('settings.debug.ai_doctrine_chosen', {
      directive: String(ship.chosen.directive ?? ''),
    });
  } else {
    chosen.classList.add('ad-chosen-none');
    chosen.textContent = t('settings.debug.ai_doctrine_none');
  }
  header.appendChild(chosen);
  section.appendChild(header);

  const table = doc.createElement('table');
  table.className = 'ad-candidates';

  const thead = doc.createElement('thead');
  const headRow = doc.createElement('tr');
  for (const labelId of [
    'settings.debug.ai_doctrine_col_objective',
    'settings.debug.ai_doctrine_col_score',
    'settings.debug.ai_doctrine_col_directive',
    'settings.debug.ai_doctrine_col_target',
    'settings.debug.ai_doctrine_col_status',
  ]) {
    const th = doc.createElement('th');
    th.textContent = t(labelId);
    headRow.appendChild(th);
  }
  thead.appendChild(headRow);
  table.appendChild(thead);

  const tbody = doc.createElement('tbody');
  const chosenId = ship.chosen && typeof ship.chosen === 'object' ? ship.chosen.id : null;
  for (const candidate of Array.isArray(ship.candidates) ? ship.candidates : []) {
    if (!candidate || typeof candidate !== 'object') continue;
    tbody.appendChild(candidateRow(candidate, chosenId, doc));
  }
  table.appendChild(tbody);
  section.appendChild(table);
  return section;
}

/**
 * Build the panel DOM from a parsed payload. Pure: returns a detached element,
 * mutates nothing.
 *
 * @param {object} payload  a parsed `AiStatePayload`
 * @param {{doc?: Document}} [opts]
 * @returns {HTMLElement}
 */
export function buildAiDoctrinePanel(payload, opts = {}) {
  const doc = opts.doc || document;

  const root = doc.createElement('div');
  root.className = 'ad-panel';

  const title = doc.createElement('div');
  title.className = 'ad-title';
  title.textContent = t('settings.debug.ai_doctrine');
  root.appendChild(title);

  const caption = doc.createElement('div');
  caption.className = 'ad-caption';
  const tick = Number.isFinite(payload.tick) ? payload.tick : 0;
  caption.textContent = t('settings.debug.ai_doctrine_tick', { tick });
  root.appendChild(caption);

  for (const ship of payload.ships) {
    if (!ship || typeof ship !== 'object') continue;
    root.appendChild(shipSection(ship, doc));
  }
  return root;
}

/**
 * Render the panel (or an empty-state placeholder) into `container` from the raw
 * bridge JSON. Clears the container first. The settings cog calls this each frame
 * while the AI doctrine-pool output is the visible one.
 *
 * @param {Element} container
 * @param {string} json  raw JSON from `wasm_get_ai_doctrine()`
 * @param {{doc?: Document}} [opts]
 */
export function renderAiDoctrinePanel(container, json, opts = {}) {
  if (!container) return;
  const doc = container.ownerDocument || opts.doc || document;
  const payload = parseAiDoctrine(json);
  container.textContent = '';
  if (!payload || payload.ships.length === 0) {
    const empty = doc.createElement('div');
    empty.className = 'ad-empty';
    empty.textContent = t('settings.debug.ai_doctrine_empty');
    container.appendChild(empty);
    return;
  }
  container.appendChild(buildAiDoctrinePanel(payload, { doc }));
}

// Expose for the classic-script bootstrap in server.html, which wires this
// renderer into the settings cog's AI doctrine output.
if (typeof window !== 'undefined') {
  window.renderAiDoctrinePanel = renderAiDoctrinePanel;
}
