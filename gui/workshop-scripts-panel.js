import { mountScriptEditor, renderScriptList } from '../editor/script-editor-view.js';
import { applyWorkshopScript, workshopScriptUnits, workshopScriptWorlds } from '../editor/workshop-scripts.js';
import { t } from './strings.js';

export function mountWorkshopScripts({ root, provider, runtime, draft, busy, setBusy, changed, attach = true }) {
  const doc = root.ownerDocument;
  const node = (tag, id, attrs = {}) => {
    const value = doc.createElement(tag); if (id) value.textContent = t(id);
    for (const [key, item] of Object.entries(attrs)) value.setAttribute(key, item);
    return value;
  };
  const section = node('section', null, { id: 'workshop-scripts', class: 'workshop-scripts' });
  const world = node('select', null, { id: 'workshop-script-world' });
  const list = node('div', null, { id: 'workshop-script-list', role: 'list', 'aria-label': t('workshop.scripts.units') });
  const editor = node('div', null, { id: 'workshop-script-editor' });
  const status = node('p', null, { id: 'workshop-script-status', role: 'status', tabindex: '-1' });
  section.append(node('p', 'workshop.scripts.scope'), node('label', 'workshop.scripts.world', { for: world.id }),
    world, list, status, editor);
  if (attach) root.append(section);
  let disposed = false, controller = null, hostFns = null, active = null;
  let previousDraft = null, previousRevision = -1, previousWorld = null;
  const option = (value, label) => { const item = node('option', null, { value }); item.textContent = label; return item; };
  const show = (id, error = false) => {
    status.textContent = t(id); status.setAttribute('role', error ? 'alert' : 'status'); if (error) status.focus();
  };
  async function open(unit) {
    if (!unit || busy()) return;
    active = unit;
    if (hostFns == null) {
      try { hostFns = await runtime.scriptHostFunctions(); }
      catch { hostFns = []; }
    }
    if (disposed || busy() || !currentUnit(unit)) return;
    controller?.destroy();
    controller = mountScriptEditor({ host: editor, source: unit.source,
      title: `${unit.documentPath}${unit.kind === 'inline' ? ` — [script.${unit.key}]` : ''}`,
      hostFns, lineOffset: unit.lineOffset,
      getDiagnostics: async (source, lineOffset) => {
        const revision = draft()?.sourceRevision;
        const results = await runtime.scriptDiagnostics(source, lineOffset);
        if (disposed || draft()?.sourceRevision !== revision || active?.id !== unit.id) return [];
        return results.map(row => ({ ...row, file: unit.documentPath, revision }));
      },
      isDiagnosticsAvailable: () => typeof runtime.scriptDiagnostics === 'function',
      onSave: source => void save(unit, source),
    });
    paintList();
  }
  function currentUnit(unit) {
    return workshopScriptUnits(draft(), world.value).some(row => row.id === unit.id
      && row.documentPath === unit.documentPath && row.source === unit.source);
  }
  async function save(unit, source) {
    if (busy() || !currentUnit(unit)) { show('workshop.scripts.stale', true); return; }
    const target = draft(), revision = target.sourceRevision;
    setBusy(true);
    try {
      const edited = await applyWorkshopScript({ draft: target, provider, runtime, unit, source,
        current: () => !disposed && draft() === target && target.sourceRevision === revision });
      if (edited) { changed(unit.documentPath); show('workshop.scripts.applied'); }
      else show('workshop.scripts.unchanged');
    } catch (error) {
      if (!disposed) show(error?.message || 'workshop.scripts.validation_refused', true);
    } finally { if (!disposed) { setBusy(false); refresh(); } }
  }
  function units() { return workshopScriptUnits(draft(), world.value); }
  function paintList() {
    const rows = units();
    renderScriptList(list, rows, { selectedId: active?.id, onSelect: open });
    for (const row of list.querySelectorAll('.script-list-row')) row.setAttribute('role', 'button');
  }
  function refresh({ hidden = false } = {}) {
    section.hidden = hidden;
    const current = draft(), revision = current?.sourceRevision ?? -1;
    if (current !== previousDraft || revision !== previousRevision) {
      const old = world.value || previousWorld;
      const worlds = workshopScriptWorlds(current);
      world.replaceChildren(...worlds.map(row => option(row.path, row.path)));
      if (worlds.some(row => row.path === old)) world.value = old;
      previousDraft = current; previousRevision = revision; previousWorld = world.value;
      if (active && !currentUnit(active)) { active = null; controller?.destroy(); controller = null; }
      paintList();
    }
    world.disabled = hidden || busy() || !world.options.length;
    controller?.textarea && (controller.textarea.disabled = hidden || busy());
  }
  world.addEventListener('change', () => { previousWorld = world.value; active = null; controller?.destroy(); controller = null; paintList(); });
  refresh();
  return { node: section, refresh, dispose() { disposed = true; controller?.destroy(); section.remove(); } };
}
