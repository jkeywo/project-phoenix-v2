import { afterEach, describe, expect, it } from 'vitest';
import { setBaseCatalogue, setLocale, setOverlayCatalogues } from '../../gui/strings.js';
import { snapshotDiagnosticText } from '../../gui/snapshot-status.js';

const CORE = 'id,en\n'
  + 'server.snapshot.no_run,Nothing to save\n'
  + 'server.snapshot.saved_at_tick,Saved at tick {tick}\n'
  + 'server.snapshot.write_failed,Save failed. {detail}\n'
  + 'server.snapshot.digest_mismatch,Recorded {expected}; restored {actual}\n'
  + 'server.snapshot.technical_refusal,Save failed. {detail}\n';
const GERMAN = 'id,de,de_source,de_provenance\n'
  + 'server.snapshot.no_run,Nichts zu speichern,Nothing to save,machine\n'
  + 'server.snapshot.saved_at_tick,Bei Tick {tick} gespeichert,Saved at tick {tick},machine\n'
  + 'server.snapshot.write_failed,Speichern fehlgeschlagen. {detail},Save failed. {detail},machine\n';

afterEach(() => {
  setOverlayCatalogues([]);
  setLocale('en');
});

describe('host save and recovery presentation', () => {
  it('shows German setup/save outcomes with typed tick and retained storage detail', () => {
    setBaseCatalogue(CORE);
    setOverlayCatalogues([{ source: 'fixture-de', csv: GERMAN }]);
    setLocale('de');
    expect(snapshotDiagnosticText('{"kind":"no_run"}')).toBe('Nichts zu speichern');
    expect(snapshotDiagnosticText('{"kind":"saved_at_tick","params":{"tick":1234}}'))
      .toBe('Bei Tick 1.234 gespeichert');
    expect(snapshotDiagnosticText('{"kind":"write_failed","params":{"detail":"QuotaExceededError"}}'))
      .toBe('Speichern fehlgeschlagen. QuotaExceededError');
  });

  it('uses effective English for missing German recovery and unknown legacy outcomes', () => {
    setBaseCatalogue(CORE);
    setOverlayCatalogues([{ source: 'fixture-de', csv: GERMAN }]);
    setLocale('de');
    expect(snapshotDiagnosticText('{"kind":"digest_mismatch","params":{"expected":"a1","actual":"b2"}}'))
      .toBe('Recorded a1; restored b2');
    expect(snapshotDiagnosticText('old store refused')).toBe('Save failed. old store refused');
  });
});
