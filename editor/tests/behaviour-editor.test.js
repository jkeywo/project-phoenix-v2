import { describe, it, expect } from 'vitest';
import { BehaviourEditor } from '../behaviour-editor.js';

function makeEditor(data) {
  const ed = new BehaviourEditor();
  ed.load(data);
  return ed;
}

const MINIMAL_BEHAVIOUR = {
  initial_state: 'idle',
  state: [{ kind: 'idle', parameters: {} }],
};

const FULL_BEHAVIOUR = {
  initial_state: 'patrol',
  state: [
    { kind: 'patrol', parameters: { anchor: 'alpha' } },
    { kind: 'attack', parameters: { target_entity: 'enemy', range: 200.0 } },
  ],
  transition: [
    { from: ['patrol', 'idle'], to: 'attack', condition: { kind: 'target_in_range', parameters: { range: 200.0 } } },
  ],
};

// ── 1. Load empty ──────────────────────────────────────────────────

describe('load empty / default', () => {
  it('getStates returns empty array after load({})', () => {
    const ed = makeEditor({});
    expect(ed.getStates()).toEqual([]);
  });

  it('getInitialState returns null after load({})', () => {
    const ed = makeEditor({});
    expect(ed.getInitialState()).toBeNull();
  });

  it('getTransitions returns empty array after load({})', () => {
    const ed = makeEditor({});
    expect(ed.getTransitions()).toEqual([]);
  });
});

// ── 2. Load with single state ──────────────────────────────────────

describe('load with single state', () => {
  it('state is present and initial_state is set', () => {
    const ed = makeEditor(MINIMAL_BEHAVIOUR);
    const states = ed.getStates();
    expect(states).toHaveLength(1);
    expect(states[0].name).toBe('idle');
    expect(states[0].kind).toBe('idle');
    expect(ed.getInitialState()).toBe('idle');
  });
});

// ── 3. Add state ───────────────────────────────────────────────────

describe('addState', () => {
  it('appears in getStates after addState', () => {
    const ed = makeEditor({});
    ed.addState('patrol', 'patrol', { anchor: 'beta' });
    const states = ed.getStates();
    expect(states).toHaveLength(1);
    expect(states[0].name).toBe('patrol');
    expect(states[0].kind).toBe('patrol');
    expect(states[0].parameters).toEqual({ anchor: 'beta' });
  });

  it('returns a defensive copy — mutating returned array does not affect internal state', () => {
    const ed = makeEditor({});
    ed.addState('idle', 'idle', {});
    const states = ed.getStates();
    states.push({ name: 'fake' });
    expect(ed.getStates()).toHaveLength(1);
  });
});

// ── 4. Remove state ────────────────────────────────────────────────

describe('removeState', () => {
  it('removes the state from getStates', () => {
    const ed = makeEditor(FULL_BEHAVIOUR);
    ed.removeState('patrol');
    const names = ed.getStates().map((s) => s.name);
    expect(names).not.toContain('patrol');
    expect(names).toContain('attack');
  });

  it('removes the dropped state from all transition.from arrays', () => {
    const ed = makeEditor(FULL_BEHAVIOUR);
    ed.removeState('patrol');
    for (const t of ed.getTransitions()) {
      expect(t.from).not.toContain('patrol');
    }
  });

  it('removes transition whose from array becomes empty after cleanup', () => {
    const ed = makeEditor({
      initial_state: 'a',
      state: [{ kind: 'idle', parameters: {}, name: 'a' }, { kind: 'idle', parameters: {}, name: 'b' }],
      transition: [{ from: ['a'], to: 'b', condition: { kind: 'timer', parameters: { seconds: 5 } } }],
    });
    ed.removeState('a');
    expect(ed.getTransitions()).toHaveLength(0);
  });
});

// ── 5. Set initial state ───────────────────────────────────────────

describe('setInitialState', () => {
  it('getInitialState returns the set value', () => {
    const ed = makeEditor(FULL_BEHAVIOUR);
    ed.setInitialState('attack');
    expect(ed.getInitialState()).toBe('attack');
  });
});

// ── 6. Set initial state to non-existent ───────────────────────────

describe('setInitialState to non-existent', () => {
  it('setInitialState("unknown") — validate returns error', () => {
    const ed = makeEditor(FULL_BEHAVIOUR);
    ed.setInitialState('unknown');
    const result = ed.validate();
    expect(result.valid).toBe(false);
    expect(result.errors.some((e) => e.includes('initial_state'))).toBe(true);
  });
});

// ── 7. Add transition ──────────────────────────────────────────────

