import { describe, it, expect } from 'vitest';
import { CommsEditor } from '../comms-editor.js';

function makeEditor(comms) {
  const ed = new CommsEditor();
  ed.load(comms);
  return ed;
}

const SAMPLE_TEMPLATE = {
  from: 'starbase_alpha',
  trigger: { kind: 'on_hailed', entity: 'starbase_alpha' },
  node: {
    body: 'Welcome to Starbase Alpha. State your business.',
    responses: [
      {
        text: 'We come in peace.',
        actions: [
          { type: 'add_objective', id: 'dock', text: 'Dock at Starbase Alpha', mandatory: true },
        ],
      },
      {
        text: 'We have an emergency!',
        actions: [
          { type: 'add_objective', id: 'repair', text: 'Request repairs', mandatory: false },
        ],
        follow_up: {
          body: "We'll send a repair team. Stand by.",
          responses: [],
        },
      },
    ],
  },
};

// ── 1. Load empty ──────────────────────────────────────────────────

describe('load empty', () => {
  it('getTemplates returns empty array after load([])', () => {
    const ed = makeEditor([]);
    expect(ed.getTemplates()).toEqual([]);
  });

  it('getTemplates returns empty array after load(null)', () => {
    const ed = makeEditor(null);
    expect(ed.getTemplates()).toEqual([]);
  });

  it('getTemplates returns empty array after load(undefined)', () => {
    const ed = new CommsEditor();
    ed.load(undefined);
    expect(ed.getTemplates()).toEqual([]);
  });
});

// ── 2. Add template ────────────────────────────────────────────────

describe('addTemplate', () => {
  it('appears in getTemplates after addTemplate', () => {
    const ed = makeEditor([]);
    ed.addTemplate('starbase_alpha', 'on_hailed', 'starbase_alpha');
    const templates = ed.getTemplates();
    expect(templates).toHaveLength(1);
    expect(templates[0].from).toBe('starbase_alpha');
    expect(templates[0].trigger.kind).toBe('on_hailed');
    expect(templates[0].trigger.entity).toBe('starbase_alpha');
    expect(templates[0].node.body).toBe('');
    expect(templates[0].node.responses).toEqual([]);
  });

  it('returns defensive copy — mutating returned array does not affect internal state', () => {
    const ed = makeEditor([]);
    ed.addTemplate('starbase_alpha', 'on_hailed', 'starbase_alpha');
    const templates = ed.getTemplates();
    templates.push({ from: 'fake' });
    expect(ed.getTemplates()).toHaveLength(1);
  });

  it('addTemplate with on_timer trigger', () => {
    const ed = makeEditor([]);
    ed.addTemplate('station', 'on_timer', 'ignored');
    const templates = ed.getTemplates();
    expect(templates[0].trigger.kind).toBe('on_timer');
    expect(templates[0].trigger.after_secs).toBe(10.0);
    expect(templates[0].trigger.entity).toBeUndefined();
  });
});

// ── 3. Set template from field ─────────────────────────────────────

describe('setTemplateField', () => {
  it('sets "from" field', () => {
    const ed = makeEditor([SAMPLE_TEMPLATE]);
    ed.setTemplateField(0, 'from', 'earth_spacedock');
    expect(ed.getTemplates()[0].from).toBe('earth_spacedock');
  });

  it('sets "trigger.kind" field', () => {
    const ed = makeEditor([SAMPLE_TEMPLATE]);
    ed.setTemplateField(0, 'trigger.kind', 'on_destroyed');
    expect(ed.getTemplates()[0].trigger.kind).toBe('on_destroyed');
  });

  it('sets "trigger.entity" field', () => {
    const ed = makeEditor([SAMPLE_TEMPLATE]);
    ed.setTemplateField(0, 'trigger.entity', 'earth_spacedock');
    expect(ed.getTemplates()[0].trigger.entity).toBe('earth_spacedock');
  });

  it('is a no-op for out-of-range index', () => {
    const ed = makeEditor([SAMPLE_TEMPLATE]);
    ed.setTemplateField(99, 'from', 'earth_spacedock');
    expect(ed.getTemplates()[0].from).toBe('starbase_alpha');
  });
});

