import { readFileSync } from 'node:fs';
import { describe, expect, it, vi } from 'vitest';

const SERVER_HTML = readFileSync(new URL('../../server.html', import.meta.url), 'utf8');
const WORKSPACE = readFileSync(new URL('../../gui/gm-workspace.js', import.meta.url), 'utf8');

/** Compile the classic-page Fire seam out of server.html, as #1292's twin does. */
function compileFireSeam({ localGm, wasmSubmit, marker = 'window.__hostFireGmEvent = function(request) {' }) {
  const start = SERVER_HTML.indexOf(marker);
  const end = SERVER_HTML.indexOf('\n    };', start);
  if (start < 0 || end < 0) throw new Error('GM event fire seam not found');
  const source = SERVER_HTML.slice(start, end + '\n    };'.length);
  const win = { wasm_submit_gm_action: wasmSubmit };
  new Function('window', 'localGm', source)(win, localGm);
  return { win, source };
}

describe('server GM mission page seam', () => {
  it('submits the exact attributed Fire for an admitted local GM', () => {
    const wasmSubmit = vi.fn();
    const { win } = compileFireSeam({
      localGm: () => ({ id: 'gm-alex' }),
      wasmSubmit,
    });

    expect(win.__hostFireGmEvent({
      event: 'base-world::breach_alarm',
      correlation: 'gm-fire-7',
    })).toBe(true);
    expect(wasmSubmit).toHaveBeenCalledWith(JSON.stringify({
      operator_id: 'gm-alex',
      correlation: 'gm-fire-7',
      action: 'fire_gm_event',
      event: 'base-world::breach_alarm',
    }));
  });

  it('refuses locally without an admitted GM or a complete request', () => {
    const wasmSubmit = vi.fn();
    const absent = compileFireSeam({ localGm: () => null, wasmSubmit });
    expect(absent.win.__hostFireGmEvent({
      event: 'base-world::breach_alarm',
      correlation: 'gm-fire-8',
    })).toBe(false);

    const admitted = compileFireSeam({
      localGm: () => ({ id: 'gm-alex' }),
      wasmSubmit,
    });
    for (const request of [
      null,
      {},
      { event: 'base-world::breach_alarm' },
      { correlation: 'gm-fire-9' },
      { event: '', correlation: 'gm-fire-9' },
    ]) {
      expect(admitted.win.__hostFireGmEvent(request)).toBe(false);
    }
    expect(wasmSubmit).not.toHaveBeenCalled();
  });

  it('propagates a bounded WASM-ingress refusal instead of showing Pending', () => {
    const wasmSubmit = vi.fn(() => false);
    const { win } = compileFireSeam({
      localGm: () => ({ id: 'gm-alex' }),
      wasmSubmit,
    });

    expect(win.__hostFireGmEvent({
      event: 'base-world::breach_alarm',
      correlation: 'gm-fire-full',
    })).toBe(false);
    expect(wasmSubmit).toHaveBeenCalledOnce();
  });

  it('wires the gm_mission Host Channel into accessible page controls', () => {
    expect(WORKSPACE).toContain("import { createGmMissionPanel } from './gm-mission-panel.js'");
    const gmMissionPanel = { update: vi.fn(), reset: vi.fn(), refreshAdmission: vi.fn() };
    const gmObjectivePanel = { update: vi.fn(), reset: vi.fn(), refreshAdmission: vi.fn() };
    const handler = WORKSPACE.match(/gm_mission:\s+(function\(p\)\s*\{[^\r\n]+\})/)[1];
    const compile = source => new Function('gmMissionPanel', 'gmObjectivePanel', `return (${source});`)(gmMissionPanel, gmObjectivePanel);
    const payload = { events: [], results: [], objective_palette: [], objectives: [], objective_results: [] };
    compile(handler)(payload);
    for (const panel of [gmMissionPanel, gmObjectivePanel]) expect(panel.update).toHaveBeenCalledWith(payload);
    for (const [binding, method] of [['Reset', 'reset'], ['Refresh', 'refreshAdmission']]) {
      const source = WORKSPACE.match(new RegExp(`win\\.__hostGmMission${binding} = ([^\\r\\n]+);`))[1];
      compile(source)();
      for (const panel of [gmMissionPanel, gmObjectivePanel]) expect(panel[method]).toHaveBeenCalledOnce();
    }
    expect(SERVER_HTML).toMatch(/s\.phase === 'Lobby'[\s\S]+window\.__hostGmMissionReset\(\)/);
    expect(SERVER_HTML).toMatch(/id="gm-mission-panel" role="region"/);
    expect(SERVER_HTML).toMatch(/id="gm-mission-events" role="list"/);
    expect(SERVER_HTML).toMatch(/id="gm-mission-log" role="log"[^>]+aria-live="polite"/);
    // The Fire control's label is authored by the panel from the String Table,
    // so the markup must NOT carry player-visible English of its own.
    expect(SERVER_HTML).toMatch(/id="gm-mission-heading" data-i18n="server\.gm\.mission\.heading"/);
  });

  it('submits only the authored Objective vocabulary for the authenticated local operator', () => {
    const wasmSubmit = vi.fn(() => true);
    const marker = 'window.__hostObjectiveAction = function(request) {';
    const { win } = compileFireSeam({ localGm: () => ({ id: 'gm-alex' }), wasmSubmit, marker });
    const request = { operator_id: 'gm-alex', correlation: 'objective-page-1', objective: 'rescue',
      verb: 'activate', recipients: ['ship-a'], text: 'cannot override authored text', score: 999 };
    expect(win.__hostObjectiveAction(request)).toBe(true);
    expect(JSON.parse(wasmSubmit.mock.calls[0][0])).toEqual({ action: 'objective_action',
      operator_id: 'gm-alex', correlation: 'objective-page-1', objective: 'rescue', verb: 'activate', recipients: ['ship-a'] });
    for (const change of [{ operator_id: 'other' }, { verb: 'reopen' }, { recipients: null }, { objective: '' }]) {
      expect(win.__hostObjectiveAction({ ...request, ...change })).toBe(false);
    }
    const absent = compileFireSeam({ localGm: () => null, wasmSubmit, marker });
    expect(absent.win.__hostObjectiveAction(request)).toBe(false);
    expect(wasmSubmit).toHaveBeenCalledOnce();
    wasmSubmit.mockReturnValue(false); expect(win.__hostObjectiveAction(request)).toBe(false);
  });
});
