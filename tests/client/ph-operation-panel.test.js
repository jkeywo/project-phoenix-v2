// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { STATE_LABELS } from '../../gui/components/ph-operation-panel.js';
import '../../gui/components/ph-operation-panel.js';

function setup() {
  document.body.innerHTML = '<ph-operation-panel id="test-panel"></ph-operation-panel>';
  return document.getElementById('test-panel');
}

const CAPABILITY = { verb: 'stabilise', label: 'operation.verb.stabilise' };

const HOLDING = {
  id: 1,
  verb: 'stabilise',
  verb_label: 'operation.verb.stabilise',
  target_uuid: '00000000-0000-8000-8000-000000000042',
  target_name: 'world.entity.skyhook_depot.name',
  progress: 0.4,
  state: 'holding',
};

function panelState(overrides = {}) {
  return {
    operations: { capabilities: [CAPABILITY], active: HOLDING, refusal: null },
    target_uuid: '00000000-0000-8000-8000-000000000042',
    ...overrides,
  };
}

function fill(el) {
  return el.shadowRoot.querySelector('.fill').style.width;
}

function button(el) {
  return el.shadowRoot.getElementById('action');
}

describe('PhOperationPanel', () => {
  beforeEach(() => { document.body.innerHTML = ''; });
  afterEach(() => { document.body.innerHTML = ''; });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-operation-panel')).toBeDefined();
  });

  it('tells a hull that can perform nothing that it can perform nothing', () => {
    // Distinct from "capable and idle": an empty box would leave the crew
    // wondering whether the panel was broken.
    const el = setup();
    el.state = { operations: { capabilities: [], active: null } };
    expect(el.shadowRoot.querySelector('.empty').textContent)
      .toBe(t('component.operations.none'));
    expect(button(el).hidden).toBe(true);
  });

  it('renders a capable idle hull with a start button', () => {
    const el = setup();
    el.state = panelState({ operations: { capabilities: [CAPABILITY], active: null } });
    expect(el.shadowRoot.querySelector('.empty').textContent)
      .toBe(t('component.operations.idle'));
    expect(button(el).hidden).toBe(false);
    expect(button(el).textContent).toBe(t('component.operations.start'));
  });

  it('offers no start button when the console has no target selected', () => {
    // The operation has to be ordered against something; the captain names it
    // by selecting it on the sensor radar first.
    const el = setup();
    el.state = {
      operations: { capabilities: [CAPABILITY], active: null },
      target_uuid: null,
    };
    expect(button(el).hidden).toBe(true);
    expect(el.action).toBeNull();
  });

  it('paints the server-computed progress and never a clock of its own', () => {
    const el = setup();
    el.state = panelState();
    expect(fill(el)).toBe('40%');
    // A second payload at the same progress must paint the same width — the
    // component holds no timer that could advance it between updates.
    el.state = panelState();
    expect(fill(el)).toBe('40%');
    el.state = panelState({
      operations: { capabilities: [CAPABILITY], active: { ...HOLDING, progress: 0.75 } },
    });
    expect(fill(el)).toBe('75%');
  });

  it('clamps a progress outside 0..1 rather than painting past the track', () => {
    const el = setup();
    for (const [progress, expected] of [[-0.5, '0%'], [1.5, '100%']]) {
      el.state = panelState({
        operations: { capabilities: [CAPABILITY], active: { ...HOLDING, progress } },
      });
      expect(fill(el)).toBe(expected);
    }
  });

  it('resolves the verb and target through the string table rather than rendering ids', () => {
    // No English crosses the wire: the payload carries strings.csv ids.
    const el = setup();
    el.state = panelState();
    const verb = el.shadowRoot.querySelector('.verb').textContent;
    expect(verb).toContain(t('operation.verb.stabilise'));
    expect(verb).toContain(t('world.entity.skyhook_depot.name'));
    expect(verb).not.toContain('operation.verb.stabilise');
  });

  it('shows a stalled hold with its reason, and freezes the bar where it stood', () => {
    const el = setup();
    el.state = panelState({
      operations: {
        capabilities: [CAPABILITY],
        active: { ...HOLDING, state: 'stalled', reason: 'operation.refused.out_of_range' },
      },
    });
    expect(el.shadowRoot.querySelector('.state').textContent)
      .toBe(t('component.operations.state.stalled'));
    expect(el.shadowRoot.getElementById('reason').textContent)
      .toBe(t('operation.refused.out_of_range'));
    expect(fill(el)).toBe('40%');
    expect(el.getAttribute('data-state')).toBe('stalled');
  });

  it('renders every terminal state as its own word rather than a stale bar', () => {
    // 'completed' at 100% and 'failed' at 100% would be the same picture; the
    // word is what tells them apart, and the mission cares about the difference.
    const el = setup();
    for (const state of ['completed', 'aborted', 'failed']) {
      el.state = panelState({
        operations: {
          capabilities: [CAPABILITY],
          active: { ...HOLDING, progress: 1, state },
        },
      });
      expect(el.shadowRoot.querySelector('.state').textContent)
        .toBe(t(STATE_LABELS[state]));
      expect(el.getAttribute('data-state')).toBe(state);
    }
  });

  it('offers abort while an operation is live and start again once it has settled', () => {
    const el = setup();
    el.state = panelState();
    expect(el.action).toEqual({ action: 'abort_operation' });
    expect(button(el).textContent).toBe(t('component.operations.abort'));

    el.state = panelState({
      operations: {
        capabilities: [CAPABILITY],
        active: { ...HOLDING, progress: 1, state: 'completed' },
      },
    });
    expect(el.action).toEqual({
      action: 'start_operation',
      verb: 'stabilise',
      target_uuid: '00000000-0000-8000-8000-000000000042',
    });
  });

  it('a stalled operation is still live, so its button stands it down', () => {
    // The distinction that matters: stalled is an operation in trouble, not an
    // operation that is over, so the control has to be the abort.
    const el = setup();
    el.state = panelState({
      operations: {
        capabilities: [CAPABILITY],
        active: { ...HOLDING, state: 'stalled', reason: 'operation.refused.out_of_range' },
      },
    });
    expect(el.action).toEqual({ action: 'abort_operation' });
  });

  it('sends the action it advertises when its button is pressed', () => {
    const el = setup();
    const sent = [];
    el.sendAction = (name, payload) => sent.push([name, payload]);
    el.state = panelState();
    button(el).click();
    expect(sent).toEqual([['abort_operation', {}]]);
  });

  it('shows a start refusal, which is a different thing from a stall', () => {
    // No operation was opened at all — the crew act on that differently from
    // one that opened and is not advancing.
    const el = setup();
    el.state = panelState({
      operations: {
        capabilities: [CAPABILITY],
        active: null,
        refusal: 'operation.refused.not_capable',
      },
    });
    expect(el.shadowRoot.getElementById('reason').textContent)
      .toBe(t('operation.refused.not_capable'));
  });

  it('survives a null state and an absent operations block', () => {
    const el = setup();
    for (const state of [null, {}, { operations: null }]) {
      el.state = state;
      expect(el.shadowRoot.querySelector('.empty')).not.toBeNull();
    }
  });

  it('has a string-table row behind every state it can render', () => {
    for (const [code, id] of Object.entries(STATE_LABELS)) {
      expect(t(id), `state ${code} renders a raw id`).not.toContain('⟨');
    }
  });
});
