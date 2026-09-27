import './strings-boot.js';
import { applyToDom, t } from './strings.js';
import { mountWorkshopAuthoring } from './workshop-authoring.js';
import { createBrowserWorkshopProvider } from '../editor/workshop-provider.js';
import { createWorkshopHandoffStore } from '../editor/workshop-handoff.js';
import { workshopSourceProvider } from '../editor/workshop-source-provider.js';
import { parseWorkshopLaunch } from '../editor/workshop-launch.js';
import { createLocalePreference, WORKSHOP_LOCALE_STORAGE_KEY } from './locale-preference.js';
import { mountSurfaceLanguage } from './surface-language.js';

applyToDom(document);
async function openWorkshop() {
  const root = document.getElementById('workshop');
  const url = new URL(window.location.href);
  const fragment = new URLSearchParams(url.hash.slice(1));
  const token = fragment.get('source');
  const launch = parseWorkshopLaunch(fragment);
  let provider, failed = false;
  if (token) {
    fragment.delete('source');
    url.hash = fragment.toString();
    window.history.replaceState(window.history.state, '', url);
    try {
      const source = await createWorkshopHandoffStore().take(token);
      if (!source) throw Error('Source transfer expired or was already opened');
      provider = workshopSourceProvider(source);
    } catch { failed = true; }
  }
  let workshop = null;
  const locale = createLocalePreference({ doc: document, nav: navigator, storage: localStorage,
    storageKey: WORKSHOP_LOCALE_STORAGE_KEY, findConsoles: () => [],
    onChange: () => workshop?.refreshLanguage() });
  const languageControl = mountSurfaceLanguage({ doc: document, id: 'workshop-language', preference: locale });
  workshop = mountWorkshopAuthoring({ root, provider: provider || createBrowserWorkshopProvider(), launch,
    languageControl });
  locale.apply();
  // The translation editor can sample a locale and restore the private choice
  // without writing storage or rebuilding the draft.
  window.phWorkshopLanguage = { preview: next => locale.preview(next), restore: () => locale.apply(),
    select: next => locale.select(next), presentation: () => locale.presentation() };
  if (failed) {
    const message = document.createElement('p');
    message.setAttribute('role', 'alert'); message.dataset.i18n = 'workshop.source_failed';
    message.textContent = t('workshop.source_failed'); root.prepend(message);
  }
}
openWorkshop();
