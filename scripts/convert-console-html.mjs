/**
 * scripts/convert-console-html.mjs — One-shot localiser for the per-ship
 * console HTML pages (gui/<ship>/<station>.html).
 *
 * Handles the shapes that repeat across all 22 pages, tags them with
 * data-i18n / data-i18n-attr, and appends the matching rows to
 * assets/strings/strings.csv (merge semantics — existing ids are kept):
 *
 *   - <title>…</title> and data-screen-label="…"
 *   - the <h1>TITLE<span class="sub">SUB</span></h1> hero
 *   - AUTO badges, footer station label, footer-target placeholder
 *   - <h2>Panel Heading</h2>
 *   - <div class="metric"><span>Label</span>…
 *   - <span class="label">Button Label</span> inside buttons
 *   - adds `import { t } from '../strings.js';` after the console-core import
 *
 * Ids are per-station (console.sensors.title), shared across ships when the
 * text agrees; when two ships disagree for the same slot the second gets a
 * ship-qualified id (console.courier_pilot.title). Everything it cannot
 * confidently classify is printed as a leftover for hand conversion.
 *
 * Idempotent: nodes already carrying data-i18n are skipped.
 */

import { readFile, writeFile, readdir } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseCsv } from '../gui/strings.js';
import { csvField } from './strings-csv.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const SHIPS = ['battleship', 'cruiser', 'courier', 'destroyer'];

const rows = new Map(); // id → { context, en }
const idByText = new Map(); // `${station}|${field}|${text}` memo

function slug(s) {
  return s.toLowerCase().replace(/&amp;/g, 'and').replace(/[^a-z0-9]+/g, '_').replace(/^_+|_+$/g, '').slice(0, 30);
}

/**
 * Mint (or reuse) an id for text at a station slot. Same station+field+text
 * across ships → same id. Same slot, different text → ship-qualified id.
 */
function mintId(ship, station, field, text, context) {
  const memo = `${station}|${field}|${text}`;
  if (idByText.has(memo)) return idByText.get(memo);

  let id = `console.${station}.${field}`;
  if (rows.has(id) || idByText.has(`__taken:${id}`)) {
    id = `console.${ship}_${station}.${field}`;
  }
  idByText.set(memo, id);
  idByText.set(`__taken:${`console.${station}.${field}`}`, true);
  rows.set(id, { context, en: `[${text}]` });
  return id;
}

const leftovers = [];

