/**
 * Compose the core String Table with partial catalogues supplied by ordinary
 * mod packs. Sources are ordered from lowest to highest precedence.
 *
 * A translation column is paired with `<locale>_source`, the exact effective
 * English value it was translated from, and optional `<locale>_provenance`.
 * Keeping the source value (rather than a time or row number) makes freshness
 * deterministic after English overrides and survives CSV/ZIP round trips.
 */

import { parseCsv } from './csv.js';

export const STRING_CATALOGUE_PATH = 'assets/strings/strings.csv';

const LOCALE = /^[a-z]{2,3}(?:-[A-Z]{2})?$/;
const PARAM = /\{([A-Za-z_][A-Za-z0-9_]*)\}/g;

function localeColumns(header) {
  // `id` is both a valid BCP-47 language tag and this format's structural key.
  // In a String Table it is always structural, never an Indonesian catalogue.
  return header.filter((name) => name !== 'id' && LOCALE.test(name));
}

function parameters(text) {
  return [...String(text).matchAll(PARAM)].map((match) => match[1]).sort();
}

function sameParameters(a, b) {
  const left = parameters(a);
  const right = parameters(b);
  return left.length === right.length && left.every((value, i) => value === right[i]);
}

/** Structural and fallback diagnostics available before a pack is installed. */
export function validatePartialStringCatalogue(text) {
  const findings = [];
  const add = (severity, category, id, message) => findings.push({ severity, category, id, message });
  let rows;
  try {
    rows = parseCsv(String(text || ''));
  } catch (error) {
    add('error', 'malformed-string-catalogue', '',
      `String Table CSV is malformed: ${error.message}`);
    return findings;
  }
  if (!rows.length) {
    add('error', 'string-catalogue-schema', '', 'String Table is empty');
    return findings;
  }
  const header = rows[0];
  const idCol = header.indexOf('id');
  if (idCol < 0) add('error', 'string-catalogue-schema', '', "missing required 'id' column");
  if (new Set(header).size !== header.length) {
    add('error', 'string-catalogue-schema', '', 'duplicate column names');
  }
  const locales = localeColumns(header);
  if (!locales.length) add('error', 'string-catalogue-schema', '', 'no locale columns');
  for (const name of header) {
    const match = name.match(/^(.+)_(source|provenance)$/);
    if (match && !locales.includes(match[1])) {
      add('error', 'string-catalogue-schema', '', `${name} has no matching ${match[1]} locale column`);
    }
  }
  const ids = new Set();
  for (let index = 1; index < rows.length; index += 1) {
    const row = rows[index];
    const id = idCol < 0 ? '' : String(row[idCol] || '').trim();
    if (row.length !== header.length) {
      add('error', 'malformed-string-catalogue', id,
        `row ${index + 1} has ${row.length} fields; header has ${header.length}`);
      continue;
    }
    if (!id) add('error', 'string-catalogue-schema', '', `row ${index + 1} has a blank id`);
    else if (ids.has(id)) add('error', 'string-catalogue-schema', id, 'duplicate id');
    ids.add(id);
    const cells = Object.fromEntries(header.map((name, col) => [name, row[col] ?? '']));
    for (const locale of locales.filter((name) => name !== 'en')) {
      const value = cells[locale];
      if (!value?.trim()) {
        add('warning', 'translation-fallback', id, `${locale} is blank; players use effective English`);
        continue;
      }
      const source = cells[`${locale}_source`];
      if (!source) {
        add('warning', 'translation-fallback', id,
          `${locale}_source is missing or blank; players use effective English`);
        continue;
      }
      const reference = cells.en || source;
      if (!sameParameters(reference, value)) {
        add('warning', 'invalid-translation-placeholders', id,
          `${locale} placeholders do not match ${cells.en ? 'English' : `${locale}_source`}; players use effective English`);
      }
    }
  }
  return findings;
}

function finding(category, source, id, locale, message, extra = {}) {
  return { category, source, id, locale, message, ...extra };
}

