// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { renderHostLanding } from '../../gui/host-landing-render.js';
import { landingViewModel } from '../../gui/host-landing-view.js';
import { renderHostScenarios } from '../../gui/host-scenario-render.js';
import { scenarioCatalogView } from '../../gui/host-scenarios.js';
import { applyToDom, setBaseCatalogue, setLocale, setOverlayCatalogues, t } from '../../gui/strings.js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const markup = fs.readFileSync(path.join(root, 'server.html'), 'utf8');
const core = fs.readFileSync(path.join(root, 'assets/strings/strings.csv'), 'utf8');
const german = 'id,de,de_source,de_provenance\n'
  + 'server.landing.new_game,Neues Spiel,[New Game],machine\n'
  + 'server.select_world,Welt auswählen,[Select a world],machine\n'
  + 'server.launch_ai_ship,KI-Schiff starten,⬡ Launch AI Ship,machine\n';

afterEach(() => { setOverlayCatalogues([]); setLocale('en'); });

describe('operator setup to launch language', () => {
  it('renders German setup and selection while an untranslated ship stage falls back to English', () => {
    setBaseCatalogue(core);
    setOverlayCatalogues([
      { source: 'fixture-de', csv: german },
      { source: 'fixture-untranslated', csv: 'id,en\nserver.select_ship,[Choose a ship to launch]\n' },
    ]);
    setLocale('de');
    const parsed = new DOMParser().parseFromString(markup, 'text/html');
    const doc = document.implementation.createHTMLDocument('');
    for (const id of ['landing-panel', 'scenario-panel', 'lobby-panel']) {
      doc.body.appendChild(doc.importNode(parsed.getElementById(id), true));
    }
    renderHostLanding(doc, landingViewModel(), t);
    expect(doc.querySelector('[data-landing-entry="new_game"]').textContent).toContain('Neues Spiel');

    const selected = [];
    const catalogue = [{ id: 'combat_test', world: 'assets/worlds/combat_test.toml',
      label: 'world.combat_test.title', ships: [
        { template_path: 'assets/entities/alliance_destroyer.toml', label: 'Destroyer' },
        { template_path: 'assets/entities/alliance_cruiser.toml', label: 'Cruiser' },
      ] }];
    const hooks = { selectScenario: (id) => selected.push(id), shipStillNeeded: () => true };
    renderHostScenarios(doc, scenarioCatalogView(catalogue, null, false), t, hooks);
    expect(doc.getElementById('world-list-label').textContent).toBe('Welt auswählen');
    doc.querySelector('[data-scenario-id="combat_test"]').click();
    expect(selected).toEqual(['combat_test']);
    expect(scenarioCatalogView(catalogue, { scenario_id: 'combat_test' }, false).labelId)
      .toBe('server.select_ship');
    expect(t('server.select_ship')).toBe('[Choose a ship to launch]');

    applyToDom(doc);
    expect(doc.getElementById('ai-launch-btn').textContent).toBe('KI-Schiff starten');
  });
});
