import { definitionsSnapshot, snapshotIsCurrent } from '../editor/workshop-definitions.js';

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

/** Retain dirty forms by exact source and adapter-owned selection identity. */
export function retainUnappliedForms(forms) {
  return forms.filter(({ state, baseline }) => state && baseline && JSON.stringify(state) !== JSON.stringify(baseline))
    .map(form => ({ ...form, identity: form.identity.slice() }));
}

export function restoreUnappliedForms(forms) {
  for (const form of forms) {
    const current = form.current();
    if (form.source === current.source && form.identity.length === current.identity.length
        && form.identity.every((part, index) => part === current.identity[index])) form.restore(form.state);
  }
}

/** Answer freshness and retention order are shared; typed reads remain local. */
export async function refreshWorkshopReading({ draft, disposed, read, validate, forms, install, announce }) {
  const candidate = draft();
  if (!candidate) return;
  const snapshot = definitionsSnapshot(candidate);
  const result = await read(snapshot.files);
  if (disposed() || !snapshotIsCurrent(snapshot, draft())) return;
  validate(result);
  // Capture after the answer: edits made while reading must survive the repaint.
  const retained = retainUnappliedForms(forms());
  install(snapshot, result);
  restoreUnappliedForms(retained);
  announce?.();
}