describe('addTransition', () => {
  it('appears in getTransitions after addTransition', () => {
    const ed = makeEditor(FULL_BEHAVIOUR);
    ed.addTransition(['patrol'], 'attack', { kind: 'timer', parameters: { seconds: 10 } });
    const ts = ed.getTransitions();
    expect(ts).toHaveLength(2);
    const added = ts[1];
    expect(added.from).toEqual(['patrol']);
    expect(added.to).toBe('attack');
    expect(added.condition.kind).toBe('timer');
  });
});

// ── 8. Remove transition ───────────────────────────────────────────

describe('removeTransition', () => {
  it('removes transition at index', () => {
    const ed = makeEditor(FULL_BEHAVIOUR);
    ed.removeTransition(0);
    expect(ed.getTransitions()).toHaveLength(0);
  });

  it('is a no-op for out-of-range index', () => {
    const ed = makeEditor(FULL_BEHAVIOUR);
    ed.removeTransition(99);
    expect(ed.getTransitions()).toHaveLength(1);
  });
});

// ── 9. Update transition ───────────────────────────────────────────

describe('updateTransition', () => {
  it('updates fields on the transition', () => {
    const ed = makeEditor(FULL_BEHAVIOUR);
    ed.updateTransition(0, { to: 'patrol', condition: { kind: 'timer', parameters: { seconds: 3 } } });
    const t = ed.getTransitions()[0];
    expect(t.to).toBe('patrol');
    expect(t.condition.kind).toBe('timer');
    expect(t.condition.parameters.seconds).toBe(3);
  });

  it('does not affect other transitions', () => {
    const ed = makeEditor(FULL_BEHAVIOUR);
    ed.addTransition(['attack'], 'patrol', { kind: 'timer', parameters: { seconds: 1 } });
    ed.updateTransition(0, { to: 'patrol' });
    expect(ed.getTransitions()[1].to).toBe('patrol');
    expect(ed.getTransitions()[1].condition.kind).toBe('timer');
  });
});

// ── 10. Duplicate state names ──────────────────────────────────────

describe('duplicate state names', () => {
  it('validate returns error for duplicate names', () => {
    const ed = makeEditor({});
    ed.addState('dup', 'idle', {});
    ed.addState('dup', 'idle', {});
    const result = ed.validate();
    expect(result.valid).toBe(false);
    expect(result.errors.some((e) => e.includes('duplicate') || e.includes('Duplicate'))).toBe(true);
  });
});

// ── 11. Transition with invalid from / to ──────────────────────────

describe('transition validation', () => {
  it('transition.from referencing non-existent state returns error', () => {
    const ed = makeEditor(FULL_BEHAVIOUR);
    ed.addTransition(['ghost'], 'attack', { kind: 'timer', parameters: { seconds: 1 } });
    const result = ed.validate();
    expect(result.valid).toBe(false);
    expect(result.errors.some((e) => e.includes('from'))).toBe(true);
  });

  it('transition.to referencing non-existent state returns error', () => {
    const ed = makeEditor(FULL_BEHAVIOUR);
    ed.addTransition(['patrol'], 'ghost', { kind: 'timer', parameters: { seconds: 1 } });
    const result = ed.validate();
    expect(result.valid).toBe(false);
    expect(result.errors.some((e) => e.includes('to'))).toBe(true);
  });
});

// ── 12. toBehaviour serialization ──────────────────────────────────

describe('toBehaviour serialization', () => {
  it('produces correct shape matching TOML schema', () => {
    const ed = makeEditor(FULL_BEHAVIOUR);
    const out = ed.toBehaviour();
    expect(out).toHaveProperty('initial_state', 'patrol');
    expect(out).toHaveProperty('state');
    expect(out).toHaveProperty('transition');
    expect(Array.isArray(out.state)).toBe(true);
    expect(Array.isArray(out.transition)).toBe(true);
    expect(out.state).toHaveLength(2);
    expect(out.transition).toHaveLength(1);
  });

  it('empty behaviour returns empty arrays and no initial_state', () => {
    const ed = makeEditor({});
    const out = ed.toBehaviour();
    // Without an initial_state set, initial_state should not appear or be null
    // The shape should still be valid TOML — arrays present even if empty
    expect(out.state).toEqual([]);
    expect(out.transition).toEqual([]);
  });

  it('serialized state blocks include name field', () => {
    const ed = makeEditor({});
    ed.addState('patrol', 'patrol', { anchor: 'alpha' });
    const out = ed.toBehaviour();
    expect(out.state[0].name).toBe('patrol');
    expect(out.state[0].kind).toBe('patrol');
    expect(out.state[0].parameters).toEqual({ anchor: 'alpha' });
  });
});

// ── 13. Round-trip: create → serialize → load → assert equal ──────

