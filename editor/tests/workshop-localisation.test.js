// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest';
import { WorkshopDocument } from '../workshop-document.js';
import { createStoreZip } from '../mod-pack-export.js';
import { mountWorkshopLocalisation } from '../../gui/workshop-localisation-panel.js';
import { editTranslationValue, workshopCatalogueReport } from '../workshop-localisation.js';
import { validatePartialStringCatalogue } from '../../gui/string-catalogue.js';

const PATH = 'assets/strings/strings.csv';
const CORE = 'id,context,en\nstation.helm.name,Bridge station,Helm\n'
  + 'console.course,Helm readout,"Course {degrees}"\ncore.only,Unused,Core only\n'
  + 'messages.one,Counted messages,{n} message\nmessages.other,Counted messages,{n} messages\n';
const ENGLISH_OVERRIDE = 'id,en\nstation.helm.name,Flight Control\n';
const DRAFT = 'id,context,de,de_source,de_provenance\n'
  + 'station.helm.name,tab,Ruder,Helm,human\n'
  + 'console.course,course,Kurs,Course {degrees},machine\n';

function documentFor(csv = DRAFT) {
  return new WorkshopDocument(createStoreZip([
    { path: 'scenarios.toml', text: '[pack]\nformat=1\nid="de-pack"\nname="German"\nversion="1"\n' },
    ...(csv === null ? [] : [{ path: PATH, text: csv }]),
  ]));
}

const dependencies = { base_files: { [PATH]: CORE }, packs: [
  { id: 'english-edit', files: { [PATH]: ENGLISH_OVERRIDE }, manifest_toml: '' },
] };

const COPY = {
  'workshop.localisation.entry': '{id} — {status}',
  'workshop.localisation.sources': 'English: {english}; translation: {translation}; winner: {winner}; provenance: {provenance}',
  'workshop.localisation.diagnostic': '{category} from {source}; winner: {winner}; shadowed: {shadowed}',
  'workshop.localisation.summary': '{count} authored strings checked for {locale}.',
  'workshop.localisation.category': '{category}: {count}',
  'workshop.localisation.no_locales': 'No usable translation locale is available.',
  'workshop.localisation.context': 'Context: {context}',
  'workshop.localisation.english': 'English: {value}',
  'workshop.localisation.preview': '{locale} preview: {value}',
};
const translate = (id, params = {}) => Object.entries(params)
  .reduce((text, [key, value]) => text.replaceAll(`{${key}}`, value), COPY[id] || id);

