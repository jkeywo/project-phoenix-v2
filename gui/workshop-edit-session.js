import { definitionsSnapshot, snapshotIsCurrent } from '../editor/workshop-definitions.js';

/** Shared lifetime of a runtime-backed Workshop edit. Panels own the operation,
 * refusal vocabulary and form; the session owns when its answer can land. */
export function createWorkshopEditSession({
  draft, disposed, setBusy, changed, refresh, reload, showChanged, showError,
  restoreFocus = () => {},
}) {
  let operationCurrent = () => true;
  let running = false;
  async function guarded(action, after = () => {}) {
    if (running) return;
    const release = setBusy(true);
    if (release === false) return;
    running = true;
    operationCurrent = typeof release?.current === 'function' ? release.current : () => true;
    let focus = null;
    try { focus = await action(); }
    catch (error) { if (!disposed()) showError(error); }
    finally {
      running = false;
      const released = typeof release === 'function' ? release() : true;
      if (!disposed() && released) {
        if (typeof release !== 'function') setBusy(false);
        refresh();
        // Controls must be enabled before the panel chooses a landing spot.
        if (focus?.length) restoreFocus(...focus);
        after();
      }
    }
  }

  async function land(read, path, result, focus = []) {
    if (disposed() || !operationCurrent()) return null;
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

/** Own busy presentation without inventing freshness or cancellation policy. */
export async function runWorkshopMutation({ setBusy, start = () => {}, invoke, success, error,
  current, successCurrent = current, release }) {
  const end = setBusy(true);
  if (end === false) return;
  start();
  try {
    const result = await invoke();
    if (successCurrent() && (typeof end?.current !== 'function' || end.current())) success(result);
  } catch (reason) {
    if (current()) error(reason);
  } finally {
    const released = typeof end === 'function' ? end() : true;
    if (current() && released) {
      if (typeof end !== 'function') setBusy(false);
      release();
    }
  }
}

/** Workspace admission is shared by all panels. Each acquisition returns its
 * own release capability; a stale completion cannot unlock a later operation. */
export function createWorkshopOperations({ blocked = () => false, changed = () => {} } = {}) {
  let owner = null, disposed = false;
  const operations = {
    busy: () => disposed || owner !== null || blocked(),
    held: () => owner !== null,
    acquire() {
      if (disposed || owner !== null || blocked()) return null;
      const lease = {}; owner = lease; changed();
      const release = () => {
        if (disposed || owner !== lease) return false;
        owner = null; changed(); return true;
      };
      release.current = () => !disposed && owner === lease;
      return release;
    },
    invalidate() { owner = null; changed(); },
    dispose() { disposed = true; owner = null; },
    panel() {
      return {
        busy: () => disposed || owner !== null || blocked(),
        setBusy(value) { if (value) return operations.acquire() || false; },
      };
    },
  };
  return operations;
}