async function convert(ship, file) {
  const abs = path.join(root, 'gui', ship, file);
  const station = path.basename(file, '.html');
  let src = await readFile(abs, 'utf8');
  const rel = `gui/${ship}/${file}`;

  // 1. <title>
  src = src.replace(/<title>([^<]+)<\/title>/, (m, text) => {
    const id = mintId(ship, station, 'doc_title', text, `Browser-tab title of the ${station} console page`);
    return `<title data-i18n="${id}">${text}</title>`;
  });

  // 2. data-screen-label (skip if already tagged)
  src = src.replace(/data-screen-label="([^"]+)"(?! data-i18n-attr)/, (m, text) => {
    const id = mintId(ship, station, 'screen_label', text, `Phone-frame screen label of the ${station} console`);
    return `data-screen-label="${text}" data-i18n-attr="data-screen-label:${id}"`;
  });

  // 3. hero h1 with sub
  src = src.replace(/<h1>([^<]+)<span class="sub">([^<]+)<\/span><\/h1>/, (m, title, sub) => {
    const tId = mintId(ship, station, 'title', title, `${station} console — main hero heading`);
    const sId = mintId(ship, station, 'subtitle', sub, `${station} console — hero sub-heading under the title`);
    return `<h1><span data-i18n="${tId}">${title}</span><span class="sub" data-i18n="${sId}">${sub}</span></h1>`;
  });
  // hero h1 without sub
  src = src.replace(/<h1>([^<]+)<\/h1>/, (m, title) => {
    if (m.includes('data-i18n')) return m;
    const tId = mintId(ship, station, 'title', title, `${station} console — main hero heading`);
    return `<h1 data-i18n="${tId}">${title}</h1>`;
  });

  // 4. AUTO badges (element form; component templates handle their own)
  src = src.replace(/(<span class="auto-badge"[^>]*hidden)>AUTO<\/span>/g, (m, pre) =>
    pre.includes('data-i18n') ? m : `${pre} data-i18n="console.common.auto">AUTO</span>`);

  // 5. footer "X STATION" (with or without an id attribute)
  src = src.replace(/<span( id="footer-status")?>([A-Z][A-Z ]+ STATION)<\/span>/g, (m, idAttr, text) => {
    if (m.includes('data-i18n')) return m;
    const id = mintId(ship, station, 'station_footer', text, `${station} console — footer station name`);
    return `<span${idAttr || ''} data-i18n="${id}">${text}</span>`;
  });

  // 6. footer-target placeholders (plain and styled variants)
  src = src.replace(/<span id="footer-target"([^>]*)>NO TARGET<\/span>/g, (m, attrs) =>
    m.includes('data-i18n') ? m : `<span id="footer-target"${attrs} data-i18n="console.common.no_target">NO TARGET</span>`);
  src = src.replace(/<span id="footer-target"([^>]*)>NO ACTIVE HAIL<\/span>/g, (m, attrs) => {
    if (m.includes('data-i18n')) return m;
    rows.set('console.common.no_active_hail', { context: 'Console footer when no hail is open', en: '[NO ACTIVE HAIL]' });
    return `<span id="footer-target"${attrs} data-i18n="console.common.no_active_hail">NO ACTIVE HAIL</span>`;
  });
  src = src.replace(/<span id="footer-target"([^>]*)>NO WAYPOINT<\/span>/g, (m, attrs) => {
    if (m.includes('data-i18n')) return m;
    rows.set('console.common.no_waypoint', { context: 'Console footer when no waypoint is set', en: '[NO WAYPOINT]' });
    return `<span id="footer-target"${attrs} data-i18n="console.common.no_waypoint">NO WAYPOINT</span>`;
  });

  // 6b. courier hero form: <h1>TITLE<small>SUB</small></h1>
  src = src.replace(/<h1>([^<]+)<small>([^<]+)<\/small><\/h1>/, (m, title, sub) => {
    const tId = mintId(ship, station, 'title', title, `${station} console — main hero heading`);
    const sId = mintId(ship, station, 'subtitle', sub, `${station} console — hero sub-heading under the title`);
    return `<h1><span data-i18n="${tId}">${title}</span><small data-i18n="${sId}">${sub}</small></h1>`;
  });

  // 7. <h2>Heading</h2>
  src = src.replace(/<h2>([^<{][^<]*)<\/h2>/g, (m, text) => {
    if (m.includes('data-i18n')) return m;
    const id = mintId(ship, station, slug(text), text, `${station} console — "${text}" panel heading`);
    return `<h2 data-i18n="${id}">${text}</h2>`;
  });

  // 8. metric label spans: <div class="metric"><span>Label</span>
  src = src.replace(/(<div class="metric">)<span>([^<]+)<\/span>/g, (m, pre, text) => {
    const id = mintId(ship, station, slug(text), text, `${station} console — "${text}" metric label`);
    return `${pre}<span data-i18n="${id}">${text}</span>`;
  });

  // 9. button labels: <span class="label">Text</span>
  src = src.replace(/<span class="label">([A-Za-z][^<]*)<\/span>/g, (m, text) => {
    if (text === 'Cancel Impulse') {
      rows.set('console.common.cancel_impulse', { context: 'Button that aborts an active impulse-drive charge', en: '[Cancel Impulse]' });
      return '<span class="label" data-i18n="console.common.cancel_impulse">Cancel Impulse</span>';
    }
    const id = mintId(ship, station, slug(text), text, `${station} console — "${text}" button`);
    return `<span class="label" data-i18n="${id}">${text}</span>`;
  });

  // 10. ensure the t import exists next to console-core
  if (!src.includes("from '../strings.js'") && src.includes("from '../console-core.js'")) {
    src = src.replace(
      /(import \{[^}]*\} from '\.\.\/console-core\.js';)/,
      `$1\n    import { t } from '../strings.js';`,
    );
  }

  await writeFile(abs, src, 'utf8');

  // Report anything that still looks like display text.
  const suspicious = /(?:>[A-Z][A-Z0-9 &.%/:'-]{2,}<)|(?:textContent = '[A-Za-z])|(?:\|\| '[A-Z])|(?:\? '[A-Z][A-Z ])/;
  src.split('\n').forEach((line, i) => {
    if (line.includes('data-i18n') || line.includes("t('")) return;
    if (/^\s*(\/\/|\*|<!--)/.test(line)) return;
    if (suspicious.test(line)) leftovers.push(`${rel}:${i + 1}: ${line.trim().slice(0, 110)}`);
  });
}

// ── CSV append (merge — never overwrite existing ids) ───────────────────────

async function main() {
  for (const ship of SHIPS) {
    for (const file of await readdir(path.join(root, 'gui', ship))) {
      if (file.endsWith('.html')) await convert(ship, file);
    }
  }

  const csvPath = path.join(root, 'assets', 'strings', 'strings.csv');
  const existing = await readFile(csvPath, 'utf8');
  // Read through the real parser. Splitting raw lines on ',' walks straight into
  // the file's multi-line quoted comms prose and mints ids out of continuation
  // lines — junk that a genuinely new id could collide with and be dropped from
  // the merge without a word.
  const known = new Set(parseCsv(existing).map((r) => r[0]));
  const fresh = [...rows].filter(([id]) => !known.has(id));
  const lines = fresh.map(([id, r]) => [id, r.context, r.en].map(csvField).join(','));
  if (lines.length) {
    await writeFile(csvPath, existing.replace(/\n?$/, '\n') + lines.join('\n') + '\n', 'utf8');
  }

  console.log(`tagged pages; ${fresh.length} new CSV rows`);
  console.log(`\n${leftovers.length} leftover lines for hand conversion:`);
  for (const l of leftovers) console.log('  ' + l);
}

main().catch((e) => { console.error(e); process.exit(1); });
