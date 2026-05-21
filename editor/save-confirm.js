/**
 * save-confirm.js — Once-per-session comment-warning gate.
 *
 * Wraps `CommentWarning` with a lazy singleton + a thin
 * `confirmSaveIfCommented(rawText, { confirmFn })` helper that prompts
 * the user the FIRST time a TOML file containing `#` is saved in the
 * current project-root session, and stays silent thereafter (unless
 * the user changes project root, which calls `reset()`).
 *
 * The detection heuristic lives on `CommentWarning.shouldWarn`; this
 * module deliberately does NOT change it.
 */

import { CommentWarning } from './comment-warning.js';

let _singleton = null;

/**
 * Lazily build the per-session `CommentWarning` singleton.
 */
export function getCommentWarning() {
  if (!_singleton) _singleton = new CommentWarning();
  return _singleton;
}

/**
 * Returns `true` if the save should proceed, `false` if the user
 * declined. `confirmFn` defaults to `window.confirm`; tests inject a
 * stub. On acceptance, the singleton is acknowledged so subsequent
 * commented saves in the same session don't re-prompt.
 *
 * For uncommented files this is a fast no-op returning `true`.
 */
export function confirmSaveIfCommented(rawText, opts = {}) {
  const cw = getCommentWarning();
  if (!cw.shouldWarn(rawText)) return true;

  const confirmFn = opts.confirmFn
    || (typeof window !== 'undefined' && typeof window.confirm === 'function'
      ? window.confirm.bind(window)
      : null);

  if (typeof confirmFn !== 'function') {
    // No confirm surface available — proceed without prompting, but
    // still mark acknowledged so we don't spin on every save.
    cw.acknowledge();
    return true;
  }

  const accepted = !!confirmFn(
    'This file contains TOML comments (#). The editor writes a normalised '
    + 'TOML stream and will discard them. Save anyway?'
  );
  if (accepted) cw.acknowledge();
  return accepted;
}

/**
 * Wire `reset()` to fire whenever the user picks a new project root.
 * Returns the unsubscribe handle from `onRootChanged` (or `null` if
 * the dependency wasn't supplied).
 */
export function resetCommentWarningOnRootChange(deps = {}) {
  const onRootChanged = deps.onRootChanged;
  if (typeof onRootChanged !== 'function') return null;
  return onRootChanged(() => {
    getCommentWarning().reset();
  });
}

/** Test hook: drop the singleton between cases. */
export function _resetSingletonForTest() {
  _singleton = null;
}
