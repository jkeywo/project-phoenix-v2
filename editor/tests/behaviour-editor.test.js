import { describe, it, expect } from 'vitest';
import { BehaviourEditor } from '../behaviour-editor.js';

/**
 * The legacy FSM surface (initial_state / state[] / transition[]) was
 * retired in issue #794 along with BehaviourEditor's FSM methods — see that
 * module's header. This suite covers what remains: doctrine load/save
 * round-trip and doctrine validation.
 */

function makeEditor(data) {
  const ed = new BehaviourEditor();
  ed.load(data);
  return ed;
}

const DOCTRINE_BEHAVIOUR = {
  doctrine: [
    { id: 'patrol-a', directive_kind: 'Patrol', base_priority: 1, directive_anchors: ['alpha'] },
    { id: 'destroy-a', directive_kind: 'Destroy', base_priority: 2 },
  ],
};

// ── Load empty / default ──────────────────────────────────────────

describe('load empty / default', () => {
  it('getDoctrine returns empty array after load({})', () => {
    const ed = makeEditor({});
    expect(ed.getDoctrine()).toEqual([]);
  });

  it('load(null) does not throw', () => {
    const ed = new BehaviourEditor();
    expect(() => ed.load(null)).not.toThrow();
  });

  it('load(undefined) does not throw', () => {
    const ed = new BehaviourEditor();
    expect(() => ed.load(undefined)).not.toThrow();
  });
});

// ── Load with doctrine ──────────────────────────────────────────────

describe('load with doctrine', () => {
  it('doctrine entries are present after load', () => {
    const ed = makeEditor(DOCTRINE_BEHAVIOUR);
    const doctrine = ed.getDoctrine();
    expect(doctrine).toHaveLength(2);
    expect(doctrine[0].id).toBe('patrol-a');
    expect(doctrine[1].id).toBe('destroy-a');
  });

  it('getDoctrine returns a defensive copy', () => {
    const ed = makeEditor(DOCTRINE_BEHAVIOUR);
    const doctrine = ed.getDoctrine();
    doctrine[0].id = 'mutated';
    expect(ed.getDoctrine()[0].id).toBe('patrol-a');
  });
});

// ── toBehaviour serialization ──────────────────────────────────────

describe('toBehaviour serialization', () => {
  it('produces only a doctrine key, matching the retired-FSM TOML schema', () => {
    const ed = makeEditor(DOCTRINE_BEHAVIOUR);
    const out = ed.toBehaviour();
    expect(out).toHaveProperty('doctrine');
    expect(out).not.toHaveProperty('state');
    expect(out).not.toHaveProperty('transition');
    expect(out).not.toHaveProperty('initial_state');
    expect(out.doctrine).toHaveLength(2);
  });

  it('empty behaviour returns an empty object (no doctrine key)', () => {
    const ed = makeEditor({});
    const out = ed.toBehaviour();
    expect(out).toEqual({});
  });
});

// ── Round-trip ───────────────────────────────────────────────────────

describe('round-trip', () => {
  it('load → toBehaviour → load → getDoctrine matches', () => {
    const ed1 = makeEditor(DOCTRINE_BEHAVIOUR);
    const serialized = ed1.toBehaviour();
    const ed2 = makeEditor(serialized);
    expect(ed2.getDoctrine()).toEqual(ed1.getDoctrine());
  });
});

// ── getData ────────────────────────────────────────────────────────

describe('getData', () => {
  it('returns { doctrine }', () => {
    const ed = makeEditor(DOCTRINE_BEHAVIOUR);
    const data = ed.getData();
    expect(data).toHaveProperty('doctrine');
    expect(Array.isArray(data.doctrine)).toBe(true);
    expect(Object.keys(data)).toEqual(['doctrine']);
  });
});

// ── Doctrine validation ──────────────────────────────────────────────

describe('doctrine validation', () => {
  it('valid doctrine passes', () => {
    const ed = makeEditor(DOCTRINE_BEHAVIOUR);
    const result = ed.validate();
    expect(result.valid).toBe(true);
    expect(result.errors).toEqual([]);
  });

  it('missing id is an error', () => {
    const ed = makeEditor({ doctrine: [{ directive_kind: 'Destroy', base_priority: 1 }] });
    const result = ed.validate();
    expect(result.valid).toBe(false);
    expect(result.errors.some((e) => e.includes('missing id'))).toBe(true);
  });

  it('missing directive_kind is an error', () => {
    const ed = makeEditor({ doctrine: [{ id: 'a', base_priority: 1 }] });
    const result = ed.validate();
    expect(result.valid).toBe(false);
    expect(result.errors.some((e) => e.includes('directive_kind'))).toBe(true);
  });

  it('non-numeric base_priority is an error', () => {
    const ed = makeEditor({ doctrine: [{ id: 'a', directive_kind: 'Destroy', base_priority: 'high' }] });
    const result = ed.validate();
    expect(result.valid).toBe(false);
    expect(result.errors.some((e) => e.includes('base_priority'))).toBe(true);
  });

  it('Patrol directive without directive_anchors is an error', () => {
    const ed = makeEditor({ doctrine: [{ id: 'a', directive_kind: 'Patrol', base_priority: 1 }] });
    const result = ed.validate();
    expect(result.valid).toBe(false);
    expect(result.errors.some((e) => e.includes('directive_anchors'))).toBe(true);
  });

  it('empty doctrine list passes (no doctrine authored)', () => {
    const ed = makeEditor({});
    const result = ed.validate();
    expect(result.valid).toBe(true);
    expect(result.errors).toEqual([]);
  });
});
