import { t } from './strings.js';

/** A private presentation choice shared by the GM and Workshop surfaces. */
export function mountSurfaceLanguage({ doc, id, preference, labelId = 'settings.language' }) {
  const label = doc.createElement('label');
  label.htmlFor = id;
  label.dataset.i18n = labelId;
  const select = doc.createElement('select');
  select.id = id;
  const hint = doc.createElement('p');
  hint.dataset.i18n = 'settings.language.private_hint';
  const root = doc.createElement('div');
  root.className = 'surface-language';
  root.append(label, select, hint);
  const refresh = () => {
    label.textContent = t(labelId);
    hint.textContent = t('settings.language.private_hint');
    const locales = preference.presentation().locales;
    const current = preference.locale();
    select.replaceChildren(...locales.map((locale) => {
      const option = doc.createElement('option');
      option.value = locale;
      option.textContent = locale === 'en' ? 'English' : locale === 'de' ? 'Deutsch' : locale;
      return option;
    }));
    select.value = current;
  };
  select.addEventListener('change', () => {
    preference.select(select.value);
    refresh();
    select.focus({ preventScroll: true });
  });
  refresh();
  return { root, refresh, select };
}