function readSource(input, diagnostics) {
  const source = String(input?.source || 'unknown');
  let rows;
  try {
    rows = parseCsv(String(input?.text || ''));
  } catch (error) {
    diagnostics.push(finding(
      'invalid-catalogue', source, '', '', `catalogue CSV is malformed: ${error.message}`,
    ));
    return { source, locales: [], rows: [] };
  }
  if (rows.length === 0) {
    diagnostics.push(finding('invalid-catalogue', source, '', '', 'catalogue is empty'));
    return { source, locales: [], rows: [] };
  }
  const header = rows[0];
  const idCol = header.indexOf('id');
  if (idCol < 0) {
    diagnostics.push(finding('invalid-catalogue', source, '', '', "catalogue has no 'id' column"));
    return { source, locales: [], rows: [] };
  }
  const locales = localeColumns(header);
  const seen = new Set();
  const parsed = [];
  for (let i = 1; i < rows.length; i += 1) {
    const row = rows[i];
    if (row.length !== header.length) {
      diagnostics.push(finding(
        'invalid-entry', source, row[idCol] || '', '',
        `row ${i + 1} has ${row.length} fields; header has ${header.length}`,
      ));
      continue;
    }
    const id = String(row[idCol] || '').trim();
    if (!id) {
      diagnostics.push(finding('invalid-entry', source, '', '', `row ${i + 1} has a blank id`));
      continue;
    }
    if (seen.has(id)) {
      diagnostics.push(finding('conflicting-entry', source, id, '', 'duplicate id; later row wins'));
    }
    seen.add(id);
    const cells = Object.fromEntries(header.map((name, col) => [name, row[col] ?? '']));
    const previous = parsed.findIndex((entry) => entry.id === id);
    if (previous >= 0) parsed.splice(previous, 1);
    parsed.push({ id, cells });
  }
  return { source, locales, rows: parsed };
}

/**
 * @param {Array<{source:string,text:string}>} inputs core first, mods in load order
 * @param {string} locale selected BCP-47 language tag
 */
export function composeStringCatalogues(inputs, locale = 'en') {
  const diagnostics = [];
  const sources = (inputs || []).map((input) => readSource(input, diagnostics));
  const localeSet = new Set(['en']);
  for (const source of sources) for (const value of source.locales) localeSet.add(value);

  /** @type {Map<string, Array<{source:string,cells:Record<string,string>}>>} */
  const byId = new Map();
  for (const source of sources) {
    for (const row of source.rows) {
      const chain = byId.get(row.id) || [];
      chain.push({ source: source.source, cells: row.cells });
      byId.set(row.id, chain);
    }
  }

  const table = new Map();
  const entries = new Map();
  for (const [id, chain] of byId) {
    const englishCandidates = chain.filter((candidate) => Object.hasOwn(candidate.cells, 'en'));
    const usableEnglish = englishCandidates.filter((candidate) => candidate.cells.en.trim() !== '');
    const english = usableEnglish.at(-1);
    if (!english) {
      diagnostics.push(finding('missing-english', chain.at(-1).source, id, 'en', 'no usable English source'));
      continue;
    }

    let value = english.cells.en;
    let status = locale === 'en' ? 'current' : 'missing';
    let winningSource = english.source;
    const candidates = locale === 'en'
      ? []
      : chain.filter((candidate) => Object.hasOwn(candidate.cells, locale));
    const winner = candidates.at(-1);

    if (candidates.length > 1) {
      diagnostics.push(finding(
        'conflicting-entry', winner.source, id, locale,
        `translation from '${winner.source}' wins ordinary mod precedence`,
        { winner: winner.source, shadowed: candidates.slice(0, -1).map((candidate) => candidate.source) },
      ));
    }

    if (locale !== 'en') {
      if (!winner) {
        diagnostics.push(finding('missing-translation', english.source, id, locale, 'using English fallback'));
      } else if (winner.cells[locale].trim() === '') {
        status = 'blank';
        diagnostics.push(finding('blank-translation', winner.source, id, locale, 'using English fallback'));
      } else if (!sameParameters(english.cells.en, winner.cells[locale])) {
        status = 'invalid';
        diagnostics.push(finding(
          'invalid-translation', winner.source, id, locale,
          'placeholder names do not match effective English; using English fallback',
        ));
      } else {
        const sourceKey = `${locale}_source`;
        const translatedFrom = winner.cells[sourceKey];
        if (translatedFrom === undefined || translatedFrom === '') {
          status = 'invalid';
          diagnostics.push(finding(
            'invalid-translation', winner.source, id, locale,
            `missing ${sourceKey} freshness metadata; using English fallback`,
          ));
        } else if (translatedFrom !== english.cells.en) {
          status = 'stale';
          diagnostics.push(finding(
            'stale-translation', winner.source, id, locale,
            'effective English changed after this translation; using English fallback',
            { translatedFrom, effectiveEnglish: english.cells.en },
          ));
        } else {
          value = winner.cells[locale];
          status = 'current';
          winningSource = winner.source;
        }
      }
    }

    table.set(id, value);
    entries.set(id, {
      id,
      locale,
      value,
      english: english.cells.en,
      englishSource: english.source,
      translation: winner?.cells?.[locale],
      translationSource: winner?.source || null,
      provenance: winner?.cells?.[`${locale}_provenance`] || '',
      status,
      winningSource,
    });
  }

  return { table, entries, diagnostics, locales: [...localeSet].sort() };
}