describe('Workshop localisation diagnostics and refresh', () => {
  it('keeps unrelated locales and metadata while recording source only after a real value edit', () => {
    const original = 'id,context,de,de_source,de_provenance,fr,fr_source\r\n'
      + 'station.helm.name,Bridge,Ruder,Helm,human,Barre,Helm\r\n';
    expect(editTranslationValue(original, 'station.helm.name', 'de', 'Ruder', 'Flight Control', 'Bridge', 'human'))
      .toBe(original);
    const edited = editTranslationValue(original, 'station.helm.name', 'de', 'Flugsteuerung',
      'Flight Control', 'Bridge', 'human');
    expect(edited).toContain('Flugsteuerung,Flight Control,human,Barre,Helm\r\n');
    expect(edited).toContain('\r\n');
  });
  it('renders an invalid-catalogue diagnostic for a transient unterminated draft', () => {
    const draft = documentFor('id,de,de_source\nfoo,"Hallo, Welt","Hello');
    const root = document.createElement('div');
    document.body.replaceChildren(root);
    const panel = mountWorkshopLocalisation({ root, draft: () => draft,
      dependencies: () => dependencies, t: translate });

    expect(() => panel.refresh()).not.toThrow();
    expect(root.querySelector('[data-category="invalid-catalogue"]')).not.toBeNull();
    expect(root.querySelector('#workshop-localisation-summary').textContent)
      .toBe('No usable translation locale is available.');
    expect(root.querySelector('#workshop-localisation-rows').children).toHaveLength(0);
  });

  it('shows missing, invalid, stale, conflict, winning source and provenance diagnostics', () => {
    const draft = documentFor();
    const root = document.createElement('div');
    document.body.appendChild(root);
    const panel = mountWorkshopLocalisation({ root, draft: () => draft,
      dependencies: () => dependencies, t: translate });
    panel.refresh();

    expect(root.querySelector('[data-string-id="station.helm.name"]').dataset.status).toBe('stale');
    expect(root.querySelector('[data-string-id="station.helm.name"]').textContent)
      .toContain('english-edit');
    expect(root.querySelector('[data-string-id="station.helm.name"]').textContent)
      .toContain('human');
    expect(root.querySelector('[data-category="conflicting-entry"]')).not.toBeNull();
    expect(root.querySelector('[data-category="missing-translation"]')).not.toBeNull();
    expect(root.querySelector('[data-category="invalid-translation"]')).not.toBeNull();
    expect(root.querySelector('[data-category="stale-translation"]')).not.toBeNull();
  });

  it('retains stale translation until explicit refresh and exports refreshed metadata', () => {
    const draft = documentFor();
    const changed = vi.fn();
    const root = document.createElement('div');
    document.body.appendChild(root);
    const panel = mountWorkshopLocalisation({ root, draft: () => draft,
      dependencies: () => dependencies, changed, t: translate });
    panel.refresh();
    expect(draft.read(PATH)).toContain('Ruder,Helm,human');

    root.querySelector('[data-refresh-source="station.helm.name"]').click();
    expect(changed).toHaveBeenCalledWith(PATH);
    expect(draft.read(PATH)).toContain('Ruder,Flight Control,human');

    const exported = new WorkshopDocument(draft.archive());
    const report = workshopCatalogueReport(dependencies, exported, 'de');
    expect(report.entries.get('station.helm.name')).toMatchObject({
      status: 'current', value: 'Ruder', provenance: 'human',
      englishSource: 'english-edit', winningSource: 'draft',
    });
  });

  it('imports a pack then edits German values and plural forms before export and reopen', () => {
    const draft = documentFor(null);
    const changed = vi.fn();
    const root = document.createElement('div');
    document.body.replaceChildren(root);
    const panel = mountWorkshopLocalisation({ root, draft: () => draft,
      dependencies: () => dependencies, changed, t: translate });
    panel.refresh();
    const get = id => root.querySelector(`#workshop-localisation-${id}`);
    get('new-locale').value = 'de';
    get('add-locale').click();
    expect(get('locale').value).toBe('de');
    const selectKey = id => {
      get('search').value = id;
      get('search').dispatchEvent(new Event('input'));
      get('keys').value = id;
      get('keys').dispatchEvent(new Event('change'));
    };
    selectKey('console.course');
    expect(get('editor').textContent).toContain('Helm readout');
    expect(get('editor').textContent).toContain('Course {degrees}');
    get('value').value = 'Kurs {wrong}';
    get('value').dispatchEvent(new Event('input'));
    get('save').click();
    expect(get('edit-status').textContent).toBe('workshop.localisation.parameter_error');
    expect(document.activeElement).toBe(get('value'));
    expect(draft.read(PATH)).toBeUndefined();

    get('value').value = 'Kurs {degrees}';
    get('value').dispatchEvent(new Event('input'));
    get('provenance').value = 'human';
    get('provenance').dispatchEvent(new Event('input'));
    get('save').click();
    expect(changed).toHaveBeenCalledWith(PATH);
    expect(get('preview').textContent).toContain('Kurs {degrees}');
    for (const [id, translated] of [['messages.one', '{n} Nachricht'],
      ['messages.other', '{n} Nachrichten']]) {
      selectKey(id);
      get('value').value = translated;
      get('value').dispatchEvent(new Event('input'));
      get('save').click();
    }
    expect(draft.canUndo()).toBe(true);
    expect(validatePartialStringCatalogue(draft.read(PATH)).filter(item => item.severity === 'error'))
      .toEqual([]);
    const reopened = new WorkshopDocument(draft.archive());
    const report = workshopCatalogueReport(dependencies, reopened, 'de');
    expect(report.entries.get('console.course')).toMatchObject({
      status: 'current', value: 'Kurs {degrees}', provenance: 'human', englishSource: 'core',
    });
    expect(report.entries.get('messages.one')).toMatchObject({ status: 'current', value: '{n} Nachricht' });
    expect(report.entries.get('messages.other')).toMatchObject({ status: 'current', value: '{n} Nachrichten' });
    expect(reopened.read('scenarios.toml')).toContain('id="de-pack"');
  });
});
