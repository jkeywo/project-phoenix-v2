import './ph-stock-target.js';
import { t, wireText } from '../strings.js';

/** One Console-scoped stock view. Explicit foreign feeds never add controls. */
export function mountLocalStockControls(doc) {
  const root = doc.createElement('details'); root.className = 'local-stock-controls'; root.hidden = true;
  const heading = doc.createElement('summary'); heading.textContent = t('logistics.local-stores'); root.append(heading);
  const style = doc.createElement('style'); style.textContent = `.local-stock-controls{position:fixed;bottom:.3rem;left:.3rem;max-width:90%;max-height:45vh;overflow:auto;z-index:20;background:var(--surface-panel);color:var(--ink);padding:.4rem;border:1px solid var(--edge-faint)}.local-stock-controls summary{min-height:var(--control-hit-min);cursor:pointer}.local-stock-controls fieldset{min-width:0;max-width:30rem}.local-stock-controls p{overflow-wrap:anywhere}`; root.append(style);
  const rows = new Map(); doc.body.append(root);
  return { update(views = []) {
    const seen = new Set();
    for (const view of views) {
      const key = JSON.stringify([view.system_id, view.store]); seen.add(key);
      let row = rows.get(key);
      if (!row) {
        const node = doc.createElement('fieldset'), title = doc.createElement('legend'), summary = doc.createElement('p'), goal = doc.createElement('ph-stock-target');
        const delegated = doc.createElement('p'); node.append(title, summary, goal, delegated); root.append(node);
        row = { node, title, summary, goal, delegated }; rows.set(key, row);
      }
      row.title.textContent = `${wireText(view.system_id)}: ${wireText(view.resource)}`;
      row.summary.textContent = t('logistics.stock-summary', { resource: wireText(view.resource), level: view.level, available: view.available,
        held: view.held, ceiling: view.ceiling, incoming: view.incoming, shortage: Math.max(0, view.target - view.level - view.incoming - view.held), compartment: wireText(view.compartment) });
      // Delegated owned systems keep the common read-only summary. A different
      // Console's explicitly shared local stock is information, never authority.
      row.goal.state = view.controllable && !view.auto ? { ...view, generic: true } : null;
      row.delegated.textContent = t('logistics.local-goal-summary', { target: view.target });
      row.delegated.hidden = view.controllable && !view.auto;
    }
    for (const [key, row] of rows) if (!seen.has(key)) { row.node.remove(); rows.delete(key); }
    root.hidden = views.length === 0;
  }, dispose() { rows.clear(); root.remove(); } };
}