// ── 4. Set root node body ──────────────────────────────────────────

describe('setNodeBody', () => {
  it('sets the body on the root node', () => {
    const ed = makeEditor([SAMPLE_TEMPLATE]);
    ed.setNodeBody(0, [], 'Updated greeting.');
    const node = ed.getNode(0, []);
    expect(node.body).toBe('Updated greeting.');
  });

  it('getNode returns null for invalid template index', () => {
    const ed = makeEditor([SAMPLE_TEMPLATE]);
    expect(ed.getNode(99, [])).toBeNull();
  });
});

// ── 5. Add response ────────────────────────────────────────────────

describe('addResponse', () => {
  it('response appears in the node', () => {
    const ed = makeEditor([]);
    ed.addTemplate('station', 'on_hailed', 'station');
    ed.addResponse(0, []);
    const node = ed.getNode(0, []);
    expect(node.responses).toHaveLength(1);
    expect(node.responses[0].text).toBe('');
    expect(node.responses[0].actions).toEqual([]);
  });
});

// ── 6. Set response text ───────────────────────────────────────────

describe('setResponseText', () => {
  it('updates the response text', () => {
    const ed = makeEditor([SAMPLE_TEMPLATE]);
    ed.setResponseText(0, [], 0, 'We surrender!');
    const node = ed.getNode(0, []);
    expect(node.responses[0].text).toBe('We surrender!');
  });
});

// ── 7. Remove response ─────────────────────────────────────────────

describe('removeResponse', () => {
  it('removes the response at the given index', () => {
    const ed = makeEditor([SAMPLE_TEMPLATE]);
    ed.removeResponse(0, [], 0);
    const node = ed.getNode(0, []);
    expect(node.responses).toHaveLength(1);
    expect(node.responses[0].text).toBe('We have an emergency!');
  });

  it('is a no-op for out-of-range index', () => {
    const ed = makeEditor([SAMPLE_TEMPLATE]);
    ed.removeResponse(0, [], 99);
    const node = ed.getNode(0, []);
    expect(node.responses).toHaveLength(2);
  });
});

// ── 8. Add follow-up ───────────────────────────────────────────────

describe('addFollowUp', () => {
  it('creates a follow_up node on the response', () => {
    const ed = makeEditor([]);
    ed.addTemplate('station', 'on_hailed', 'station');
    ed.addResponse(0, []);
    ed.addFollowUp(0, [], 0);
    const node = ed.getNode(0, []);
    expect(node.responses[0].follow_up).toBeDefined();
    expect(node.responses[0].follow_up.body).toBe('');
    expect(node.responses[0].follow_up.responses).toEqual([]);
  });
});

// ── 9. Remove follow-up ────────────────────────────────────────────

describe('removeFollowUp', () => {
  it('removes the follow_up node from the response', () => {
    const ed = makeEditor([SAMPLE_TEMPLATE]);
    ed.removeFollowUp(0, [], 1);
    const node = ed.getNode(0, []);
    expect(node.responses[1].follow_up).toBeUndefined();
  });

  it('does not affect other responses', () => {
    const ed = makeEditor([SAMPLE_TEMPLATE]);
    ed.removeFollowUp(0, [], 1);
    const node = ed.getNode(0, []);
    expect(node.responses[0].follow_up).toBeUndefined();
  });
});

// ── 10. Nested follow-up access ─────────────────────────────────────

describe('nested follow-up via nodePath', () => {
  it('accesses root node with empty path', () => {
    const ed = makeEditor([SAMPLE_TEMPLATE]);
    const node = ed.getNode(0, []);
    expect(node.body).toBe('Welcome to Starbase Alpha. State your business.');
    expect(node.responses).toHaveLength(2);
  });

  it('accesses first response follow-up with [0]', () => {
    const ed = makeEditor([SAMPLE_TEMPLATE]);
    // Response 0 has no follow_up, so nodePath [0] should return null
    expect(ed.getNode(0, [0])).toBeNull();
  });

  it('accesses second response follow-up with [1]', () => {
    const ed = makeEditor([SAMPLE_TEMPLATE]);
    const sub = ed.getNode(0, [1]);
    expect(sub).not.toBeNull();
    expect(sub.body).toBe("We'll send a repair team. Stand by.");
    expect(sub.responses).toEqual([]);
  });

  it('returns null for missing follow-up in chain', () => {
    const ed = makeEditor([SAMPLE_TEMPLATE]);
    // Response 0 has no follow_up, so [0, 0] should return null
    expect(ed.getNode(0, [0, 0])).toBeNull();
  });
});

