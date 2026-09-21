import { refreshTranslationSource, workshopCatalogueReport } from '../editor/workshop-localisation.js';
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
  node.append(label, locale, summary, diagnosticSummary, rows);
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

  function refresh() {
    const value = draft?.();
    const deps = dependencies?.();
    if (!value?.paths?.().includes(STRING_CATALOGUE_PATH) || !deps) {
      locale.replaceChildren();
      rows.replaceChildren();
      diagnosticSummary.replaceChildren();
      summary.textContent = t('workshop.localisation.unavailable');
      return;
    }
    const discoveryReport = workshopCatalogueReport(deps, value, 'en');
    const discovered = discoveryReport.locales.filter((item) => item !== 'en');
    showDiagnostics(discoveryReport);
    const selected = discovered.includes(locale.value) ? locale.value : discovered[0] || '';
    locale.replaceChildren(...discovered.map((name) => {
      const option = doc.createElement('option'); option.value = name; option.textContent = name; return option;
    }));
    locale.value = selected;
    locale.disabled = !selected;
    if (!selected) {
      rows.replaceChildren();
      summary.textContent = t('workshop.localisation.no_locales');
      return;
    }
    const report = workshopCatalogueReport(deps, value, selected);
    summary.textContent = t('workshop.localisation.summary', {
      locale: selected, count: String(report.authored.length),
    });
    showDiagnostics(report);
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
  locale.addEventListener('change', refresh);
  return { node, refresh };
}
