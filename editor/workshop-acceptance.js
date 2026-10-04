import { WorkshopDocument } from './workshop-document.js';

/** Accept a form's exact-source changes as one chronological edit. Capture
 * before any asynchronous preparation; neither preparation nor validation may
 * borrow the changing live draft. Provider capabilities remain explicit. */
export async function prepareWorkshopChanges({ draft, provider, runtime, dependencies,
  prepare, inspect, current = () => true, validateUnchanged = false, stale = 'stale-workshop-operation', refused = 'runtime-validation-refused' }) {
  const revision = draft.sourceRevision;
  const fresh = () => {
    if (!current() || draft.sourceRevision !== revision) throw new Error(stale);
  };
  fresh();
  const candidate = provider?.restoreDocument
    ? provider.restoreDocument(draft.snapshot()) : WorkshopDocument.restore(draft.snapshot());
  const supplied = dependencies;
  const suppliedSource = supplied && JSON.stringify(supplied);
  // null explicitly marks preparation that does not read dependencies.
  const captured = supplied === null ? null : supplied || await runtime.dependencies();
  fresh();
  const dependencySource = JSON.stringify(captured);
  const effective = structuredClone(captured);
  const unchanged = () => {
    fresh();
    if (JSON.stringify(captured) !== dependencySource
      || (supplied && JSON.stringify(supplied) !== suppliedSource)) throw new Error(stale);
  };
  const changes = await prepare(candidate, effective);
  unchanged();
  if (!changes.length && !validateUnchanged) return { candidate, changes, report: null, commit() { unchanged(); return { changes, applied: false, report: null }; } };
  candidate.apply(changes);
  const inspection = inspect?.(candidate, effective);
  const report = await runtime.validate(candidate.kind === 'mod' && !provider?.save ? candidate.archive() : null, candidate);
  unchanged();
  if (!report?.accepted) {
    const error = new Error(refused); error.report = report; throw error;
  }
  return { candidate, changes, report, inspection, commit() {
    unchanged();
    const applied = draft.apply(changes);
    return { changes, applied, report, inspection };
  } };
}

export async function acceptWorkshopChanges(options) {
  return (await prepareWorkshopChanges(options)).commit();
}