// ── 11. Set body on nested node via nodePath ────────────────────────

describe('set body on nested node via nodePath', () => {
  it('sets body on follow-up of index [1]', () => {
    const ed = makeEditor([SAMPLE_TEMPLATE]);
    ed.setNodeBody(0, [1], 'Negative. We are on our own.');
    const sub = ed.getNode(0, [1]);
    expect(sub.body).toBe('Negative. We are on our own.');
  });

  it('adding and setting body on a deeply nested node', () => {
    const ed = makeEditor([]);
    ed.addTemplate('station', 'on_hailed', 'station');
    ed.addResponse(0, []);
    ed.addFollowUp(0, [], 0);
    // Add response to the follow-up node
    ed.addResponse(0, [0]);
    ed.addFollowUp(0, [0], 0);
    ed.setNodeBody(0, [0, 0], 'Deeply nested reply');
    const deep = ed.getNode(0, [0, 0]);
    expect(deep.body).toBe('Deeply nested reply');
    expect(deep.responses).toEqual([]);
  });
});

// ── 12. toComms serialization ───────────────────────────────────────

describe('toComms serialization', () => {
  it('produces valid structure matching TOML schema', () => {
    const ed = makeEditor([SAMPLE_TEMPLATE]);
    const out = ed.toComms();
    expect(out).toHaveLength(1);
    const t = out[0];
    expect(t).toHaveProperty('from', 'starbase_alpha');
    expect(t).toHaveProperty('trigger');
    expect(t.trigger).toHaveProperty('kind', 'on_hailed');
    expect(t.trigger).toHaveProperty('entity', 'starbase_alpha');
    expect(t).toHaveProperty('node');
    expect(t.node).toHaveProperty('body');
    expect(t.node).toHaveProperty('responses');
    expect(Array.isArray(t.node.responses)).toBe(true);
    expect(t.node.responses).toHaveLength(2);
    expect(t.node.responses[0]).toHaveProperty('text');
    expect(t.node.responses[0]).toHaveProperty('actions');
    expect(Array.isArray(t.node.responses[0].actions)).toBe(true);
  });

  it('follow_up node is serialized when present', () => {
    const ed = makeEditor([SAMPLE_TEMPLATE]);
    const out = ed.toComms();
    expect(out[0].node.responses[1].follow_up).toBeDefined();
    expect(out[0].node.responses[1].follow_up.body).toBe("We'll send a repair team. Stand by.");
    expect(out[0].node.responses[1].follow_up.responses).toEqual([]);
  });

  it('follow_up is absent when not set', () => {
    const ed = makeEditor([SAMPLE_TEMPLATE]);
    const out = ed.toComms();
    expect(out[0].node.responses[0].follow_up).toBeUndefined();
  });

  it('empty editor returns empty array', () => {
    const ed = makeEditor([]);
    expect(ed.toComms()).toEqual([]);
  });
});

// ── 13. Round-trip ──────────────────────────────────────────────────

