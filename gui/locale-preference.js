import {
  applyToDom, getCataloguePresentation, getLocale, localiseTree,
  rawDeliveredMessage, setLocale,
} from './strings.js';

export const LOCALE_STORAGE_KEY = 'phoenix-private-locale';
export const GM_LOCALE_STORAGE_KEY = 'phoenix-private-gm-locale';
export const WORKSHOP_LOCALE_STORAGE_KEY = 'phoenix-private-workshop-locale';

function browserLocale(nav) {
  const raw = nav?.languages?.[0] || nav?.language || 'en';
  return String(raw).trim() || 'en';
}

export function loadPrivateLocale(storage, nav, key = LOCALE_STORAGE_KEY) {
  try {
    const saved = storage?.getItem(key);
    if (saved) return saved;
  } catch (_) { /* private storage may be unavailable */ }
  return browserLocale(nav);
}

export function persistPrivateLocale(storage, locale, key = LOCALE_STORAGE_KEY) {
  try { storage?.setItem(key, locale); } catch (_) { /* session still works */ }
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
    if (!reload) iframe.contentWindow.__languageSwitchPending = true;
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
  onChange = () => {}, findConsoles = null, storageKey = LOCALE_STORAGE_KEY } = {}) {
  let requested = loadPrivateLocale(storage, nav, storageKey);
  let rawComms = null;
  let rawObjectives = null;
  setLocale(requested);
  const apply = ({ reload = false, preview = null } = {}) => {
    setLocale(matchAvailableLocale(preview || requested, getCataloguePresentation().locales));
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
    /** Capture only full-replacement presentation payloads, never authority. */
    capture(delivered) {
      const raw = rawDeliveredMessage(delivered);
      if (raw?.type === 'Welcome') {
        rawComms = null;
        rawObjectives = null;
      } else if (raw?.type === 'CommsState') {
        rawComms = raw;
        rawObjectives = raw;
      } else if (raw?.type === 'ObjectiveSummary') {
        rawObjectives = raw;
      }
    },
    /** Re-render retained prose without replaying commands or reducer effects. */
    relocaliseRetained({ simState, commsState } = {}) {
      if (rawComms) {
        const data = localiseTree(rawComms).data || {};
        if (commsState) {
          commsState.messages = data.messages || [];
          commsState.objectives = data.objectives || [];
          commsState.contacts = data.contacts || [];
          commsState.version += 1;
        }
      }
      if (rawObjectives && simState) {
        simState.objectives = localiseTree(rawObjectives).data?.objectives || [];
      }
    },
    select(next) {
      requested = String(next || 'en');
      setLocale(matchAvailableLocale(requested, getCataloguePresentation().locales));
      persistPrivateLocale(storage, requested, storageKey);
      return apply();
    },
    /** Temporary presentation for an editor preview; leaves the saved choice alone. */
    preview(next) { return apply({ preview: next }); },
    installIframe(iframe) { return installPresentationInIframe(iframe); },
  };
}
