import { describe, expect, it } from 'vitest';
import { composeStringCatalogues, explainCatalogueEntry } from '../../gui/string-catalogue.js';
import {
  getCataloguePresentation, getCatalogueReport, getLocale, setBaseCatalogue, setLocale,
  setOverlayCatalogues, t,
} from '../../gui/strings.js';
import { localiseDeliveredMessage } from '../../gui/rendezvous-transport.js';

const CORE = 'id,context,en\n'
  + 'station.helm.name,tab,Helm\n'
  + 'console.helm.course,course,"Course {degrees}°"\n'
  + 'console.helm.multiline,status,"Line one\nLine two"\n';

const GERMAN = 'id,context,de,de_source,de_provenance\n'
  + 'station.helm.name,tab,Ruder,Helm,machine\n'
  + 'console.helm.course,course,"Kurs {degrees}°","Course {degrees}°",machine\n'
  + 'console.helm.multiline,status,"Zeile eins\nZeile zwei","Line one\nLine two",machine\n';

describe('ordinary mod String Table composition', () => {
  it('advertises only actual presentation locales, never the structural id column', () => {
    setBaseCatalogue(CORE);
    setOverlayCatalogues([]);
    expect(getCataloguePresentation().locales).toEqual(['en']);
    setOverlayCatalogues([{ source: 'de-console', csv: GERMAN }]);
    expect(getCataloguePresentation().locales).toEqual(['de', 'en']);
    setOverlayCatalogues([]);
  });

  it('selects accented/multiline German and keeps identifiers outside the table unchanged', () => {
    const result = composeStringCatalogues([
      { source: 'core', text: CORE },
      { source: 'de-console', text: GERMAN },
    ], 'de');
    expect(result.table.get('station.helm.name')).toBe('Ruder');
    expect(result.table.get('console.helm.course')).toBe('Kurs {degrees}°');
    expect(result.table.get('console.helm.multiline')).toBe('Zeile eins\nZeile zwei');
    expect(result.table.has('helm')).toBe(false);
  });

  it('uses per-key English fallback for missing, blank and invalid translations', () => {
    const partial = 'id,de,de_source\n'
      + 'station.helm.name,,Helm\n'
      + 'console.helm.course,Kurs,Course {degrees}°\n';
    const result = composeStringCatalogues([
      { source: 'core', text: CORE },
      { source: 'partial', text: partial },
    ], 'de');
    expect(result.table.get('station.helm.name')).toBe('Helm');
    expect(result.table.get('console.helm.course')).toBe('Course {degrees}°');
    expect(result.table.get('console.helm.multiline')).toBe('Line one\nLine two');
    expect(result.diagnostics.map((item) => item.category)).toEqual(expect.arrayContaining([
      'blank-translation', 'invalid-translation', 'missing-translation',
    ]));
  });

  it('installs the Welcome stack before a representative Console lookup', () => {
    setBaseCatalogue(CORE);
    setLocale('de');
    setOverlayCatalogues([{ source: 'de-console', csv: GERMAN }]);
    expect(t('station.helm.name')).toBe('Ruder');
    expect(t('console.helm.course', { degrees: 27 })).toBe('Kurs 27°');
    expect(getCatalogueReport().entries.get('station.helm.name').winningSource).toBe('de-console');
    setLocale('en');
    setOverlayCatalogues([]);
  });

  it('negotiates de-DE to de at production ingress before localising Welcome', () => {
    setBaseCatalogue(CORE);
    setLocale('de-DE');
    const delivered = localiseDeliveredMessage({
      type: 'Welcome',
      data: {
        station_label: 'station.helm.name',
        string_catalogues: [{ source: 'de-console', csv: GERMAN }],
      },
    });
    expect(getLocale()).toBe('de');
    expect(delivered.data.station_label).toBe('Ruder');
    expect(delivered.data.string_catalogues[0].csv).toBe(GERMAN);
    setLocale('en');
    setOverlayCatalogues([]);
  });

  it('invalidates against effective overridden English while retaining provenance for refresh', () => {
    const changedEnglish = 'id,en\nstation.helm.name,Flight Control\n';
    const stale = composeStringCatalogues([
      { source: 'core', text: CORE },
      { source: 'english-edit', text: changedEnglish },
      { source: 'de-console', text: GERMAN },
    ], 'de');
    expect(stale.table.get('station.helm.name')).toBe('Flight Control');
    const detail = explainCatalogueEntry(stale, 'station.helm.name');
    expect(detail.entry).toMatchObject({
      status: 'stale',
      englishSource: 'english-edit',
      translation: 'Ruder',
      translationSource: 'de-console',
      provenance: 'machine',
    });
    expect(detail.diagnostics).toEqual(expect.arrayContaining([
      expect.objectContaining({
        category: 'stale-translation',
        translatedFrom: 'Helm',
        effectiveEnglish: 'Flight Control',
      }),
      expect.objectContaining({
        category: 'conflicting-entry', locale: 'en', winner: 'english-edit', shadowed: ['core'],
      }),
    ]));

    const refreshed = GERMAN.replace(',Helm,machine', ',Flight Control,machine');
    const current = composeStringCatalogues([
      { source: 'core', text: CORE },
      { source: 'english-edit', text: changedEnglish },
      { source: 'de-console', text: refreshed },
    ], 'de');
    expect(current.table.get('station.helm.name')).toBe('Ruder');
    expect(current.entries.get('station.helm.name').status).toBe('current');
  });

  it('reports the deterministic winning translation source without exposing a marker', () => {
    const later = 'id,de,de_source,de_provenance\n'
      + 'station.helm.name,Steuerstand,Helm,human\n';
    const result = composeStringCatalogues([
      { source: 'core', text: CORE },
      { source: 'older-de', text: GERMAN },
      { source: 'later-de', text: later },
    ], 'de');
    expect(result.table.get('station.helm.name')).toBe('Steuerstand');
    expect(result.table.get('station.helm.name')).not.toContain('missing');
    expect(explainCatalogueEntry(result, 'station.helm.name')).toMatchObject({
      entry: { winningSource: 'later-de', provenance: 'human', status: 'current' },
      diagnostics: [expect.objectContaining({
        category: 'conflicting-entry', winner: 'later-de', shadowed: ['older-de'],
      })],
    });
  });
});
