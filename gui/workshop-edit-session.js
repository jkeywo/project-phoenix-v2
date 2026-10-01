import { snapshotIsCurrent } from '../editor/workshop-definitions.js';

/** Shared lifetime of a runtime-backed Workshop edit. Panels own the operation,
 * refusal vocabulary and form; the session owns when its answer can land. */
export function createWorkshopEditSession({
  draft, disposed, setBusy, changed, refresh, reload, showChanged, showError,
  restoreFocus = () => {},
}) {
  async function guarded(action, after = () => {}) {
    setBusy(true);
    let focus = null;
    try { focus = await action(); }
    catch (error) { if (!disposed()) showError(error); }
    finally {
      if (!disposed()) {
        setBusy(false);
        refresh();
        // Controls must be enabled before the panel chooses a landing spot.
        if (focus?.length) restoreFocus(...focus);
        after();
      }
    }
  }

  async function land(read, path, result, focus = []) {
    if (disposed()) return null;
    if (!snapshotIsCurrent(read, draft())) throw new Error('workshop.inspector_stale');
    if (typeof result !== 'string') throw new Error('workshop.inspector_refused');
    if (draft().edit(path, result)) changed(path);
    showChanged();
    // A refresh failure cannot turn an already committed edit into a refusal.
    try { await reload({ announce: false }); } catch (_) { /* reading remains stale */ }
    return focus;
  }

  return { guarded, land };
}
