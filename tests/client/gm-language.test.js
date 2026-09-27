// @vitest-environment jsdom
import { afterEach, expect, it, vi } from 'vitest';
import { mountGmLanguage } from '../../gui/gm-language.js';
import { GM_LOCALE_STORAGE_KEY } from '../../gui/locale-preference.js';
import { getLocale, setBaseCatalogue, setLocale, setOverlayCatalogues } from '../../gui/strings.js';

afterEach(() => {
  document.documentElement.classList.remove('phoenix-gm-page');
  document.body.replaceChildren();
  localStorage.clear();
  setLocale('en');
  setOverlayCatalogues([]);
});

it('restores the host page locale when a browser GM exits and reuses the GM choice on return', () => {
  setBaseCatalogue('id,en\nserver.gm.console.title,Game Master\nsettings.language,Language\n'
    + 'settings.language.private_hint,Saved locally\n');
  setOverlayCatalogues([{ source: 'de-fixture', csv: 'id,de,de_source\n'
    + 'server.gm.console.title,Spielleitung,Game Master\n'
    + 'settings.language,Sprache,Language\n' }]);
  setLocale('en');
  document.body.innerHTML = '<main id="gm-console"><h1 data-i18n="server.gm.console.title">Game Master</h1>'
    + '<div id="gm-language-control"></div></main>';
  const workspace = { refreshLanguage: vi.fn() };
  const boundary = mountGmLanguage({ doc: document, win: window, nav: { language: 'en' },
    storage: localStorage, workspace });
  expect(boundary.preference()).toBeNull();
  document.documentElement.classList.add('phoenix-gm-page');
  window.dispatchEvent(new Event('phoenix-gm-page-changed'));
  const select = document.getElementById('gm-language');
  select.value = 'de'; select.dispatchEvent(new Event('change'));
  expect(getLocale()).toBe('de');
  expect(document.querySelector('h1').textContent).toBe('Spielleitung');
  expect(localStorage.getItem(GM_LOCALE_STORAGE_KEY)).toBe('de');
  document.documentElement.classList.remove('phoenix-gm-page');
  window.dispatchEvent(new Event('phoenix-gm-page-changed'));
  expect(getLocale()).toBe('en');
  expect(document.querySelector('h1').textContent).toBe('Game Master');
  expect(localStorage.getItem(GM_LOCALE_STORAGE_KEY)).toBe('de');
  document.documentElement.classList.add('phoenix-gm-page');
  window.dispatchEvent(new Event('phoenix-gm-page-changed'));
  expect(getLocale()).toBe('de');
  expect(select.value).toBe('de');
  expect(workspace.refreshLanguage).toHaveBeenCalled();
  boundary.dispose();
});
