/**
 * behaviour-editor.js
 *
 * Pure-logic module for editing the [behaviour] block of an entity TOML.
 *
 * Doctrine-based AI (issue #572) is the only supported format:
 * `[[behaviour.doctrine]]` entries — no FSM; behaviour driven by standing
 * doctrine objectives scored each tick.
 *
 * The legacy FSM surface (`initial_state` / `state[]` / `transition[]`,
 * pre-#572) was retired in issue #794: `BehaviourConfig` is
 * `#[serde(deny_unknown_fields)]` and carries no such fields on the Rust
 * side (see `src/entities/entity_override.rs`'s note on the retired FSM),
 * nothing in `assets/` authors it, and round-tripping it through this editor
 * would produce a TOML entity load can no longer parse. This module no
 * longer reads or writes those keys at all.
 *
 * No DOM manipulation is performed here; the class is fully testable in Node.
 */

export class BehaviourEditor {
  constructor() {
    this._doctrine = [];
  }

  load(behaviour = {}) {
    const b = behaviour || {};
    this._doctrine = [];

    if (Array.isArray(b.doctrine)) {
      this._doctrine = b.doctrine.map((d) => ({ ...d }));
    }
  }

  getData() {
    return {
      doctrine: this._doctrine.map((d) => ({ ...d })),
    };
  }

  getDoctrine() {
    return this._doctrine.map((d) => ({ ...d }));
  }

  toBehaviour() {
    const out = {};
    if (this._doctrine.length > 0) {
      out.doctrine = this._doctrine.map((d) => ({ ...d }));
    }
    return out;
  }

  validate() {
    const errors = [];

    // Doctrine-based validation (issue #572).
    for (let i = 0; i < this._doctrine.length; i++) {
      const d = this._doctrine[i];
      if (!d.id) {
        errors.push(`Doctrine [${i}]: missing id`);
      }
      if (!d.directive_kind) {
        errors.push(`Doctrine [${i}]: missing directive_kind`);
      }
      if (d.base_priority == null || typeof d.base_priority !== 'number') {
        errors.push(`Doctrine [${i}]: base_priority must be a number`);
      }
      if ((d.directive_kind === 'Patrol' || d.directive_kind === 'patrol') && (!Array.isArray(d.directive_anchors) || d.directive_anchors.length === 0)) {
        errors.push(`Doctrine [${i}]: Patrol directive needs directive_anchors`);
      }
    }

    return { valid: errors.length === 0, errors };
  }
}
