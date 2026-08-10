/**
 * script-editor-view.js
 *
 * DOM view for the Scenario Mode script editor (issue #983, Rhai M5).
 *
 * Renders a `.rhai` text editor: a `<textarea>` for native editing over a
 * `<pre>` syntax-highlight layer, a host-fn autocomplete dropdown, and a
 * diagnostics list fed by the WASM-provided compile pass. All the pure logic
 * (tokenizing, completion matching, unit extraction) lives in
 * `script-editor.js`; this file owns the DOM.
 *
 * Nothing here loads an external resource — the highlighter is the inline
 * tokenizer, satisfying the editor's no-CDN / CSP constraint.
 */

import {
  tokenizeRhai,
  completionContext,
  matchCompletions,
} from './script-editor.js';

function escapeHtml(s) {
  return String(s ?? '')
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;');
}

/**
 * Render the list of a world's script units into `host`.
 *
 * @param {HTMLElement|null} host
 * @param {Array<{id,label,kind}>} units
 * @param {object} opts
 * @param {string|null} [opts.selectedId]
 * @param {(unit: object) => void} [opts.onSelect]
 */
export function renderScriptList(host, units, { selectedId = null, onSelect } = {}) {
  if (!host) return;
  host.innerHTML = '';
  if (!units || units.length === 0) {
    const p = document.createElement('p');
    p.className = 'placeholder';
    p.textContent = 'No scripts in this world';
    host.appendChild(p);
    return;
  }
  for (const unit of units) {
    const row = document.createElement('div');
    row.className = 'script-list-row clickable';
    if (unit.id === selectedId) row.classList.add('active');
    row.dataset.scriptId = unit.id;
    row.innerHTML =
      `<span class="script-list-icon">${unit.kind === 'sibling' ? '📄' : '❴❵'}</span>` +
      `<span class="script-list-label">${escapeHtml(unit.label)}</span>`;
    if (typeof onSelect === 'function') {
      row.addEventListener('click', () => onSelect(unit));
    }
    host.appendChild(row);
  }
}

/**
 * Mount a script editor into `host`.
 *
 * @param {object} opts
 * @param {HTMLElement} opts.host
 * @param {string} [opts.source]           Initial source text.
 * @param {string} [opts.title]            Header label (e.g. the unit label).
 * @param {Array<object>} [opts.hostFns]   Host-fn registry for autocomplete.
 * @param {(source: string, lineOffset: number) => (Array|Promise<Array>)}
 *        [opts.getDiagnostics]  Compile pass; returns `{message,line,column,severity}[]`.
 * @param {() => boolean} [opts.isDiagnosticsAvailable]
 *        Reports whether the live compile pass is actually wired. When it returns
 *        `false`, an empty diagnostics result is shown as "live checks
 *        unavailable" (see issue #995), NOT as "No problems" — so a dead WASM
 *        seam never masquerades as a clean compile. Omit it (or return `true`) to
 *        treat empty as genuinely clean.
 * @param {number} [opts.lineOffset]       Added to every diagnostic line — the
 *                                         inline-block span mapping (0 for a
 *                                         standalone buffer / sibling file).
 * @param {(source: string) => void} [opts.onChange]
 * @param {(source: string) => void} [opts.onSave]
 * @param {number} [opts.diagnosticsDelayMs]  Debounce (0 in tests for sync).
 * @returns {object} controller
 */
