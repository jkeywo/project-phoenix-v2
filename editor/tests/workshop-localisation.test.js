// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest';
import { WorkshopDocument } from '../workshop-document.js';
import { createStoreZip } from '../mod-pack-export.js';
import { mountWorkshopLocalisation } from '../../gui/workshop-localisation-panel.js';
import { workshopCatalogueReport } from '../workshop-localisation.js';

const PATH = 'assets/strings/strings.csv';
const CORE = 'id,en\nstation.helm.name,Helm\nconsole.course,"Course {degrees}"\ncore.only,Core only\n';
const ENGLISH_OVERRIDE = 'id,en\nstation.helm.name,Flight Control\n';
const DRAFT = 'id,context,de,de_source,de_provenance\n'
  + 'station.helm.name,tab,Ruder,Helm,human\n'
  + 'console.course,course,Kurs,Course {degrees},machine\n';

function documentFor(csv = DRAFT) {
  return new WorkshopDocument(createStoreZip([
    { path: 'scenarios.toml', text: '[pack]\nformat=1\nid="de-pack"\nname="German"\nversion="1"\n' },
    { path: PATH, text: csv },
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
};
const translate = (id, params = {}) => Object.entries(params)
  .reduce((text, [key, value]) => text.replaceAll(`{${key}}`, value), COPY[id] || id);

describe('Workshop localisation diagnostics and refresh', () => {
  it('renders an invalid-catalogue diagnostic for a transient unterminated draft', () => {
    const draft = documentFor('id,de,de_source\nfoo,"Hallo, Welt","Hello');
    const root = document.createElement('div');
    document.body.appendChild(root);
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
});
