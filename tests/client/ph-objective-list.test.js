// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { CAPTAIN_OBJECTIVE_PRIORITY_ACTION_ID } from '../../gui/stations/captain-actions.js';
import '../../gui/components/ph-objective-list.js';

function setup(opts) {
  const activateSemanticAction = opts && opts.activateSemanticAction;
  if (activateSemanticAction) {
    window.activateSemanticAction = activateSemanticAction;
  }
  document.body.innerHTML = '<ph-objective-list id="test-panel"></ph-objective-list>';
  const el = document.getElementById('test-panel');
  return { el };
}

function queryText(host, sel) {
  const el = host.shadowRoot.querySelector(sel);
  return el ? el.textContent.trim() : null;
}

describe('PhObjectiveList', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
    delete window.activateSemanticAction;
  });

  afterEach(() => {
    document.body.innerHTML = '';
    delete window.activateSemanticAction;
  });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-objective-list')).toBeDefined();
  });

  it('creates a shadow root', () => {
    const { el } = setup();
    expect(el.shadowRoot).toBeDefined();
  });

  it('renders NO OBJECTIVES placeholder when objectives is an empty array', () => {
    const { el } = setup();
    el.state = { objectives: [] };
    expect(queryText(el, '.list')).toBe(t('component.objectives.empty'));
  });

  it('renders NO OBJECTIVES placeholder when state is null', () => {
    const { el } = setup();
    el.state = null;
    expect(queryText(el, '.list')).toBe(t('component.objectives.empty'));
  });

  it('renders NO OBJECTIVES placeholder when objectives is null', () => {
    const { el } = setup();
    el.state = { objectives: null };
    expect(queryText(el, '.list')).toBe(t('component.objectives.empty'));
  });

  it('renders a mix of done and pending objectives', () => {
    const { el } = setup();
    el.state = {
      objectives: [
        { id: 'obj-1', text: 'Scan the anomaly', done: true },
        { id: 'obj-2', text: 'Hail the vessel', done: false },
        { id: 'obj-3', text: 'Report to command', done: true },
      ],
    };
    const rows = el.shadowRoot.querySelectorAll('.row');
    expect(rows.length).toBe(3);
    const textContents = Array.from(rows).map(r => r.querySelector('.text').textContent.trim());
    expect(textContents).toEqual(['Scan the anomaly', 'Hail the vessel', 'Report to command']);
  });

  it('marks done items visually with .done class on row and indicator', () => {
    const { el } = setup();
    el.state = {
      objectives: [
        { id: 'obj-1', text: 'Completed task', done: true },
        { id: 'obj-2', text: 'Pending task', done: false },
      ],
    };
    const rows = el.shadowRoot.querySelectorAll('.row');
    expect(rows[0].classList.contains('done')).toBe(true);
    expect(rows[1].classList.contains('done')).toBe(false);
    const indicators = el.shadowRoot.querySelectorAll('.indicator');
    expect(indicators[0].classList.contains('done')).toBe(true);
    expect(indicators[0].classList.contains('pending')).toBe(false);
    expect(indicators[1].classList.contains('pending')).toBe(true);
    expect(indicators[1].classList.contains('done')).toBe(false);
  });

  it('normalizes status === "Completed" as done=true', () => {
    const { el } = setup();
    el.state = {
      objectives: [
        { id: 'obj-1', text: 'Done via status', status: 'Completed' },
        { id: 'obj-2', text: 'Active objective', status: 'Active' },
        { id: 'obj-3', text: 'Failed objective', status: 'Failed' },
      ],
    };
    const rows = el.shadowRoot.querySelectorAll('.row');
    expect(rows[0].classList.contains('done')).toBe(true);
    expect(rows[1].classList.contains('done')).toBe(false);
    expect(rows[2].classList.contains('done')).toBe(false);
  });

  it('prefers explicit done field over status field', () => {
    const { el } = setup();
    el.state = {
      objectives: [
        { id: 'obj-1', text: 'Overridden', done: false, status: 'Completed' },
      ],
    };
    const rows = el.shadowRoot.querySelectorAll('.row');
    expect(rows[0].classList.contains('done')).toBe(false);
  });

  it('updates display when state changes', () => {
    const { el } = setup();
    el.state = {
      objectives: [{ id: 'obj-1', text: 'First objective', done: false }],
    };
    expect(queryText(el, '.text')).toBe('First objective');
    el.state = {
      objectives: [
        { id: 'obj-1', text: 'First objective', done: true },
        { id: 'obj-2', text: 'New objective', done: false },
      ],
    };
    const rows = el.shadowRoot.querySelectorAll('.row');
    expect(rows.length).toBe(2);
    expect(rows[0].classList.contains('done')).toBe(true);
    expect(queryText(el, '.empty')).toBeNull();
  });

  it('routes a visible objective through the parameterised Captain semantic identity', () => {
    const activateSemanticAction = vi.fn();
    const { el } = setup({ activateSemanticAction });
    el.state = {
      objectives: [
        { id: 'obj-1', text: 'Scan the anomaly', done: false },
        { id: 'obj-2', text: 'Hail the vessel', done: false },
      ],
    };
    const rows = el.shadowRoot.querySelectorAll('.row');
    rows[1].click();
    expect(activateSemanticAction).toHaveBeenCalledTimes(1);
    expect(activateSemanticAction).toHaveBeenCalledWith(
      CAPTAIN_OBJECTIVE_PRIORITY_ACTION_ID,
      expect.objectContaining({
        context: 'captain', source: 'control', detail: { id: 'obj-2' },
      }),
    );
  });

  it('keeps unassigned history visible without a Captain action', () => {
    const activateSemanticAction = vi.fn();
    const { el } = setup({ activateSemanticAction });
    el.state = {
      objectives: [{ id: 'obj-1', text: 'Hold the line', unassigned: true }],
      boosted_objective_id: 'obj-1',
    };
    const row = el.shadowRoot.querySelector('.row');
    expect(row.textContent).toContain(t('component.objectives.unassigned'));
    expect(row.getAttribute('aria-disabled')).toBe('true');
    expect(row.getAttribute('aria-selected')).toBe('false');
    expect(row.tabIndex).toBe(-1);
    row.click();
    expect(activateSemanticAction).not.toHaveBeenCalled();

    el.state = { objectives: [{ id: 'obj-1', text: 'Hold the line', unassigned: false }] };
    expect(row.getAttribute('aria-disabled')).toBe('false');
    row.click();
    expect(activateSemanticAction).toHaveBeenCalledTimes(1);
  });

  it('does not throw when semantic activation is unavailable and row is clicked', () => {
    const { el } = setup();
    el.state = {
      objectives: [{ id: 'obj-1', text: 'Scan the anomaly', done: false }],
    };
    const row = el.shadowRoot.querySelector('.row');
    expect(() => row.click()).not.toThrow();
  });

  it('marks the row matching boosted_objective_id with the boosted class', () => {
    const { el } = setup();
    el.state = {
      objectives: [
        { id: 'obj-1', text: 'Scan the anomaly', done: false },
        { id: 'obj-2', text: 'Hail the vessel', done: false },
        { id: 'obj-3', text: 'Report to command', done: false },
      ],
      boosted_objective_id: 'obj-2',
    };
    const rows = el.shadowRoot.querySelectorAll('.row');
    expect(rows[0].classList.contains('boosted')).toBe(false);
    expect(rows[1].classList.contains('boosted')).toBe(true);
    expect(rows[2].classList.contains('boosted')).toBe(false);
  });

  it('marks no row as boosted when boosted_objective_id is null', () => {
    const { el } = setup();
    el.state = {
      objectives: [
        { id: 'obj-1', text: 'Scan the anomaly', done: false },
        { id: 'obj-2', text: 'Hail the vessel', done: false },
      ],
      boosted_objective_id: null,
    };
    const rows = el.shadowRoot.querySelectorAll('.row');
    expect(rows[0].classList.contains('boosted')).toBe(false);
    expect(rows[1].classList.contains('boosted')).toBe(false);
  });

  it('marks no row as boosted when boosted_objective_id is absent', () => {
    const { el } = setup();
    el.state = {
      objectives: [
        { id: 'obj-1', text: 'Scan the anomaly', done: false },
      ],
    };
    const rows = el.shadowRoot.querySelectorAll('.row');
    expect(rows[0].classList.contains('boosted')).toBe(false);
  });
});