describe('round-trip', () => {
  it('create behaviour → toBehaviour → load → getData matches', () => {
    const ed1 = makeEditor({});
    ed1.addState('idle', 'idle', {});
    ed1.addState('patrol', 'patrol', { anchor: 'alpha' });
    ed1.addState('attack', 'attack', { target_entity: 'enemy', range: 200.0 });
    ed1.setInitialState('patrol');
    ed1.addTransition(['idle', 'patrol'], 'attack', { kind: 'target_in_range', parameters: { range: 150.0 } });
    ed1.addTransition(['attack'], 'patrol', { kind: 'timer', parameters: { seconds: 30 } });

    const serialized = ed1.toBehaviour();
    const ed2 = makeEditor(serialized);

    expect(ed2.getInitialState()).toBe(ed1.getInitialState());
    expect(ed2.getStates()).toEqual(ed1.getStates());
    expect(ed2.getTransitions()).toEqual(ed1.getTransitions());
  });
});

// ── 14. getStateNames ──────────────────────────────────────────────

describe('getStateNames', () => {
  it('returns all state names', () => {
    const ed = makeEditor(FULL_BEHAVIOUR);
    expect(ed.getStateNames()).toEqual(['patrol', 'attack']);
  });

  it('returns empty array when no states', () => {
    const ed = makeEditor({});
    expect(ed.getStateNames()).toEqual([]);
  });
});

// ── 15. Update state kind → parameters reset ───────────────────────

describe('updateState kind changes', () => {
  it('changing kind from patrol to idle resets parameters to empty object', () => {
    const ed = makeEditor({});
    ed.addState('patrol', 'patrol', { anchor: 'alpha' });
    ed.updateState('patrol', { kind: 'idle' });
    const s = ed.getStates().find((st) => st.name === 'patrol');
    expect(s.kind).toBe('idle');
    expect(s.parameters).toEqual({});
  });

  it('changing kind from idle to patrol adds default anchor parameter', () => {
    const ed = makeEditor({});
    ed.addState('idle', 'idle', {});
    ed.updateState('idle', { kind: 'patrol' });
    const s = ed.getStates().find((st) => st.name === 'idle');
    expect(s.kind).toBe('patrol');
    expect(s.parameters).toHaveProperty('anchor');
    expect(typeof s.parameters.anchor).toBe('string');
  });

  it('changing kind to attack adds default target_entity and range', () => {
    const ed = makeEditor({});
    ed.addState('idle', 'idle', {});
    ed.updateState('idle', { kind: 'attack' });
    const s = ed.getStates().find((st) => st.name === 'idle');
    expect(s.kind).toBe('attack');
    expect(s.parameters).toHaveProperty('target_entity');
    expect(s.parameters).toHaveProperty('range');
    expect(typeof s.parameters.range).toBe('number');
  });

  it('changing kind retains existing parameters if they match the new kind', () => {
    const ed = makeEditor({});
    ed.addState('attack', 'attack', { target_entity: 'enemy', range: 200.0 });
    ed.updateState('attack', { kind: 'attack' });
    const s = ed.getStates().find((st) => st.name === 'attack');
    expect(s.parameters).toEqual({ target_entity: 'enemy', range: 200.0 });
  });
});

// ── getData ────────────────────────────────────────────────────────

describe('getData', () => {
  it('returns { initialState, states, transitions }', () => {
    const ed = makeEditor(FULL_BEHAVIOUR);
    const data = ed.getData();
    expect(data).toHaveProperty('initialState');
    expect(data).toHaveProperty('states');
    expect(data).toHaveProperty('transitions');
    expect(Array.isArray(data.states)).toBe(true);
    expect(Array.isArray(data.transitions)).toBe(true);
  });
});

// ── validate — no states ───────────────────────────────────────────

describe('validate — missing states', () => {
  it('returns error when there are no states', () => {
    const ed = makeEditor({});
    const result = ed.validate();
    expect(result.valid).toBe(false);
    expect(result.errors.some((e) => e.includes('state'))).toBe(true);
  });
});

// ── load edges ─────────────────────────────────────────────────────

describe('load edge cases', () => {
  it('load(null) does not throw', () => {
    const ed = new BehaviourEditor();
    expect(() => ed.load(null)).not.toThrow();
  });

  it('load(undefined) does not throw', () => {
    const ed = new BehaviourEditor();
    expect(() => ed.load(undefined)).not.toThrow();
  });

  it('load with states but no initial_state leaves getInitialState null', () => {
    const ed = makeEditor({
      state: [{ kind: 'idle', parameters: {}, name: 'idle' }],
    });
    expect(ed.getInitialState()).toBeNull();
    expect(ed.getStates()).toHaveLength(1);
  });
});

// ── removeState clears initial_state ───────────────────────────────

describe('removeState clears initial_state', () => {
  it('removing the state that is initial_state sets initial_state to null', () => {
    const ed = makeEditor(FULL_BEHAVIOUR);
    ed.removeState('patrol');
    expect(ed.getInitialState()).toBeNull();
  });
});
