/**
 * gui/station-activity-chart.js — the debug dock's station-activity chart
 * (issue #1145, PRD #1144).
 *
 * The first JSON-driven renderer on the structured debug pipeline. It parses the
 * `StationActivityPayload` the WASM bridge publishes (`wasm_get_station_activity`)
 * and draws per-station admitted-command activity over time, split human vs AI —
 * the evidence crew-control and Backfill tuning need. Nothing here talks to the
 * simulation: it is a pure function of the payload, so it is unit-tested in jsdom
 * without a browser or a WASM bundle, exactly as the rest of `gui/` is.
 *
 * This is the renderer pattern the later PRD #1144 surfaces copy: a `parse*`
 * guard, a pure `build*Chart(payload)` that returns DOM, and a
 * `render*(container, json)` wrapper the settings cog wires in.
 */

import { t } from './strings.js';

const SVG_NS = 'http://www.w3.org/2000/svg';

// Layout constants — presentation only, no gameplay meaning. Pixels.
const ROW_HEIGHT = 34;
const BAR_MAX_HEIGHT = 24;
const BAR_WIDTH = 10;
const BAR_GAP = 3;
const LABEL_WIDTH = 96;
const PADDING = 8;

/**
 * Parse the raw bridge JSON into a payload, or `null` when there is nothing
 * renderable yet (empty string before the first publish, or malformed input).
 *
 * @param {string} json
 * @returns {object|null}
 */
export function parseStationActivity(json) {
  if (typeof json !== 'string' || json.length === 0) return null;
  let payload;
  try {
    payload = JSON.parse(json);
  } catch {
    return null;
  }
  if (!payload || !Array.isArray(payload.buckets)) return null;
  return payload;
}

/** The distinct station ids across every bucket, sorted for a stable lane order. */
function stationsIn(payload) {
  const seen = new Set();
  for (const bucket of payload.buckets) {
    for (const entry of bucket.stations || []) {
      if (entry && typeof entry.station === 'string') seen.add(entry.station);
    }
  }
  return Array.from(seen).sort();
}

/** The largest single-bucket total across all stations, for vertical scaling. */
function maxTotal(payload) {
  let max = 0;
  for (const bucket of payload.buckets) {
    for (const entry of bucket.stations || []) {
      const total = (entry.human | 0) + (entry.ai | 0) + (entry.offline | 0);
      if (total > max) max = total;
    }
  }
  return max;
}

/** Find one station's entry within a bucket, or a zeroed stand-in. */
function entryFor(bucket, station) {
  const found = (bucket.stations || []).find((e) => e && e.station === station);
  return found || { station, human: 0, ai: 0, offline: 0 };
}

function svgEl(name, attrs = {}, doc) {
  const el = doc.createElementNS(SVG_NS, name);
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, String(v));
  return el;
}

/**
 * Build the chart DOM from a parsed payload. Pure: returns a detached element,
 * mutates nothing.
 *
 * @param {object} payload  a parsed `StationActivityPayload`
 * @param {{doc?: Document}} [opts]
 * @returns {HTMLElement}
 */
export function buildStationActivityChart(payload, opts = {}) {
  const doc = opts.doc || document;
  const stations = stationsIn(payload);
  const buckets = payload.buckets;
  const scaleMax = Math.max(1, maxTotal(payload));

  const root = doc.createElement('div');
  root.className = 'sa-chart';

  const title = doc.createElement('div');
  title.className = 'sa-title';
  title.textContent = t('settings.debug.station_activity');
  root.appendChild(title);

  const caption = doc.createElement('div');
  caption.className = 'sa-caption';
  // `bucket_secs` may be absent on a bare-fixture payload; fall back to a dash.
  const secs = Number.isFinite(payload.bucket_secs) ? payload.bucket_secs : 0;
  caption.textContent = t('settings.debug.station_activity_buckets', { secs });
  root.appendChild(caption);

  const legend = doc.createElement('div');
  legend.className = 'sa-legend';
  for (const [role, labelId] of [
    ['human', 'settings.debug.station_activity_human'],
    ['ai', 'settings.debug.station_activity_ai'],
  ]) {
    const item = doc.createElement('span');
    item.className = 'sa-legend-item';
    item.setAttribute('data-role', role);
    const swatch = doc.createElement('span');
    swatch.className = `sa-swatch sa-swatch-${role}`;
    const label = doc.createElement('span');
    label.textContent = t(labelId);
    item.appendChild(swatch);
    item.appendChild(label);
    legend.appendChild(item);
  }
  root.appendChild(legend);

  const slot = BAR_WIDTH + BAR_GAP;
  const chartWidth = LABEL_WIDTH + Math.max(1, buckets.length) * slot + PADDING;
  const chartHeight = Math.max(1, stations.length) * ROW_HEIGHT + PADDING;

  const svg = svgEl(
    'svg',
    {
      class: 'sa-svg',
      viewBox: `0 0 ${chartWidth} ${chartHeight}`,
      width: chartWidth,
      height: chartHeight,
      role: 'img',
    },
    doc,
  );

  stations.forEach((station, rowIdx) => {
    const rowTop = rowIdx * ROW_HEIGHT + PADDING;
    const baseline = rowTop + BAR_MAX_HEIGHT;

    const group = svgEl('g', { class: 'sa-station', 'data-station': station }, doc);

    const label = svgEl(
      'text',
      {
        class: 'sa-station-label',
        x: 0,
        y: baseline,
        'dominant-baseline': 'ideographic',
      },
      doc,
    );
    label.textContent = station;
    group.appendChild(label);

    buckets.forEach((bucket, bucketIdx) => {
      const entry = entryFor(bucket, station);
      const x = LABEL_WIDTH + bucketIdx * slot;
      // AI stacked below Human so the "Backfill carrying a station" band reads
      // as the base of each bar.
      let y = baseline;
      for (const role of ['ai', 'human']) {
        const count = entry[role] | 0;
        if (count <= 0) continue;
        const h = Math.max(1, Math.round((count / scaleMax) * BAR_MAX_HEIGHT));
        y -= h;
        const rect = svgEl(
          'rect',
          {
            class: `sa-bar sa-bar-${role}`,
            'data-role': role,
            'data-station': station,
            'data-bucket': bucketIdx,
            'data-count': count,
            x,
            y,
            width: BAR_WIDTH,
            height: h,
          },
          doc,
        );
        group.appendChild(rect);
      }
    });

    svg.appendChild(group);
  });

  root.appendChild(svg);
  return root;
}

/**
 * Render the chart (or an empty-state placeholder) into `container` from the raw
 * bridge JSON. Clears the container first. The settings cog calls this each
 * frame while the station-activity output is the visible one.
 *
 * @param {Element} container
 * @param {string} json  raw JSON from `wasm_get_station_activity()`
 * @param {{doc?: Document}} [opts]
 */
export function renderStationActivityChart(container, json, opts = {}) {
  if (!container) return;
  const doc = container.ownerDocument || opts.doc || document;
  const payload = parseStationActivity(json);
  container.textContent = '';
  if (!payload || payload.buckets.length === 0) {
    const empty = doc.createElement('div');
    empty.className = 'sa-empty';
    empty.textContent = t('settings.debug.station_activity_empty');
    container.appendChild(empty);
    return;
  }
  container.appendChild(buildStationActivityChart(payload, { doc }));
}

// Expose for the classic-script bootstrap in server.html, which wires this
// renderer into the settings cog's station-activity output.
if (typeof window !== 'undefined') {
  window.renderStationActivityChart = renderStationActivityChart;
}
