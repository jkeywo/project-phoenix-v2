import { applyToDom, getLocale, setLocale } from './strings.js';
import { createLocalePreference, GM_LOCALE_STORAGE_KEY } from './locale-preference.js';
import { mountSurfaceLanguage } from './surface-language.js';

/** Keep a browser GM's private language inside GM mode on the shared host page. */
export function mountGmLanguage({ doc = document, win = window, nav = navigator,
  storage = localStorage, workspace } = {}) {
  let hostLocale = null;
  let gmLocale = null;
  const applyMode = () => {
    if (!doc.documentElement.classList.contains('phoenix-gm-page')) {
      if (hostLocale !== null) {
        setLocale(hostLocale);
        applyToDom(doc);
        hostLocale = null;
      }
      return;
    }
    if (hostLocale === null) hostLocale = getLocale();
    if (!gmLocale) {
      gmLocale = createLocalePreference({ doc, nav, storage,
        storageKey: GM_LOCALE_STORAGE_KEY, findConsoles: () => [],
        onChange: () => workspace.refreshLanguage() });
      const language = mountSurfaceLanguage({ doc, id: 'gm-language', preference: gmLocale });
      doc.getElementById('gm-language-control').append(language.root);
    }
    gmLocale.apply();
  };
  win.addEventListener('phoenix-gm-page-changed', applyMode);
  applyMode();
  return { applyMode, preference: () => gmLocale,
    dispose() { win.removeEventListener('phoenix-gm-page-changed', applyMode); } };
}
