/**
 * tests/smoke/strings.js — resolve string-table ids for smoke assertions.
 *
 * Assertions go through ts('some.id') instead of hardcoding English, so they
 * survive both the [bracket] placeholder phase and future copy edits — the id
 * is the contract, exactly as in the vitest suites.
 *
 * Carries its own tiny RFC 4180 parse because this package is CommonJS and
 * gui/strings.js is an ES module. gui/strings.js remains canonical; if the
 * two ever disagree these assertions fail loudly against the rendered page,
 * which is the drift alarm we want.
 */

import { readFileSync } from 'node:fs';
import * as path from 'node:path';

function parseCsv(text) {
  const rows = [];
  let row = [];
  let field = '';
  let quoted = false;
  for (let i = text.charCodeAt(0) === 0xfeff ? 1 : 0; i < text.length; i += 1) {
    const ch = text[i];
    if (quoted) {
      if (ch === '"') {
        if (text[i + 1] === '"') { field += '"'; i += 1; } else quoted = false;
      } else field += ch;
    } else if (ch === '"' && field === '') quoted = true;
    else if (ch === ',') { row.push(field); field = ''; }
    else if (ch === '\r') { /* skip */ }
    else if (ch === '\n') {
      row.push(field); field = '';
      if (!(row.length === 1 && row[0] === '')) rows.push(row);
      row = [];
    } else field += ch;
  }
  if (field !== '' || row.length > 0) { row.push(field); rows.push(row); }
  return rows;
}

const table = new Map();
{
  const csvPath = path.resolve(__dirname, '../../assets/strings/strings.csv');
  const rows = parseCsv(readFileSync(csvPath, 'utf8'));
  const header = rows[0];
  const idCol = header.indexOf('id');
  const enCol = header.indexOf('en');
  for (let r = 1; r < rows.length; r += 1) {
    const id = (rows[r][idCol] || '').trim();
    if (id) table.set(id, rows[r][enCol] ?? '');
  }
}

/** Resolve a string id, substituting {placeholder} tokens. Throws on a missing id. */
export function ts(id, params) {
  const raw = table.get(id);
  if (raw === undefined) throw new Error(`strings.csv has no id '${id}'`);
  if (!params) return raw;
  return raw.replace(/\{(\w+)\}/g, (m, key) =>
    Object.prototype.hasOwnProperty.call(params, key) ? String(params[key]) : m,
  );
}
