// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  createLocalePreference, installPresentationInIframe, LOCALE_STORAGE_KEY,
} from '../../gui/locale-preference.js';
import {
  installCataloguePresentation, setBaseCatalogue, setLocale, setOverlayCatalogues,
} from '../../gui/strings.js';
import { localiseDeliveredMessage } from '../../gui/rendezvous-transport.js';
import { preserveConsoleEditContext } from '../../gui/console-core.js';
import { ClientSimState } from '../../gui/sim-state.js';
import { ClientCommsState } from '../../gui/comms-state.js';
import '../../gui/components/ph-battery-bar.js';
import '../../gui/components/ph-boost-btn.js';

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

  it('updates an already-mounted Console component without reloading its realm', () => {
    setBaseCatalogue(CORE);
    setLocale('en');
    setOverlayCatalogues([{ source: 'de-pack', csv: DE }]);
    let component = document.createElement('ph-battery-bar');
    document.body.appendChild(component);
    expect(component.shadowRoot.getElementById('charging-indicator').textContent).toBe('Charging');

    const reload = vi.fn();
    const iframe = { contentWindow: {
      phStringCatalogue: { install(presentation) {
        installCataloguePresentation(presentation);
        component.refreshLocale();
      } },
      location: { reload },
    } };
    const preference = createLocalePreference({ doc: document, nav: { language: 'en' },
      storage: localStorage, findConsoles: () => [iframe] });
    preference.select('de');
    expect(reload).not.toHaveBeenCalled();
    expect(component.shadowRoot.getElementById('charging-indicator').textContent).toBe('Lädt');
  });

  it('refreshes an id-less title in a mounted component nested in a shadow root', () => {
    const core = 'id,en\ncomponent.boost.title,Boost\ncomponent.boost.binding,Shift\n'
      + 'component.boost.ready,Ready\ncomponent.boost.boosting_full,Boosting\n'
      + 'console.common.auto,Auto\n';
    const de = 'id,de,de_source\ncomponent.boost.title,Schubverstärkung,Boost\n';
    setBaseCatalogue(core);
    setLocale('en');
    const outer = document.createElement('div');
    document.body.appendChild(outer);
    const shadow = outer.attachShadow({ mode: 'open' });
    const boost = document.createElement('ph-boost-btn');
    shadow.appendChild(boost);
    const title = boost.shadowRoot.querySelector('.header span');
    const button = boost.shadowRoot.getElementById('btn');
    expect(title.textContent).toBe('Boost');
    window.phStringCatalogue.install({ locale: 'de', catalogues: [{ source: 'de-pack', csv: de }] });
    expect(title.textContent).toBe('Schubverstärkung');
    expect(boost.shadowRoot.getElementById('btn')).toBe(button);
  });

  it('relocalises retained Objective and Comms copy from semantic wire values', () => {
    const core = 'id,en\nobjective.text,Hold {n} ships\ncomms.body,Meet at {n}\n';
    const de = 'id,de,de_source\n'
      + 'objective.text,{n} Schiffe halten,Hold {n} ships\n'
      + 'comms.body,Treffen bei {n},Meet at {n}\n';
    setBaseCatalogue(core);
    setOverlayCatalogues([{ source: 'de-pack', csv: de }]);
    const preference = createLocalePreference({ doc: document, nav: { language: 'en' },
      storage: localStorage });
    const raw = { type: 'CommsState', data: {
      objectives: [{ id: 'obj', text: 'objective.text', text_params: { n: 2 } }],
      messages: [{ id: 'msg', body: 'comms.body', body_params: { n: 1200 },
        literal_body: false }, { id: 'user', body: 'comms.body', literal_body: true }],
      contacts: [],
    } };
    const delivered = localiseDeliveredMessage(raw);
    preference.capture(delivered);
    const simState = new ClientSimState();
    simState.stationHosts = { helm: 'self' };
    simState.apply(delivered);
    const commsState = new ClientCommsState();
    commsState.apply(delivered);
    commsState.selectThread('msg');
    preference.select('de');
    preference.relocaliseRetained({ simState, commsState });
    expect(simState.objectives[0].text).toBe('2 Schiffe halten');
    expect(commsState.messages[0].body).toBe('Treffen bei 1.200');
    expect(commsState.messages[1].body).toBe('comms.body');
    expect(commsState.selectedThreadId).toBe('msg');
    expect(commsState.version).toBeGreaterThan(1);
    expect(simState.stationHosts).toEqual({ helm: 'self' });
    preference.select('en');
    preference.relocaliseRetained({ simState, commsState });
    expect(simState.objectives[0].text).toBe('Hold 2 ships');
    expect(commsState.messages[0].body).toBe('Meet at 1,200');
    const newer = localiseDeliveredMessage({ type: 'CommsState', data: {
      objectives: [{ id: 'obj', text: 'objective.text', text_params: { n: 3 } }],
      messages: [{ id: 'new', body: 'comms.body', body_params: { n: 4 } }], contacts: [],
    } });
    preference.capture(newer);
    simState.apply(newer);
    commsState.apply(newer);
    preference.select('de');
    preference.relocaliseRetained({ simState, commsState });
    expect(commsState.messages[0].body).toBe('Treffen bei 4');
    expect(simState.objectives[0].text).toBe('3 Schiffe halten');
    const summary = localiseDeliveredMessage({ type: 'ObjectiveSummary', data: {
      objectives: [{ id: 'obj', text: 'objective.text', text_params: { n: 5 } }],
    } });
    preference.capture(summary);
    simState.apply(summary);
    preference.relocaliseRetained({ simState, commsState });
    expect(simState.objectives[0].text).toBe('5 Schiffe halten');
    expect(commsState.messages[0].body).toBe('Treffen bei 4');
  });

  it('preserves a pending form value, focus, selection and scroll during locale repaint', () => {
    document.body.innerHTML = '<div id="log"><input id="draft" value="old"></div>';
    const input = document.getElementById('draft');
    input.value = 'Änderung pending';
    input.focus();
    input.setSelectionRange(3, 8);
    document.getElementById('log').scrollTop = 80;
    preserveConsoleEditContext(document, () => {
      document.getElementById('log').innerHTML = '<input id="draft" value="new">';
      document.getElementById('log').scrollTop = 0;
    });
    expect(document.getElementById('draft').value).toBe('Änderung pending');
    expect(document.activeElement).toBe(document.getElementById('draft'));
    expect(document.activeElement.selectionStart).toBe(3);
    expect(document.getElementById('log').scrollTop).toBe(80);
  });

  it('preserves an active control inside a Console component shadow root', () => {
    const host = document.createElement('div');
    document.body.appendChild(host);
    const shadow = host.attachShadow({ mode: 'open' });
    shadow.innerHTML = '<input id="entry">';
    const original = shadow.getElementById('entry');
    original.value = 'Überprüfung';
    original.focus();
    preserveConsoleEditContext(document, () => { shadow.innerHTML = '<input id="entry">'; });
    expect(shadow.getElementById('entry').value).toBe('Überprüfung');
    expect(shadow.activeElement).toBe(shadow.getElementById('entry'));
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