describe('round-trip', () => {
  it('create template → toComms → load → getTemplates matches', () => {
    const ed1 = makeEditor([]);
    ed1.addTemplate('starbase_alpha', 'on_hailed', 'starbase_alpha');
    ed1.setNodeBody(0, [], 'Welcome.');
    ed1.addResponse(0, []);
    ed1.setResponseText(0, [], 0, 'Greetings.');
    ed1.addResponseAction(0, [], 0, {
      type: 'add_objective',
      id: 'dock',
      text: 'Dock',
      mandatory: true,
    });
    ed1.addFollowUp(0, [], 0);
    ed1.setNodeBody(0, [0], 'Proceed to dock.');
    ed1.addResponse(0, [0]);
    ed1.setResponseText(0, [0], 0, 'Acknowledged.');

    const serialized = ed1.toComms();
    const ed2 = makeEditor(serialized);

    expect(ed2.getTemplates()).toEqual(ed1.getTemplates());
  });

  it('round-trip preserves from, trigger, node tree structure', () => {
    const ed1 = makeEditor([SAMPLE_TEMPLATE]);
    const serialized = ed1.toComms();
    const ed2 = makeEditor(serialized);

    expect(ed2.getTemplates().length).toBe(1);
    expect(ed2.getTemplates()[0].from).toBe(ed1.getTemplates()[0].from);
    expect(ed2.getTemplates()[0].trigger).toEqual(ed1.getTemplates()[0].trigger);
    expect(ed2.getTemplates()[0].node.body).toBe(ed1.getTemplates()[0].node.body);
    expect(ed2.getTemplates()[0].node.responses.length).toBe(
      ed1.getTemplates()[0].node.responses.length,
    );
    expect(ed2.getTemplates()[0].node.responses[1].follow_up.body).toBe(
      ed1.getTemplates()[0].node.responses[1].follow_up.body,
    );
  });
});

// ── 14. removeTemplate ──────────────────────────────────────────────

describe('removeTemplate', () => {
  it('removes the template at the given index', () => {
    const ed = makeEditor([SAMPLE_TEMPLATE]);
    ed.addTemplate('second_station', 'on_hailed', 'second_station');
    expect(ed.getTemplates()).toHaveLength(2);
    ed.removeTemplate(0);
    expect(ed.getTemplates()).toHaveLength(1);
    expect(ed.getTemplates()[0].from).toBe('second_station');
  });

  it('is a no-op for out-of-range index', () => {
    const ed = makeEditor([SAMPLE_TEMPLATE]);
    ed.removeTemplate(99);
    expect(ed.getTemplates()).toHaveLength(1);
  });
});

// ── 15. Response actions ───────────────────────────────────────────

describe('response actions', () => {
  it('getResponseActions returns actions array', () => {
    const ed = makeEditor([SAMPLE_TEMPLATE]);
    const actions = ed.getResponseActions(0, [], 0);
    expect(actions).toHaveLength(1);
    expect(actions[0]).toEqual({
      type: 'add_objective',
      id: 'dock',
      text: 'Dock at Starbase Alpha',
      mandatory: true,
    });
  });

  it('addResponseAction adds an action', () => {
    const ed = makeEditor([]);
    ed.addTemplate('station', 'on_hailed', 'station');
    ed.addResponse(0, []);
    ed.addResponseAction(0, [], 0, {
      type: 'complete_objective',
      id: 'dock',
    });
    const actions = ed.getResponseActions(0, [], 0);
    expect(actions).toHaveLength(1);
    expect(actions[0].type).toBe('complete_objective');
  });

  it('removeResponseAction removes an action', () => {
    const ed = makeEditor([SAMPLE_TEMPLATE]);
    ed.removeResponseAction(0, [], 0, 0);
    const actions = ed.getResponseActions(0, [], 0);
    expect(actions).toHaveLength(0);
  });

  it('getResponseActions returns a defensive copy', () => {
    const ed = makeEditor([SAMPLE_TEMPLATE]);
    const actions = ed.getResponseActions(0, [], 0);
    actions.push({ type: 'fake' });
    expect(ed.getResponseActions(0, [], 0)).toHaveLength(1);
  });
});

// ── 16. Defensive copying on getTemplates ──────────────────────────

describe('defensive copying', () => {
  it('mutating getTemplates result does not affect internal state', () => {
    const ed = makeEditor([SAMPLE_TEMPLATE]);
    const t = ed.getTemplates()[0];
    t.from = 'hacked';
    expect(ed.getTemplates()[0].from).toBe('starbase_alpha');
  });

  it('mutating a nested node body from getNode does not affect internal state', () => {
    const ed = makeEditor([SAMPLE_TEMPLATE]);
    const node = ed.getNode(0, []);
    node.body = 'hacked';
    expect(ed.getNode(0, []).body).toBe('Welcome to Starbase Alpha. State your business.');
  });
});
