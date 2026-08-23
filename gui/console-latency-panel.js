/**
 * gui/console-latency-panel.js — the debug dock's console input-to-feedback
 * latency table (issue #1169, PRD #1144).
 *
 * Parses the `ConsoleLatencyPayload` the WASM bridge publishes
 * (`wasm_get_console_latency`) and draws one row per (surface, action) with its
 * p50 / p75 / max for each measured segment.
 *
 * ## Reading the table
 *
 * The surface groups answer different questions and are NOT comparable as one
 * number, which is why the table shows them apart rather than averaging them:
 *
 * - **Phone** — a real player's whole round trip over the WebRTC bridge. The
 *   number the ~100 ms polish bar is actually about.
 * - **Host page** — the same measurement with no network in it: the host page's
 *   own consoles reach the simulation in-process. The floor the phone row is
 *   read against.
 * - **Simulation** — the host's own admission→broadcast service window. The
 *   slice of the phone's round trip this repository is answerable for; whatever
 *   the phone row exceeds it by is transport and client work.
 *
 * A blank cell means that segment does not exist on that path — the host cannot
 * see a player's input event and a client cannot see the host's schedule — not
 * that it measured zero.
 *
 * Nothing here talks to the simulation: it is a pure function of the payload, so
 * it is unit-tested in jsdom without a browser or a WASM bundle, following the
 * `parse*` / `build*` / `render*` renderer pattern `gui/station-activity-chart.js`
 * established for this pipeline.
 *
 * Prose (title, caption, empty state, surface labels) goes through `t()`. The
 * per-row action ids and the p50/p75/max column heads are machine values, which
 * is why this file sits in `check-strings.mjs`'s UNLOCALISED_FILES beside
 * `gui/debug-overlays.js` — the same operator-diagnostic category.
 */

import { t } from './strings.js';

/** `LatencySurface` variant name → the string id naming it in the table. */
const SURFACE_LABELS = Object.freeze({
  PhoneConsole: 'settings.debug.console_latency_phone',
  BrowserHost: 'settings.debug.console_latency_host_page',
  SimHost: 'settings.debug.console_latency_sim',
});

/**
 * The segments, in the order a round trip happens. `input_to_ack` last because
 * it is the total the others decompose.
 */
const SEGMENTS = Object.freeze([
  'input_to_send',
  'send_to_ack',
  'admit_to_broadcast',
  'input_to_ack',
]);

/**
 * Parse the raw bridge JSON into a payload, or `null` when there is nothing
 * renderable yet (empty string before the first publish, or malformed input).
 *
 * @param {string} json
 * @returns {object|null}
 */
export function parseConsoleLatency(json) {
  if (typeof json !== 'string' || json.length === 0) return null;
  let payload;
  try {
    payload = JSON.parse(json);
  } catch {
    return null;
  }
  if (!payload || !Array.isArray(payload.actions)) return null;
  return payload;
}

/** One cell's text: a rounded millisecond figure, or an em dash when absent. */
function cellText(summary, key) {
  if (!summary || !Number.isFinite(summary[key])) return '—';
  const ms = summary[key];
  // Sub-millisecond figures are common on the in-process host path and would all
  // read as "0" at whole-millisecond resolution, which would hide the very
  // difference between the paths this table exists to show.
  return ms < 10 ? `${ms.toFixed(1)}` : `${Math.round(ms)}`;
}

/**
 * Build the table DOM from a parsed payload. Pure: returns a detached element,
 * mutates nothing.
 *
 * @param {object} payload  a parsed `ConsoleLatencyPayload`
 * @param {{doc?: Document}} [opts]
 * @returns {HTMLElement}
 */
export function buildConsoleLatencyTable(payload, opts = {}) {
  const doc = opts.doc || document;

  const root = doc.createElement('div');
  root.className = 'cl-panel';

  const title = doc.createElement('div');
  title.className = 'cl-title';
  title.textContent = t('settings.debug.console_latency');
  root.appendChild(title);

  const caption = doc.createElement('div');
  caption.className = 'cl-caption';
  caption.textContent = t('settings.debug.console_latency_caption');
  root.appendChild(caption);

  const table = doc.createElement('table');
  table.className = 'cl-table';

  const head = doc.createElement('tr');
  head.className = 'cl-head';
  // Machine column heads: the segment names are the payload's own field names
  // and the statistics are p50/p75/max, so an operator reading this table and an
  // operator reading the run report's JSON see the same words.
  for (const label of ['surface', 'action', 'n', ...SEGMENTS.map((s) => `${s} p50/p75/max`)]) {
    const th = doc.createElement('th');
    th.textContent = label;
    head.appendChild(th);
  }
  table.appendChild(head);

  for (const entry of payload.actions) {
    const row = doc.createElement('tr');
    row.className = 'cl-row';
    row.setAttribute('data-surface', String(entry.surface || ''));
    row.setAttribute('data-action', String(entry.action || ''));

    const surface = doc.createElement('td');
    surface.className = 'cl-surface';
    const labelId = SURFACE_LABELS[entry.surface];
    surface.textContent = labelId ? t(labelId) : String(entry.surface || '');
    row.appendChild(surface);

    const action = doc.createElement('td');
    action.className = 'cl-action';
    action.textContent = String(entry.action || '');
    row.appendChild(action);

    const count = doc.createElement('td');
    count.className = 'cl-count';
    count.textContent = String(entry.count | 0);
    row.appendChild(count);

    for (const segment of SEGMENTS) {
      const cell = doc.createElement('td');
      cell.className = 'cl-cell';
      cell.setAttribute('data-segment', segment);
      const summary = entry[segment];
      cell.textContent = summary
        ? `${cellText(summary, 'p50_ms')}/${cellText(summary, 'p75_ms')}/${cellText(summary, 'max_ms')}`
        : '—';
      row.appendChild(cell);
    }

    table.appendChild(row);
  }

  root.appendChild(table);
  return root;
}

/**
 * Render the table (or an empty-state placeholder) into `container` from the raw
 * bridge JSON. Clears the container first. The settings cog calls this each
 * frame while the console-latency output is the visible one.
 *
 * @param {Element} container
 * @param {string} json  raw JSON from `wasm_get_console_latency()`
 * @param {{doc?: Document}} [opts]
 */
export function renderConsoleLatencyPanel(container, json, opts = {}) {
  if (!container) return;
  const doc = container.ownerDocument || opts.doc || document;
  const payload = parseConsoleLatency(json);
  container.textContent = '';
  if (!payload || payload.actions.length === 0) {
    const empty = doc.createElement('div');
    empty.className = 'cl-empty';
    empty.textContent = t('settings.debug.console_latency_empty');
    container.appendChild(empty);
    return;
  }
  container.appendChild(buildConsoleLatencyTable(payload, { doc }));
}

// Expose for the classic-script bootstrap in server.html, which wires this
// renderer into the settings cog's console-latency output.
if (typeof window !== 'undefined') {
  window.renderConsoleLatencyPanel = renderConsoleLatencyPanel;
}
