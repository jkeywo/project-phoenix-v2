import { parseCsv, serializeCsv } from '../gui/csv.js';
import { composeStringCatalogues, explainCatalogueEntry,
  STRING_CATALOGUE_PATH } from '../gui/string-catalogue.js';

function catalogue(files) {
  return files?.[STRING_CATALOGUE_PATH] ?? files?.['strings/strings.csv'] ?? null;
}

export function workshopCatalogueSources(dependencies, draft) {
  const sources = [];
  const core = catalogue(dependencies?.base_files);
  if (typeof core === 'string') sources.push({ source: 'core', text: core });
  for (const pack of dependencies?.packs || []) {
    const text = catalogue(pack.files);
    if (typeof text === 'string') sources.push({ source: String(pack.id), text });
  }
  const authored = draft?.read?.(STRING_CATALOGUE_PATH);
  if (typeof authored === 'string') sources.push({ source: 'draft', text: authored });
  return sources;
}

export function authoredStringIds(draft) {
  let rows;
  try {
    rows = parseCsv(draft?.read?.(STRING_CATALOGUE_PATH) || '');
  } catch {
    // composeStringCatalogues owns the author-facing invalid-catalogue
    // diagnostic. A half-typed quoted field must not prevent that report from
    // reaching the Workshop panel.
    return [];
  }
  const id = rows[0]?.indexOf('id') ?? -1;
  return id < 0 ? [] : rows.slice(1).map((row) => row[id]?.trim()).filter(Boolean);
}

export function workshopCatalogueReport(dependencies, draft, locale) {
  const report = composeStringCatalogues(workshopCatalogueSources(dependencies, draft), locale);
  const ids = authoredStringIds(draft);
  return { ...report, authored: ids.map((id) => explainCatalogueEntry(report, id)) };
}

/** Context follows the same source order as the composed catalogue. */
export function workshopStringContexts(dependencies, draft) {
  const contexts = new Map();
  for (const source of workshopCatalogueSources(dependencies, draft)) {
    let rows;
    try { rows = parseCsv(source.text); } catch { continue; }
    const id = rows[0]?.indexOf('id') ?? -1;
    const context = rows[0]?.indexOf('context') ?? -1;
    if (id < 0 || context < 0) continue;
    for (const row of rows.slice(1)) {
      if (row[id] && row[context]) contexts.set(row[id], row[context]);
    }
  }
  return contexts;
}

/** One translation edit in the ordinary undoable CSV member, preserving other columns. */
export function editTranslationValue(csv, id, locale, translation, english, context = '', provenance = '') {
  if (!/^[a-z]{2,3}(?:-[A-Z]{2})?$/.test(locale) || locale === 'en' || !id || !english) {
    throw new Error('Invalid translation target');
  }
  const rows = csv ? parseCsv(csv) : [['id', 'context']];
  if (!rows.length) rows.push(['id', 'context']);
  const header = rows[0];
  const idCol = header.indexOf('id');
  if (idCol < 0) throw new Error('String Table has no id column');
  if (rows.slice(1).some(row => row.length !== header.length)) {
    throw new Error('String Table has malformed rows');
  }
  const ensureColumn = (name) => {
    const existing = header.indexOf(name);
    if (existing >= 0) return existing;
    const added = header.length;
    header.push(name);
    for (const row of rows.slice(1)) row.push('');
    return added;
  };
  const contextCol = ensureColumn('context');
  const valueCol = ensureColumn(locale);
  const sourceCol = ensureColumn(`${locale}_source`);
  const provenanceCol = ensureColumn(`${locale}_provenance`);
  let row = rows.slice(1).find(candidate => candidate[idCol] === id);
  if (!row) {
    row = Array(header.length).fill('');
    row[idCol] = id;
    row[contextCol] = context;
    rows.push(row);
  }
  const changedValue = row[valueCol] !== translation;
  row[valueCol] = translation;
  row[provenanceCol] = provenance;
  if (changedValue) row[sourceCol] = translation.trim() ? english : '';
  return serializeCsv(rows, csv?.includes('\r\n') ? '\r\n' : '\n');
}

/** Explicitly accept current effective English as the source of an existing translation. */
export function refreshTranslationSource(csv, id, locale, effectiveEnglish) {
  if (!locale || locale === 'en') return csv;
  const rows = parseCsv(csv);
  if (!rows.length) return csv;
  const header = rows[0];
  const idCol = header.indexOf('id');
  const translationCol = header.indexOf(locale);
  if (idCol < 0 || translationCol < 0) return csv;
  const row = rows.slice(1).find((candidate) => candidate[idCol] === id);
  if (!row || !String(row[translationCol] || '').trim()) return csv;
  const sourceName = `${locale}_source`;
  let sourceCol = header.indexOf(sourceName);
  if (sourceCol < 0) {
    sourceCol = header.length;
    header.push(sourceName);
    for (const candidate of rows.slice(1)) candidate.push('');
  }
  row[sourceCol] = String(effectiveEnglish);
  const newline = csv.includes('\r\n') ? '\r\n' : '\n';
  return serializeCsv(rows, newline);
}
