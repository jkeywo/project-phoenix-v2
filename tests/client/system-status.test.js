// @vitest-environment jsdom
import { describe, it, expect, afterEach } from 'vitest';
import { initConsole } from '../../gui/console-core.js';
import { mountSystemStatus } from '../../gui/components/ph-system-status.js';
import { buildConsoleView } from '../../gui/console-state.js';
import { consoleReadState } from '../../gui/console-topology.js';
import { getTable, setTable } from '../../gui/strings.js';
import { ClientSimState } from '../../gui/sim-state.js';
const live = [];
afterEach(() => { live.splice(0).forEach(widget => widget.dispose()); document.body.replaceChildren(); });
const row = (id, extra = {}) => ({ system_id: id, display_name: 'Same name', availability: 'normal', reasons: [], delegated: false, ...extra });
function mount() { const widget = mountSystemStatus(document); live.push(widget); widget.root.open = true; return widget; }
describe('common System status', () => {
  it('keeps passive unknown equipment and same-name Systems distinct, hiding delegated rows behind inspection', () => {
    const widget = mount(); widget.update([row('left'), row('right', { availability: 'unknown' }), row('auto', { delegated: true, availability: 'unavailable', reasons: [{ code: 'unpowered' }] })]);
    expect([...document.querySelectorAll('[data-system-id]')].map(el => el.dataset.systemId)).toEqual(['left', 'right']);
    expect(document.querySelector('[data-system-id=right]').getAttribute('aria-label')).toContain('Status unknown');
    document.querySelector('[data-delegated-summary]').click();
    expect(document.querySelector('[role=dialog]').textContent).toContain('Insufficient power');
  });
  it('shows literal and translated equipment names in indicators and details', () => {
    const original = getTable();
    setTable(new Map([...original, ['fixture.status.engine', 'Translated engine']]));
    try {
      const widget = mount();
      widget.update([row('dock', { display_name: '' }), row('lateral', { display_name: 'Lateral Thruster' }), row('engine', { display_name: 'fixture.status.engine' })]);
      for (const [id, label] of [['dock', 'dock'], ['lateral', 'Lateral Thruster'], ['engine', 'Translated engine']]) {
        const button = document.querySelector(`[data-system-id=${id}]`);
        expect(button.lastChild.textContent).toBe(label);
        expect(button.getAttribute('aria-label')).toBe(`${label}: Operational`);
        button.click();
        expect(document.querySelector('[role=dialog] h3').textContent).toBe(`${label}: Operational`);
        document.querySelector('[role=dialog] button').click();
      }
      const css = widget.root.querySelector('style').textContent;
      expect(css).toMatch(/\.system-status-strip button,\.system-status-dialog button \{[^}]*color:var\(--ink\)[^}]*background:transparent/);
    } finally { setTable(original); }
  });
  it('a System named delegated opens its own details rather than the AI summary', () => {
    const widget = mount(); widget.update([row('delegated', { availability: 'degraded', reasons: [{ code: 'damaged' }] }), row('ai', { delegated: true, reasons: [{ code: 'unpowered' }] })]);
    document.querySelector('[data-system-id=delegated]').click();
    expect(document.querySelector('[role=dialog]').textContent).toContain('Damaged');
    expect(document.querySelector('[role=dialog]').textContent).not.toContain('Insufficient power');
  });
  it('updates open details, represents multiple reasons and restores keyboard focus on Escape/disposal', () => {
    const widget = mount(); widget.update([row('left')]);
    const button = document.querySelector('[data-system-id=left]'); button.focus(); button.click();
    expect(document.activeElement.textContent).toBe('Close');
    widget.update([row('left', { availability: 'degraded', reasons: [{ code: 'damaged' }, { code: 'offline', component_id: 'link' }] })]);
    expect(button.firstChild.textContent).toBe('✱');
    expect(button.firstChild.dataset.availability).toBe('degraded');
    expect(document.querySelector('[role=dialog]').textContent).toContain('link');
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    expect(document.activeElement).toBe(button);
    button.click(); widget.dispose();
    expect(document.querySelector('[role=dialog]')).toBeNull();
  });
  it('preserves focused controls and live detail nodes across identical snapshots', () => {
    const widget = mount();
    const rows = [row('left', { reasons: [{ code: 'damaged' }] }), row('right')];
    widget.update(rows);
    const button = document.querySelector('[data-system-id=left]');
    button.focus();
    const list = button.parentElement;
    const changes = new MutationObserver(() => {});
    changes.observe(list, { childList: true, subtree: true });
    for (let i = 0; i < 3; i++) widget.update(structuredClone(rows));
    expect(document.activeElement).toBe(button);
    expect(changes.takeRecords()).toEqual([]);
    button.click();
    const detail = document.querySelector('[aria-live=polite]');
    const nodes = [...detail.childNodes];
    changes.observe(detail, { childList: true, subtree: true });
    for (let i = 0; i < 3; i++) widget.update(structuredClone(rows));
    expect([...detail.childNodes]).toEqual(nodes);
    expect(changes.takeRecords()).toEqual([]);
    // Authored order changes still land without recreating the controls.
    widget.update([rows[1], rows[0]]);
    expect([...list.children].map(el => el.dataset.systemId)).toEqual(['right', 'left']);
    expect(list.children[1]).toBe(button);
    changes.disconnect();
  });
  it('projects only owned System rows for cohosted Consoles, including unmodelled passive equipment', () => {
    const state = new ClientSimState();
    state.consoleDefinitions = [{ id: 'one', systems: ['left', 'passive'], feeds: [{ source: 'right', kind: 'Sensors' }] }, { id: 'two', systems: ['right'] }];
    state.systemStatus = { left: row('left'), right: row('right', { availability: 'unavailable' }) };
    state.blackboardKinds = { right: 'Sensors' }; state.blackboards = { right: {} };
    expect(buildConsoleView('one', state).system_status.map(r => [r.system_id, r.availability])).toEqual([['left', 'normal'], ['passive', 'unknown']]);
    expect(buildConsoleView('two', state).system_status.map(r => r.system_id)).toEqual(['right']);
  });
  it('read feeds reveal only disclosed repair damage and never grant control or private power reasons', () => {
    const definition = { id: 'repair-view', systems: [], feeds: [{ source: 'repair', kind: 'Repair' }] };
    const view = consoleReadState({ systemStatus: { foreign: row('foreign', { reasons: [{ code: 'unpowered' }] }) },
      blackboardKinds: { repair: 'Repair' }, blackboards: { repair: { system_hull: [{ system_id: 'foreign', display_name: 'Other', tier: 'Damaged' }] } } }, definition);
    expect(view.systemStatus.foreign).toMatchObject({ availability: 'degraded', reasons: [{ code: 'damaged' }] });
    expect(view.consoleSystems['repair-view']).toEqual([]);
  });
  it('Console core mounts one shared widget, clears absent data and disposes it', () => {
    const runtime = initConsole({ name: 'helm', render() {} });
    try {
      const payload = { console_id: 'helm-left', system_status: [row('passive', { availability: 'unknown' })] };
      window.__updateConsole('helm-left', JSON.stringify(payload));
      window.__updateConsole('helm-left', JSON.stringify(payload));
      expect(document.querySelectorAll('.system-status-strip')).toHaveLength(1);
      expect(document.querySelector('[data-system-id=passive]')).not.toBeNull();
      window.__updateConsole('helm-left', '{}');
      expect(document.querySelector('.system-status-strip').hidden).toBe(true);
    } finally { runtime.disposeSemanticActions(); }
    expect(document.querySelector('.system-status-strip')).toBeNull();
  });
  it('replaces status on every snapshot and clears it on reset', () => {
    const state = new ClientSimState();
    state.apply({ type: 'SimState', data: { snapshot: { system_status: { left: row('left') } } } });
    expect(state.systemStatus.left).toBeDefined();
    state.apply({ type: 'SimState', data: { snapshot: {} } }); expect(state.systemStatus).toEqual({});
    state.systemStatus = { left: row('left') }; state.reset(); expect(state.systemStatus).toEqual({});
  });
});
