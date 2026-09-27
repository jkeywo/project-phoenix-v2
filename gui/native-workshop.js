/** Shared offline UI on the native host's private Authoring bridge. The shell
 * calls this module; the ordinary browser boot never imports native authority. */
import { createNativeWorkshopProvider, createWorkshopBridge } from '../editor/workshop-provider.js';
import { mountWorkshopAuthoring } from './workshop-authoring.js';
import { createLocalePreference, WORKSHOP_LOCALE_STORAGE_KEY } from './locale-preference.js';
import { mountSurfaceLanguage } from './surface-language.js';

export function mountNativeWorkshop({ root, send, win = window, launch = null }) {
  const bridge = createWorkshopBridge({ send });
  const provider = createNativeWorkshopProvider({ request: bridge.request });
  let authoring = null;
  const locale = createLocalePreference({ doc: root.ownerDocument,
    nav: win.PhoenixOsLocale ? { nativeLocale: win.PhoenixOsLocale } : win.navigator,
    storage: win.PhoenixLocaleStorage || win.localStorage, storageKey: WORKSHOP_LOCALE_STORAGE_KEY,
    findConsoles: () => [], onChange: () => authoring?.refreshLanguage() });
  const languageControl = mountSurfaceLanguage({ doc: root.ownerDocument,
    id: 'workshop-language', preference: locale });
  authoring = mountWorkshopAuthoring({ root, win, provider, launch, languageControl });
  locale.apply();
  win.phWorkshopLanguage = { preview: next => locale.preview(next), restore: () => locale.apply(),
    select: next => locale.select(next), presentation: () => locale.presentation() };
  return { ready: authoring.ready, receive: bridge.receive,
    dispose() { authoring.dispose(); bridge.dispose(); } };
}
