/**
 * behaviour-editor.js
 *
 * Pure-logic module for editing the [behaviour] block of an entity TOML.
 *
 * Two formats are supported:
 *   - Doctrine-based AI (issue #572): `[[behaviour.doctrine]]` entries.
 *     No FSM; behaviour driven by standing doctrine objectives scored each tick.
 *   - Legacy FSM: `initial_state`, `state[]`, `transition[]`.
 *
 * No DOM manipulation is performed here; the class is fully testable in Node.
 */

function defaultParametersForKind(kind) {
  switch (kind) {
    case 'idle': return {};
    case 'patrol': return { anchor: '' };
    case 'attack': return { target_entity: '', range: 0 };
    default: return {};
  }
}

function cloneState(s) {
  return { name: s.name, kind: s.kind, parameters: { ...s.parameters } };
}

function cloneTransition(t) {
  return {
    from: [...t.from],
    to: t.to,
    condition: {
      kind: t.condition.kind,
      parameters: { ...(t.condition.parameters || {}) },
    },
  };
}

export class BehaviourEditor {
  constructor() {
    this._initialState = null;
    this._states = [];
    this._transitions = [];
    this._doctrine = [];
  }

  load(behaviour = {}) {
    const b = behaviour || {};
    this._initialState = b.initial_state ?? null;
    this._states = [];
    this._transitions = [];
    this._doctrine = [];

    if (Array.isArray(b.doctrine)) {
      this._doctrine = b.doctrine.map((d) => ({ ...d }));
    }

    if (Array.isArray(b.state)) {
      for (const s of b.state) {
        this._states.push(cloneState({
          name: s.name ?? s.kind,
          kind: s.kind,
          parameters: s.parameters || {},
        }));
      }
    }

    if (Array.isArray(b.transition)) {
      for (const t of b.transition) {
        const fromRaw = t.from;
        const fromArr = Array.isArray(fromRaw)
          ? fromRaw
          : (fromRaw == null ? [] : [fromRaw]);
        this._transitions.push(cloneTransition({
          from: fromArr,
          to: t.to,
          condition: t.condition || { kind: null, parameters: {} },
        }));
      }
    }
  }

  getData() {
    return {
      initialState: this._initialState,
      states: this._states.map(cloneState),
      transitions: this._transitions.map(cloneTransition),
      doctrine: this._doctrine.map((d) => ({ ...d })),
    };
  }

  getInitialState() {
    return this._initialState;
  }

  setInitialState(name) {
    this._initialState = name;
  }

  getStates() {
    return this._states.map(cloneState);
  }

  addState(name, kind, parameters) {
    const duplicate = this._states.some((s) => s.name === name);
    const params = parameters !== undefined ? parameters : defaultParametersForKind(kind);
    this._states.push({ name, kind, parameters: { ...params } });
    return { ok: true, warning: duplicate ? `State "${name}" already exists` : undefined };
  }

  removeState(name) {
    this._states = this._states.filter((s) => s.name !== name);

    this._transitions = this._transitions.filter((t) => t.to !== name);
    for (const t of this._transitions) {
      t.from = t.from.filter((f) => f !== name);
    }
    this._transitions = this._transitions.filter((t) => t.from.length > 0);

    if (this._initialState === name) {
      this._initialState = null;
    }
  }

  updateState(name, changes) {
    const state = this._states.find((s) => s.name === name);
    if (!state) return;

    if (changes.kind !== undefined && changes.kind !== state.kind) {
      state.kind = changes.kind;
      state.parameters = { ...defaultParametersForKind(changes.kind) };
    }

    if (changes.parameters !== undefined) {
      state.parameters = { ...changes.parameters };
    }
  }

  getTransitions() {
    return this._transitions.map(cloneTransition);
  }

  addTransition(from, to, condition) {
    this._transitions.push({
      from: [...from],
      to,
      condition: {
        kind: condition.kind,
        parameters: { ...(condition.parameters || {}) },
      },
    });
  }

  removeTransition(index) {
    if (index >= 0 && index < this._transitions.length) {
      this._transitions.splice(index, 1);
    }
  }

  updateTransition(index, changes) {
    if (index < 0 || index >= this._transitions.length) return;
    const t = this._transitions[index];
    if (changes.from !== undefined) t.from = [...changes.from];
    if (changes.to !== undefined) t.to = changes.to;
    if (changes.condition !== undefined) {
      t.condition = {
        kind: changes.condition.kind,
        parameters: { ...(changes.condition.parameters || {}) },
      };
    }
  }

  getDoctrine() {
    return this._doctrine.map((d) => ({ ...d }));
  }

  toBehaviour() {
    const out = {};
    if (this._initialState) {
      out.initial_state = this._initialState;
    }
    out.state = this._states.map(cloneState);
    out.transition = this._transitions.map(cloneTransition);
    if (this._doctrine.length > 0) {
      out.doctrine = this._doctrine.map((d) => ({ ...d }));
    }
    return out;
  }

  getStateNames() {
    return this._states.map((s) => s.name);
  }

  validate() {
    const errors = [];

    // Doctrine-based validation (issue #572).
    if (this._doctrine.length > 0) {
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

    // Legacy FSM validation.
    if (this._states.length === 0) {
      errors.push('Must have at least one state');
    }

    if (this._states.length > 0 && !this._initialState) {
      errors.push('initial_state is required when states are present');
    }

    const names = this._states.map((s) => s.name);
    const seen = new Set();
    for (const name of names) {
      if (seen.has(name)) {
        errors.push(`Duplicate state name: "${name}"`);
      }
      seen.add(name);
    }

    if (this._initialState !== null && !names.includes(this._initialState)) {
      errors.push(`initial_state "${this._initialState}" does not match any state name`);
    }

    for (let i = 0; i < this._transitions.length; i++) {
      const t = this._transitions[i];
      for (const f of t.from) {
        if (!names.includes(f)) {
          errors.push(`Transition ${i}: from "${f}" is not a valid state name`);
        }
      }
      if (!names.includes(t.to)) {
        errors.push(`Transition ${i}: to "${t.to}" is not a valid state name`);
      }
    }

    return { valid: errors.length === 0, errors };
  }
}

export { defaultParametersForKind };