export function mountScriptEditor({
  host,
  source = '',
  title = 'Script',
  hostFns = [],
  getDiagnostics,
  isDiagnosticsAvailable,
  lineOffset = 0,
  onChange,
  onSave,
  diagnosticsDelayMs = 250,
} = {}) {
  if (!host) throw new Error('mountScriptEditor: host is required');

  const knownFns = new Set((hostFns || []).map((h) => h.name));

  host.innerHTML = '';
  const root = document.createElement('div');
  root.className = 'script-editor';

  // Header.
  const header = document.createElement('div');
  header.className = 'script-editor-header';
  const titleEl = document.createElement('span');
  titleEl.className = 'script-editor-title';
  titleEl.textContent = title;
  const saveBtn = document.createElement('button');
  saveBtn.type = 'button';
  saveBtn.className = 'script-editor-save';
  saveBtn.textContent = 'Save Script';
  header.appendChild(titleEl);
  header.appendChild(saveBtn);
  root.appendChild(header);

  // Body: highlight layer under a textarea, with the autocomplete popup.
  const body = document.createElement('div');
  body.className = 'script-editor-body';
  const pre = document.createElement('pre');
  pre.className = 'script-highlight';
  pre.setAttribute('aria-hidden', 'true');
  const code = document.createElement('code');
  pre.appendChild(code);
  const textarea = document.createElement('textarea');
  textarea.className = 'script-editor-input';
  textarea.spellcheck = false;
  textarea.setAttribute('autocomplete', 'off');
  textarea.setAttribute('autocapitalize', 'off');
  textarea.value = source;
  const popup = document.createElement('ul');
  popup.className = 'script-autocomplete hidden';
  body.appendChild(pre);
  body.appendChild(textarea);
  body.appendChild(popup);
  root.appendChild(body);

  // Diagnostics.
  const diagEl = document.createElement('div');
  diagEl.className = 'script-diagnostics';
  root.appendChild(diagEl);

  host.appendChild(root);

  let currentCompletions = [];
  let activeIndex = -1;
  let diagTimer = null;

  function refreshHighlight() {
    const tokens = tokenizeRhai(textarea.value, knownFns);
    code.innerHTML = tokens
      .map((t) => (t.type === 'ws'
        ? escapeHtml(t.value)
        : `<span class="tok-${t.type}">${escapeHtml(t.value)}</span>`))
      .join('');
  }

  function syncScroll() {
    pre.scrollTop = textarea.scrollTop;
    pre.scrollLeft = textarea.scrollLeft;
  }

  function closeCompletions() {
    currentCompletions = [];
    activeIndex = -1;
    popup.classList.add('hidden');
    popup.innerHTML = '';
  }

  function renderCompletions() {
    popup.innerHTML = '';
    if (currentCompletions.length === 0) {
      popup.classList.add('hidden');
      return;
    }
    currentCompletions.forEach((item, i) => {
      const li = document.createElement('li');
      li.className = 'script-autocomplete-item';
      if (i === activeIndex) li.classList.add('active');
      li.innerHTML =
        `<span class="ac-sig">${escapeHtml(item.signature || item.name)}</span>` +
        (item.summary ? `<span class="ac-doc">${escapeHtml(item.summary)}</span>` : '');
      li.addEventListener('mousedown', (e) => {
        // mousedown (not click) so the textarea does not blur first.
        e.preventDefault();
        applyCompletion(item);
      });
      popup.appendChild(li);
    });
    popup.classList.remove('hidden');
  }

  function openCompletions() {
    const caret = textarea.selectionStart ?? textarea.value.length;
    const before = textarea.value.slice(0, caret);
    const ctx = completionContext(before);
    // Only suggest once there's something to filter on, to avoid a popup on
    // every keystroke in open code. A member context (`ctx.`, `effects.`)
    // suggests immediately.
    if (ctx.receiver === '' && ctx.prefix.length < 1) {
      closeCompletions();
      return;
    }
    currentCompletions = matchCompletions(hostFns, ctx);
    activeIndex = currentCompletions.length > 0 ? 0 : -1;
    renderCompletions();
  }

  function applyCompletion(item) {
    const caret = textarea.selectionStart ?? textarea.value.length;
    const before = textarea.value.slice(0, caret);
    const after = textarea.value.slice(caret);
    const prefixMatch = before.match(/([A-Za-z_][A-Za-z0-9_]*)$/);
    const start = prefixMatch ? before.length - prefixMatch[1].length : before.length;
    const isNamespace = item.category === 'namespace';
    const insert = isNamespace ? `${item.name}.` : `${item.name}(`;
    const next = before.slice(0, start) + insert + after;
    textarea.value = next;
    const pos = start + insert.length;
    textarea.selectionStart = pos;
    textarea.selectionEnd = pos;
    closeCompletions();
    refreshHighlight();
    emitChange();
    scheduleDiagnostics();
  }

  function emitChange() {
    if (typeof onChange === 'function') onChange(textarea.value);
  }

  function renderDiagnostics(diags) {
    diagEl.innerHTML = '';
    if (!diags || diags.length === 0) {
      diagEl.classList.remove('has-errors');
      // An empty result only means "clean" when the live check actually ran.
      // If the WASM seam is dead (see issue #995) getDiagnostics degrades to []
      // — say so plainly instead of claiming the script has no problems.
      if (typeof isDiagnosticsAvailable === 'function' && !isDiagnosticsAvailable()) {
        diagEl.classList.add('unavailable');
        const hint = document.createElement('span');
        hint.className = 'script-diagnostics-unavailable';
        hint.textContent =
          'Live checks unavailable — script validation is not wired in the editor yet (#995).';
        diagEl.appendChild(hint);
        return;
      }
      diagEl.classList.remove('unavailable');
      const ok = document.createElement('span');
      ok.className = 'script-diagnostics-ok';
      ok.textContent = 'No problems';
      diagEl.appendChild(ok);
      return;
    }
    diagEl.classList.remove('unavailable');
    diagEl.classList.add('has-errors');
    for (const d of diags) {
      const row = document.createElement('div');
      row.className = `script-diagnostic sev-${d.severity || 'error'}`;
      const loc = document.createElement('span');
      loc.className = 'script-diagnostic-loc';
      loc.textContent = `Line ${d.line}${d.column ? ':' + d.column : ''}`;
      const msg = document.createElement('span');
      msg.className = 'script-diagnostic-msg';
      msg.textContent = d.message;
      row.appendChild(loc);
      row.appendChild(msg);
      diagEl.appendChild(row);
    }
  }

  async function runDiagnostics() {
    if (typeof getDiagnostics !== 'function') {
      renderDiagnostics([]);
      return [];
    }
    let diags = [];
    try {
      diags = await getDiagnostics(textarea.value, lineOffset);
    } catch (err) {
      console.warn('[script-editor] diagnostics failed:', err?.message || err);
      diags = [];
    }
    renderDiagnostics(diags || []);
    return diags || [];
  }

  function scheduleDiagnostics() {
    if (diagnosticsDelayMs <= 0) { runDiagnostics(); return; }
    if (diagTimer) clearTimeout(diagTimer);
    diagTimer = setTimeout(() => { diagTimer = null; runDiagnostics(); }, diagnosticsDelayMs);
  }

  function onInput() {
    refreshHighlight();
    emitChange();
    openCompletions();
    scheduleDiagnostics();
  }

  function onKeydown(e) {
    if (!popup.classList.contains('hidden') && currentCompletions.length > 0) {
      if (e.key === 'ArrowDown') {
        e.preventDefault();
        activeIndex = (activeIndex + 1) % currentCompletions.length;
        renderCompletions();
        return;
      }
      if (e.key === 'ArrowUp') {
        e.preventDefault();
        activeIndex = (activeIndex - 1 + currentCompletions.length) % currentCompletions.length;
        renderCompletions();
        return;
      }
      if (e.key === 'Enter' || e.key === 'Tab') {
        e.preventDefault();
        applyCompletion(currentCompletions[Math.max(0, activeIndex)]);
        return;
      }
      if (e.key === 'Escape') {
        e.preventDefault();
        closeCompletions();
        return;
      }
    }
    // Ctrl/Cmd+S saves.
    if ((e.ctrlKey || e.metaKey) && (e.key === 's' || e.key === 'S')) {
      e.preventDefault();
      if (typeof onSave === 'function') onSave(textarea.value);
    }
  }

  textarea.addEventListener('input', onInput);
  textarea.addEventListener('keydown', onKeydown);
  textarea.addEventListener('scroll', syncScroll);
  textarea.addEventListener('blur', () => closeCompletions());
  saveBtn.addEventListener('click', () => {
    if (typeof onSave === 'function') onSave(textarea.value);
  });

  // Initial paint.
  refreshHighlight();
  runDiagnostics();

  return {
    el: root,
    textarea,
    getSource: () => textarea.value,
    setSource(next) {
      textarea.value = next ?? '';
      refreshHighlight();
      runDiagnostics();
    },
    get completions() { return currentCompletions; },
    openCompletions,
    applyCompletion,
    closeCompletions,
    refreshHighlight,
    runDiagnostics,
    destroy() {
      if (diagTimer) clearTimeout(diagTimer);
      textarea.removeEventListener('input', onInput);
      textarea.removeEventListener('keydown', onKeydown);
      textarea.removeEventListener('scroll', syncScroll);
      host.innerHTML = '';
    },
  };
}
