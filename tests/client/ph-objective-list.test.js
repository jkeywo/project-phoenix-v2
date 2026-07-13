// @vitest-environment jsdom
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import '../../gui/components/ph-objective-list.js';

function setup(opts) {
  const sendAction = opts && opts.sendAction;
  if (sendAction) {
    window.sendAction = sendAction;
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
    delete window.sendAction;
  });

  afterEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
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
    expect(queryText(el, '.list')).toBe('NO OBJECTIVES');
  });

  it('renders NO OBJECTIVES placeholder when state is null', () => {
    const { el } = setup();
    el.state = null;
    expect(queryText(el, '.list')).toBe('NO OBJECTIVES');
  });

  it('renders NO OBJECTIVES placeholder when objectives is null', () => {
    const { el } = setup();
    el.state = { objectives: null };
    expect(queryText(el, '.list')).toBe('NO OBJECTIVES');
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

  it('clicking an objective row calls sendAction with set_objective_priority', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      objectives: [
        { id: 'obj-1', text: 'Scan the anomaly', done: false },
        { id: 'obj-2', text: 'Hail the vessel', done: false },
      ],
    };
    const rows = el.shadowRoot.querySelectorAll('.row');
    rows[1].click();
    expect(sendAction).toHaveBeenCalledTimes(1);
    expect(sendAction).toHaveBeenCalledWith('set_objective_priority', { id: 'obj-2' });
  });

  it('does not throw when sendAction is not set and row is clicked', () => {
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
