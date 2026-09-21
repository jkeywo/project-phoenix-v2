// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  createLocalePreference, installPresentationInIframe, LOCALE_STORAGE_KEY,
} from '../../gui/locale-preference.js';
import {
  installCataloguePresentation, setBaseCatalogue, setLocale, setOverlayCatalogues,
} from '../../gui/strings.js';
import '../../gui/components/ph-battery-bar.js';

const CORE = 'id,en\ncomponent.battery_bar.charging,Charging\n';
const DE = 'id,de,de_source,de_provenance\ncomponent.battery_bar.charging,Lädt,Charging,human\n';

afterEach(() => {
  setLocale('en');
  setOverlayCatalogues([]);
  document.body.replaceChildren();
  localStorage.clear();
});

describe('private locale delivery to console realms', () => {
  it('uses the browser language, persists an explicit choice, and installs the ordered stack', () => {
    setBaseCatalogue(CORE);
    setOverlayCatalogues([{ source: 'de-pack', csv: DE }]);
    const preference = createLocalePreference({ doc: document,
      nav: { languages: ['de-DE'] }, storage: localStorage });
    preference.apply();
    expect(preference.locale()).toBe('de');
    preference.select('de');
    expect(localStorage.getItem(LOCALE_STORAGE_KEY)).toBe('de');
    expect(preference.presentation()).toMatchObject({ locale: 'de',
      catalogues: [{ source: 'de-pack', csv: DE }] });
  });

  it('installs before a representative Console component is constructed', () => {
    setBaseCatalogue(CORE);
    setOverlayCatalogues([{ source: 'de-pack', csv: DE }]);
    setLocale('de');
    const component = document.createElement('ph-battery-bar');
    document.body.appendChild(component);
    expect(component.shadowRoot.getElementById('charging-indicator').textContent).toBe('Lädt');
  });

  it('rebuilds an already-mounted real Console component after selecting German', () => {
    setBaseCatalogue(CORE);
    setLocale('en');
    setOverlayCatalogues([{ source: 'de-pack', csv: DE }]);
    let component = document.createElement('ph-battery-bar');
    document.body.appendChild(component);
    expect(component.shadowRoot.getElementById('charging-indicator').textContent).toBe('Charging');

    const reload = vi.fn(() => {
      component.remove();
      component = document.createElement('ph-battery-bar');
      document.body.appendChild(component);
    });
    const iframe = { contentWindow: {
      phStringCatalogue: { install: installCataloguePresentation },
      location: { reload },
    } };
    const preference = createLocalePreference({ doc: document, nav: { language: 'en' },
      storage: localStorage, findConsoles: () => [iframe] });
    preference.select('de');
    expect(reload).toHaveBeenCalledOnce();
    expect(component.shadowRoot.getElementById('charging-indicator').textContent).toBe('Lädt');
  });

  it('hands locale and catalogues to a mounted or reloaded iframe realm', () => {
    const install = vi.fn();
    const iframe = document.createElement('iframe');
    document.body.appendChild(iframe);
    iframe.contentWindow.phStringCatalogue = { install };
    const presentation = { locale: 'de', catalogues: [{ source: 'de-pack', csv: DE }] };
    expect(installPresentationInIframe(iframe, presentation)).toBe(true);
    expect(install).toHaveBeenCalledWith(presentation);
  });
});
