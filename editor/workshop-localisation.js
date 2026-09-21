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