describe('Objective instance progress', () => {
  it('renders current and frozen progress, then replaces the departed instance', () => {
    const { el } = setup();
    el.state = { objectives: [{ id: 'hold::one', text: 'Hold', progress: 0.25 }] };
    expect(queryText(el, '.text')).toContain(t('component.objectives.progress', { progress: 0.25 }));
    el.state = { objectives: [{ id: 'hold::one', text: 'Hold', progress: 0.25, unassigned: true }] };
    expect(queryText(el, '.text')).toContain(t('component.objectives.progress', { progress: 0.25 }));
    expect(queryText(el, '.text')).toContain(t('component.objectives.unassigned'));
    el.state = { objectives: [{ id: 'hold::two', text: 'Hold', progress: 7, status: 'Completed' }] };
    expect(el.shadowRoot.querySelectorAll('.row')).toHaveLength(1);
    expect(queryText(el, '.text')).toContain(t('component.objectives.progress', { progress: 7 }));
    expect(queryText(el, '.text')).not.toContain(t('component.objectives.unassigned'));
    expect(el.shadowRoot.querySelector('.row').classList.contains('done')).toBe(true);
    el.state = { objectives: [{ id: 'legacy', text: 'Legacy' }] };
    expect(queryText(el, '.text')).toBe('Legacy');
  });
});
