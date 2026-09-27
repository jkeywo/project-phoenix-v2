import { editTranslationValue, refreshTranslationSource, workshopCatalogueReport,
  workshopStringContexts } from '../editor/workshop-localisation.js';
import { STRING_CATALOGUE_PATH } from './string-catalogue.js';

export function mountWorkshopLocalisation({ root, draft, dependencies, changed, t, attach = true }) {
  const doc = root.ownerDocument;
  const node = doc.createElement('section');
  node.className = 'workshop-localisation';
  const label = doc.createElement('label');
  label.textContent = t('workshop.localisation.locale');
  const locale = doc.createElement('select');
  locale.id = 'workshop-localisation-locale';
  label.htmlFor = locale.id;
  const summary = doc.createElement('p');
  summary.id = 'workshop-localisation-summary';
  summary.setAttribute('role', 'status');
  const rows = doc.createElement('div');
  rows.id = 'workshop-localisation-rows';
  const diagnosticSummary = doc.createElement('div');
  diagnosticSummary.id = 'workshop-localisation-diagnostics';
  const searchLabel = doc.createElement('label');
  searchLabel.htmlFor = 'workshop-localisation-search';
  const search = doc.createElement('input');
  search.id = 'workshop-localisation-search'; search.type = 'search';
  const results = doc.createElement('select');
  results.id = 'workshop-localisation-keys'; results.size = 8;
  const editor = doc.createElement('section');
  editor.id = 'workshop-localisation-editor';
  const context = doc.createElement('p');
  const english = doc.createElement('p');
  const sources = doc.createElement('p');
  const entryDiagnostics = doc.createElement('p');
  const preview = doc.createElement('p');
  preview.id = 'workshop-localisation-preview';
  const valueLabel = doc.createElement('label');
  valueLabel.htmlFor = 'workshop-localisation-value';
  const translation = doc.createElement('textarea');
  translation.id = 'workshop-localisation-value'; translation.rows = 3;
  const provenanceLabel = doc.createElement('label');
  provenanceLabel.htmlFor = 'workshop-localisation-provenance';
  const provenance = doc.createElement('input');
  provenance.id = 'workshop-localisation-provenance';
  const save = doc.createElement('button'); save.type = 'button';
  save.id = 'workshop-localisation-save';
  const editStatus = doc.createElement('p');
  editStatus.id = 'workshop-localisation-edit-status'; editStatus.setAttribute('role', 'status');
  editor.append(context, english, sources, entryDiagnostics, preview, valueLabel, translation,
    provenanceLabel, provenance, save, editStatus);
  const newLocaleLabel = doc.createElement('label');
  newLocaleLabel.htmlFor = 'workshop-localisation-new-locale';
  const newLocale = doc.createElement('input'); newLocale.id = 'workshop-localisation-new-locale';
  const addLocale = doc.createElement('button'); addLocale.type = 'button';
  addLocale.id = 'workshop-localisation-add-locale';
  node.append(label, locale, newLocaleLabel, newLocale, addLocale,
    summary, diagnosticSummary, searchLabel, search, results, editor, rows);
  if (attach) root.appendChild(node);

  const detailLine = (id, params) => {
    const p = doc.createElement('p');
    p.textContent = t(id, params);
    return p;
  };

  const showDiagnostics = (report) => {
    const counts = new Map();
    for (const finding of report.diagnostics) {
      counts.set(finding.category, (counts.get(finding.category) || 0) + 1);
    }
    diagnosticSummary.replaceChildren(...[...counts].sort().map(([category, count]) => {
      const line = detailLine('workshop.localisation.category', {
        category, count: String(count),
      });
      line.dataset.category = category;
      return line;
    }));
  };

  let selectedId = '';
  let editingFor = '';
  let editorDirty = false;
  let preferredLocale = '';
  let currentReport = null;
  let contexts = new Map();
  const labels = () => {
    label.textContent = t('workshop.localisation.locale');
    searchLabel.textContent = t('workshop.localisation.search');
    results.setAttribute('aria-label', t('workshop.localisation.keys'));
    newLocaleLabel.textContent = t('workshop.localisation.new_locale');
    addLocale.textContent = t('workshop.localisation.add_locale');
    valueLabel.textContent = t('workshop.localisation.value');
    provenanceLabel.textContent = t('workshop.localisation.provenance');
    save.textContent = t('workshop.localisation.save');
  };

  function paintEditor() {
    const entry = currentReport?.entries?.get(selectedId);
    editor.hidden = !entry || !locale.value;
    if (editor.hidden) return;
    const key = `${locale.value}:${selectedId}`;
    context.textContent = t('workshop.localisation.context', { context: contexts.get(selectedId) || '—' });
    english.textContent = t('workshop.localisation.english', { value: entry.english });
    sources.textContent = t('workshop.localisation.sources', {
      english: entry.englishSource, translation: entry.translationSource || '—',
      winner: entry.winningSource, provenance: entry.provenance || '—',
    });
    entryDiagnostics.textContent = currentReport.diagnostics.filter(finding => finding.id === selectedId)
      .map(finding => t('workshop.localisation.diagnostic', {
        category: finding.category, source: finding.source,
        winner: finding.winner || '—', shadowed: (finding.shadowed || []).join(', ') || '—',
      })).join(' ');
    preview.textContent = t('workshop.localisation.preview', {
      english: entry.english, locale: locale.value, value: entry.value,
    });
    if (editingFor !== key || !editorDirty) {
      translation.value = entry.translation || '';
      provenance.value = entry.provenance || '';
      editingFor = key;
      editorDirty = false;
    }
    save.disabled = !selectedId;
  }

  function paintChoices() {
    if (!currentReport) { results.replaceChildren(); editor.hidden = true; return; }
    const query = search.value.trim().toLocaleLowerCase();
    const ids = [...currentReport.entries.keys()]
      .filter(id => !query || `${id} ${contexts.get(id) || ''}`.toLocaleLowerCase().includes(query));
    results.replaceChildren(...ids.slice(0, 80).map(id => {
      const option = doc.createElement('option'); option.value = id;
      option.textContent = `${id} — ${contexts.get(id) || ''}`;
      return option;
    }));
    if (!ids.includes(selectedId)) selectedId = ids[0] || '';
    if (selectedId && ![...results.options].some(option => option.value === selectedId)) {
      const option = doc.createElement('option'); option.value = selectedId;
      option.textContent = selectedId; results.prepend(option);
    }
    results.value = selectedId;
    paintEditor();
  }

  function saveEdit() {
    const value = draft?.();
    const deps = dependencies?.();
    const entry = currentReport?.entries?.get(selectedId);
    if (!value || !deps || !entry || !locale.value) return;
    let after;
    try {
      after = editTranslationValue(value.read(STRING_CATALOGUE_PATH) || '', selectedId,
        locale.value, translation.value, entry.english, contexts.get(selectedId) || '', provenance.value);
    } catch {
      editStatus.textContent = t('workshop.localisation.edit_error');
      return;
    }
    const candidate = { read: path => path === STRING_CATALOGUE_PATH ? after : value.read(path) };
    const result = workshopCatalogueReport(deps, candidate, locale.value).entries.get(selectedId);
    if (translation.value.trim() && result?.status === 'invalid') {
      editStatus.textContent = t('workshop.localisation.parameter_error');
      translation.focus();
      return;
    }
    const before = value.read(STRING_CATALOGUE_PATH);
    if ((before === undefined ? value.put(STRING_CATALOGUE_PATH, after)
      : value.edit(STRING_CATALOGUE_PATH, after))) {
      editorDirty = false;
      changed?.(STRING_CATALOGUE_PATH);
      refresh();
      editStatus.textContent = t('workshop.localisation.saved');
    }
  }

  function refresh() {
    const value = draft?.();
    const deps = dependencies?.();
    labels();
    if (!value || !deps) {
      locale.replaceChildren();
      rows.replaceChildren();
      diagnosticSummary.replaceChildren();
      currentReport = null;
      paintChoices();
      summary.textContent = t('workshop.localisation.unavailable');
      return;
    }
    const discoveryReport = workshopCatalogueReport(deps, value, 'en');
    const discovered = discoveryReport.locales.filter((item) => item !== 'en');
    showDiagnostics(discoveryReport);
    const selected = preferredLocale || (discovered.includes(locale.value) ? locale.value : discovered[0] || '');
    const choices = [...new Set([...discovered, ...(selected ? [selected] : [])])];
    locale.replaceChildren(...choices.map((name) => {
      const option = doc.createElement('option'); option.value = name; option.textContent = name; return option;
    }));
    locale.value = selected;
    locale.disabled = !selected;
    contexts = workshopStringContexts(deps, value);
    if (!selected) {
      rows.replaceChildren();
      currentReport = discoveryReport;
      paintChoices();
      summary.textContent = t('workshop.localisation.no_locales');
      return;
    }
    const report = workshopCatalogueReport(deps, value, selected);
    currentReport = report;
    summary.textContent = t('workshop.localisation.summary', {
      locale: selected, count: report.authored.length,
    });
    showDiagnostics(report);
    paintChoices();
    rows.replaceChildren(...report.authored.map(({ entry, diagnostics }) => {
      const article = doc.createElement('article');
      article.className = 'workshop-localisation-entry';
      article.dataset.stringId = entry?.id || diagnostics[0]?.id || '';
      article.dataset.status = entry?.status || 'invalid';
      const heading = doc.createElement('h3');
      heading.textContent = t('workshop.localisation.entry', {
        id: article.dataset.stringId, status: entry?.status || 'invalid',
      });
      article.append(heading);
      if (entry) {
        article.append(detailLine('workshop.localisation.sources', {
          english: entry.englishSource, translation: entry.translationSource || '—',
          winner: entry.winningSource, provenance: entry.provenance || '—',
        }));
      }
      for (const finding of diagnostics) {
        const line = detailLine('workshop.localisation.diagnostic', {
          category: finding.category, source: finding.source,
          winner: finding.winner || '—', shadowed: (finding.shadowed || []).join(', ') || '—',
        });
        line.dataset.category = finding.category;
        article.append(line);
      }
      if (entry?.translation && entry.refreshable) {
        const accept = doc.createElement('button');
        accept.type = 'button';
        accept.dataset.refreshSource = entry.id;
        accept.textContent = t('workshop.localisation.refresh');
        accept.addEventListener('click', () => {
          const before = value.read(STRING_CATALOGUE_PATH);
          const after = refreshTranslationSource(before, entry.id, selected, entry.english);
          if (after !== before && value.edit(STRING_CATALOGUE_PATH, after)) {
            changed?.(STRING_CATALOGUE_PATH);
          }
        });
        article.append(accept);
      }
      return article;
    }));
  }
  locale.addEventListener('change', () => { preferredLocale = locale.value; editorDirty = false; refresh(); });
  search.addEventListener('input', paintChoices);
  results.addEventListener('change', () => { selectedId = results.value; editorDirty = false; paintEditor(); });
  translation.addEventListener('input', () => { editorDirty = true; editStatus.textContent = ''; });
  provenance.addEventListener('input', () => { editorDirty = true; editStatus.textContent = ''; });
  save.addEventListener('click', saveEdit);
  addLocale.addEventListener('click', () => {
    const name = newLocale.value.trim();
    if (!/^[a-z]{2,3}(?:-[A-Z]{2})?$/.test(name) || name === 'en') {
      editStatus.textContent = t('workshop.localisation.locale_error');
      newLocale.focus(); return;
    }
    preferredLocale = name; editorDirty = false; refresh();
    newLocale.value = ''; locale.focus();
  });
  return { node, refresh };
}
