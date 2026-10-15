import { mountFittingPlans } from './components/fitting-plan-controls.js';
const mounted = new WeakMap();
/** Static accepted hull choices only. Lobby has no live machinery authority. */
export function renderLobbyFittingPlanner(doc, view) {
  const root = doc.getElementById('lobby-fitting-planner'); if (!root) return;
  let widget = mounted.get(root);
  if (!view?.context) { root.hidden = true; return; }
  if (!widget) { widget = mountFittingPlans(doc, { prelaunch: true }); root.append(widget.node); mounted.set(root, widget); }
  widget.update([{ fitting_planner: view.context }], view.station); root.hidden = widget.node.hidden;
}
