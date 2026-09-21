import {
  applyToDom, getCataloguePresentation, getLocale, setLocale,
} from './strings.js';

export const LOCALE_STORAGE_KEY = 'phoenix-private-locale';

function browserLocale(nav) {
  const raw = nav?.languages?.[0] || nav?.language || 'en';
  return String(raw).trim() || 'en';
}

export function loadPrivateLocale(storage, nav) {
  try {
    const saved = storage?.getItem(LOCALE_STORAGE_KEY);
    if (saved) return saved;
  } catch (_) { /* private storage may be unavailable */ }
  return browserLocale(nav);
}

export function persistPrivateLocale(storage, locale) {
  try { storage?.setItem(LOCALE_STORAGE_KEY, locale); } catch (_) { /* session still works */ }
}

export function matchAvailableLocale(requested, locales) {
  const values = Array.isArray(locales) ? locales : ['en'];
  const exact = values.find((value) => value.toLowerCase() === String(requested).toLowerCase());
  if (exact) return exact;
  const language = String(requested).split('-')[0].toLowerCase();
  return values.find((value) => value.toLowerCase().split('-')[0] === language) || 'en';
}

export function installPresentationInIframe(iframe, presentation = getCataloguePresentation(),
  { reload = false } = {}) {
  try {
    const install = iframe?.contentWindow?.phStringCatalogue?.install;
    if (typeof install !== 'function') return false;
    install(presentation);
    if (reload && typeof iframe.contentWindow?.location?.reload === 'function') {
      iframe.contentWindow.location.reload();
    }
    return true;
  } catch (_) { return false; }
}

export function installPresentationInConsoles(doc, presentation = getCataloguePresentation(), options) {
  if (!doc?.querySelectorAll) return 0;
  let installed = 0;
  for (const iframe of doc.querySelectorAll('.console-section iframe')) {
    if (installPresentationInIframe(iframe, presentation, options)) installed += 1;
  }
  return installed;
}

/** Owns the private language choice and fans it into every same-origin realm. */
export function createLocalePreference({ doc = document, nav = navigator, storage = localStorage,
  onChange = () => {}, findConsoles = null } = {}) {
  let requested = loadPrivateLocale(storage, nav);
  setLocale(requested);
  const apply = ({ reload = false } = {}) => {
    setLocale(matchAvailableLocale(requested, getCataloguePresentation().locales));
    const presentation = getCataloguePresentation();
    applyToDom(doc);
    const consoleRoot = typeof findConsoles === 'function'
      ? { querySelectorAll: () => findConsoles() }
      : doc;
    installPresentationInConsoles(consoleRoot, presentation, { reload });
    onChange(presentation);
    return presentation;
  };
  return {
    apply,
    locale: getLocale,
    presentation: getCataloguePresentation,
    select(next) {
      requested = String(next || 'en');
      const selected = setLocale(matchAvailableLocale(requested, getCataloguePresentation().locales));
      persistPrivateLocale(storage, requested);
      return apply({ reload: true });
    },
    installIframe(iframe) { return installPresentationInIframe(iframe); },
  };
}
