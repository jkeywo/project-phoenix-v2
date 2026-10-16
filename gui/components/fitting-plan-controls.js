import { t, wireText } from '../strings.js';
import { compareFittingPlan, composeFittingPlans, defaultFittingPlan, fittingPlanStorageKey, stationFittingPlan, validateFittingPlan } from '../fitting-plans.js';
import { FITTING_ACTION_IDS } from '../stations/fitting-actions.js';

export function mountFittingPlans(doc, { prelaunch = false, onRehearse = null, onPlanChanged = () => {} } = {}) {
  const node = doc.createElement('details'), heading = doc.createElement('summary'); heading.textContent = t('fitting.plan.title'); node.append(heading);
  node.className = 'fitting-plan-controls'; const el = tag => doc.createElement(tag);
  const label = (control, key) => { const label = el('label'); label.textContent = t('fitting.plan.' + key); label.append(control); return label; };
  const button = (key, action) => { const button = el('button'); button.type = 'button'; button.textContent = t('fitting.plan.' + key); button.dataset.planAction = key; button.addEventListener('click', () => { try { Promise.resolve(action()).catch(error => { status.textContent = t(error.message); }); } catch (error) { status.textContent = t(error.message); } }); return button; };
  const scope = el('select'), name = el('input'), favourites = el('select'), transfer = el('textarea'), comparison = el('div'), status = el('p'); status.setAttribute('role', 'status');
  let context = null, identity = null, plan = null, rows = [], station = null, choices = new Map(), saved = [], lastDraft = null;
  const draftKey = () => fittingPlanStorageKey(context) + '/draft';
  function rememberPlan() {
    onPlanChanged();
    try { lastDraft = JSON.stringify(plan); doc.defaultView.sessionStorage.setItem(draftKey(), lastDraft); } catch { status.textContent = t('fitting.plan.storage-unavailable'); }
  }
  function restoreDraft() {
    try { const raw = doc.defaultView.sessionStorage.getItem(draftKey()); if (!raw || raw === lastDraft) return;
      const stored = JSON.parse(raw); validateFittingPlan(stored, context); if (stored.scope !== '*') return; plan = stored; lastDraft = raw;
    } catch { status.textContent = t('fitting.plan.invalid-choice'); }
  }
  const selected = () => scope.value === '*' ? structuredClone(plan) : stationFittingPlan(plan, context, scope.value);
  function readSaved() {
    saved = [];
    try {
      const stored = doc.defaultView.localStorage.getItem(fittingPlanStorageKey(context)); if (!stored) return;
      const parsed = JSON.parse(stored);
      if (parsed.version !== 1 || !Array.isArray(parsed.favourites)) throw new Error('fitting.plan.unsupported-version');
      for (const favourite of parsed.favourites) {
        if (typeof favourite.name !== 'string' || !favourite.name.trim()) throw new Error('fitting.plan.invalid-choice');
        validateFittingPlan(favourite.plan, context); saved.push(favourite);
      }
    } catch (error) { status.textContent = t(error.message?.startsWith('fitting.') ? error.message : 'fitting.plan.storage-unavailable'); }
  }
  function paintSaved() {
    const previous = favourites.value; favourites.replaceChildren();
    for (const [index, favourite] of saved.entries()) { const option = el('option'); option.value = index; option.textContent = favourite.name; favourites.append(option); }
    if ([...favourites.options].some(option => option.value === previous)) favourites.value = previous;
  }
  function paintComparison() {
    const seen = new Set();
    for (const entry of compareFittingPlan(plan, context, rows)) {
      seen.add(entry.slot); let line = choices.get(entry.slot);
      if (!line) {
        const root = el('div'), title = el('p'), select = el('select'), facts = el('p');
        root.append(title, label(select, 'equipment'), facts); comparison.append(root); line = { root, title, select, facts }; choices.set(entry.slot, line);
        select.addEventListener('change', () => { const slot = context.slots.find(slot => slot.slot === entry.slot); plan.sections[slot.home_station].choices[slot.slot] = select.value;
          if (scope.value !== '*' || slot.home_station === station) plan.sections[slot.home_station].explicit = true; rememberPlan(); paintComparison(); });
        select.addEventListener('keydown', event => { if (!['Tab', 'Escape'].includes(event.key)) event.stopPropagation(); });
      }
      line.title.textContent = `${wireText(entry.home_station)} · ${wireText(entry.slot)}`;
      const ids = entry.compatible_equipment;
      if (line.select.dataset.options !== JSON.stringify(ids)) { line.select.dataset.options = JSON.stringify(ids); line.select.replaceChildren();
        for (const id of ids) { const option = el('option'); option.value = id; option.textContent = wireText(id); line.select.append(option); } }
      if (doc.activeElement !== line.select) line.select.value = entry.choice;
      line.select.disabled = scope.value !== '*' && scope.value !== entry.home_station || (scope.value === '*' && plan.sections[entry.home_station].explicit && entry.home_station !== station);
      line.facts.textContent = t('fitting.plan.comparison', { installed: entry.installed ? wireText(entry.installed) : t('fitting.plan.unobserved'),
        available: entry.available ?? t('fitting.plan.unobserved') });
    }
    for (const [key, line] of choices) if (!seen.has(key)) { line.root.remove(); choices.delete(key); }
    queue.disabled = !station || !compareFittingPlan(plan, context, rows).some(entry => entry.home_station === station && entry.requestable && entry.installed !== entry.choice);
  }
  const save = button('save', () => { if (!name.value.trim()) throw new Error('fitting.plan.name-required');
    const record = { name: name.value.trim(), plan: selected() }; saved = saved.filter(row => row.name !== record.name); saved.push(record);
    doc.defaultView.localStorage.setItem(fittingPlanStorageKey(context), JSON.stringify({ version: 1, favourites: saved })); paintSaved(); status.textContent = t('fitting.plan.saved'); });
  const load = button('load', () => { const record = saved[Number(favourites.value)]; if (!record) return; plan = composeFittingPlans(plan, record.plan, context); rememberPlan(); paintComparison(); status.textContent = t('fitting.plan.loaded'); });
  const exportPlan = button('export', () => { transfer.value = JSON.stringify(selected(), null, 2); status.textContent = t('fitting.plan.exported'); });
  const importPlan = button('import', () => { let imported; try { imported = JSON.parse(transfer.value); } catch { throw new Error('fitting.plan.invalid-choice'); }
    plan = composeFittingPlans(plan, imported, context); rememberPlan(); paintComparison(); status.textContent = t('fitting.plan.loaded'); });
  const queue = button('queue', () => {
    let sent = 0; for (const entry of compareFittingPlan(plan, context, rows)) if (entry.home_station === station && entry.requestable && entry.installed !== entry.choice) {
      const result = doc.defaultView.activateSemanticAction?.(FITTING_ACTION_IDS.request, { context: 'fitting', source: 'control',
        detail: { owner: entry.system, slot: entry.slot, equipment: entry.choice } }); if (result) sent++;
    }
    status.textContent = t('fitting.plan.queued', { count: sent });
  });
  scope.addEventListener('change', () => { onPlanChanged(); paintComparison(); });
  node.append(el('p'), label(scope, 'scope'), label(name, 'name'), save, label(favourites, 'favourites'), load, label(transfer, 'transfer'), exportPlan, importPlan, comparison, queue, status);
  node.querySelector('p').textContent = t(prelaunch ? 'fitting.plan.prelaunch-hint' : 'fitting.plan.hint');
  queue.hidden = prelaunch;
  const rehearse = onRehearse ? button('rehearse', () => onRehearse(selected(), structuredClone(context))) : null;
  if (rehearse) node.append(rehearse);
  const priorDisabled = new Map();
  return { node, update(nextRows = [], nextStation = null) {
    const nextContext = nextRows.find(row => row.fitting_planner)?.fitting_planner; rows = nextRows; station = nextStation;
    if (!nextContext) { node.hidden = true; return; }
    const nextIdentity = JSON.stringify([nextContext.hull_path, nextContext.content_id]);
    if (identity !== nextIdentity) {
      try { plan = defaultFittingPlan(nextContext); } catch { node.hidden = true; return; }
      identity = nextIdentity; context = nextContext; lastDraft = null; readSaved(); paintSaved(); choices.clear(); comparison.replaceChildren();
    } else context = nextContext;
    restoreDraft();
    const scopes = ['*', ...(context.slots.some(slot => slot.home_station === station) ? [station] : [])];
    if (scope.dataset.options !== JSON.stringify(scopes)) { scope.dataset.options = JSON.stringify(scopes); scope.replaceChildren(); for (const id of scopes) { const option = el('option'); option.value = id; option.textContent = id === '*' ? t('fitting.plan.whole-ship') : wireText(id); scope.append(option); } }
    node.hidden = false; paintComparison();
  }, setRehearsalEnabled(enabled) { if (rehearse) rehearse.disabled = !enabled; },
  setRehearsalBusy(busy) {
    if (busy) for (const control of node.querySelectorAll('input,select,textarea,button')) {
      if (!priorDisabled.has(control)) priorDisabled.set(control, control.disabled); control.disabled = true;
    } else { for (const [control, disabled] of priorDisabled) control.disabled = disabled; priorDisabled.clear(); }
  },
  dispose() { choices.clear(); node.remove(); } };
}
