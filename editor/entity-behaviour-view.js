/**
 * entity-behaviour-view.js
 *
 * DOM renderer for the [behaviour] component card. Wraps `BehaviourEditor`
 * to surface the inline validation banner from `editor.validate()` for
 * doctrine-based AI (issue #572).
 *
 * The legacy FSM form (initial_state select, state list, transition list)
 * was retired in issue #794 along with the FSM surface in
 * `BehaviourEditor` itself — see that module's header. Doctrine entries are
 * edited through the card's raw-TOML toggle (`entity-component-card-view.js`
 * `card.showRaw`); this view only renders the read-only validation banner.
 *
 * `onEdit` is accepted for interface parity with the other card views but is
 * not invoked here: this view performs no mutation of its own.
 */

import { BehaviourEditor } from './behaviour-editor.js';

export function renderEntityBehaviourView(host, behaviour, { onEdit } = {}) {
  if (!host) return;
  host.innerHTML = '';
  void onEdit;

  const editor = new BehaviourEditor();
  editor.load(behaviour || {});

  const root = document.createElement('div');
  root.className = 'entity-behaviour';
  host.appendChild(root);

  // Validation banner.
  const validation = editor.validate();
  if (!validation.valid) {
    const banner = document.createElement('div');
    banner.className = 'entity-behaviour-error';
    banner.textContent = validation.errors.join(' • ');
    root.appendChild(banner);
  }

  const hint = document.createElement('div');
  hint.className = 'entity-behaviour-hint';
  hint.textContent = 'Edit doctrine entries via the raw TOML toggle above.';
  root.appendChild(hint);
}
