import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { parseCsv } from '../../gui/csv.js';
import { composeStringCatalogues } from '../../gui/string-catalogue.js';
import { exportModPack, readStoreZip } from '../../editor/mod-pack-export.js';
import { mergeCatalogCsv } from '../../scripts/extract-strings.mjs';

const CATALOGUE = process.env.PHOENIX_GERMAN_CATALOGUE || 'assets/strings/strings.csv';
const csv = readFileSync(CATALOGUE, 'utf8');

describe('complete first-party German catalogue', () => {
  it('covers every current English String ID with matching source, provenance and parameters', () => {
    const rows = parseCsv(csv);
    const header = rows[0];
    expect(header).toEqual(['id', 'context', 'en', 'de', 'de_source', 'de_provenance']);
    expect(rows.length).toBeGreaterThan(4000);
    const at = Object.fromEntries(header.map((column, index) => [column, index]));
    for (const row of rows.slice(1)) {
      expect(row[at.de], row[at.id]).toBeTruthy();
      expect(row[at.de_source], row[at.id]).toBe(row[at.en]);
      expect(row[at.de_provenance], row[at.id]).toBeTruthy();
    }
    const report = composeStringCatalogues([{ source: 'core', text: csv }], 'de');
    expect(report.entries.size).toBe(rows.length - 1);
    expect(report.diagnostics).toEqual([]);
  });

  it('keeps English fallback for later stale or third-party content', () => {
    const changed = composeStringCatalogues([
      { source: 'core', text: csv },
      { source: 'mod', text: 'id,en\nsettings.language,Language changed\n'
        + 'mod.only,New third-party sentence\n' },
    ], 'de');
    expect(changed.table.get('settings.language')).toBe('Language changed');
    expect(changed.table.get('mod.only')).toBe('New third-party sentence');
    expect(changed.diagnostics).toEqual(expect.arrayContaining([
      expect.objectContaining({ category: 'stale-translation', id: 'settings.language' }),
      expect.objectContaining({ category: 'missing-translation', id: 'mod.only' }),
    ]));
  });

  it('retains reserve charge and crew responsibilities in Dynasty guidance', () => {
    const rows = parseCsv(csv);
    const byId = new Map(rows.slice(1).map(([id, , , de]) => [id, de]));
    expect(byId.get('dynasty.onboarding.strike')).toContain('Reserveenergie nur für zusätzlichen Schaden');
    expect(byId.get('dynasty.onboarding.strike')).toContain('Reichweite, Treffgenauigkeit und Abklingzeit bleiben unverändert');
    expect(byId.get('dynasty.onboarding.charging.title')).toContain('Aufladen');
    expect(byId.get('dynasty.onboarding.command.role')).toContain('Steuerung fliegt, und Geschütz feuert');
    expect(byId.get('dynasty.onboarding.gunnery.role')).toContain('Reserveenergie');
    expect(byId.get('dynasty.onboarding.cycle.text')).toContain('Reserveenergie für einen Schuss nicht aus');
    expect(byId.get('dynasty.onboarding.cycle.text')).toContain('feuert er normal');
  });

  it('survives extraction merge and a validated translation-pack export exactly', () => {
    const merged = mergeCatalogCsv(csv, [{ id: 'later.copy', context: 'later', en: 'New copy' }]);
    const rows = parseCsv(merged.text);
    expect(rows.at(-1)).toEqual(['later.copy', 'later', 'New copy', '', '', '']);
    expect(rows[1]).toEqual(parseCsv(csv)[1]);

    const result = exportModPack({
      pack: { format: 1, id: 'german-catalogue', version: '1.0.0', name: 'German catalogue',
        requires: { content_id: 'phoenix-base', content_epoch: 1 } },
      scenarios: [], files: [{ path: 'assets/strings/strings.csv', text: csv }],
    });
    expect(result.ok).toBe(true);
    expect(result.warnings).toEqual([]);
    expect(readStoreZip(result.zip)['assets/strings/strings.csv']).toBe(csv);
  });
});
