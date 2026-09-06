import { readFileSync } from 'node:fs';
import { describe, expect, it, vi } from 'vitest';

const SERVER_HTML = readFileSync(new URL('../../server.html', import.meta.url), 'utf8');

/** Compile the classic-page Fire seam out of server.html, as #1292's twin does. */
function compileFireSeam({ localGm, wasmSubmit }) {
  const marker = 'window.__hostFireGmEvent = function(request) {';
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
    expect(SERVER_HTML).toContain("import { createGmMissionPanel } from './gui/gm-mission-panel.js'");
    expect(SERVER_HTML).toMatch(/gm_mission:\s+function\(p\) \{ gmMissionPanel\.update\(p\); \}/);
    expect(SERVER_HTML).toContain('window.__hostGmMissionReset = gmMissionPanel.reset');
    expect(SERVER_HTML).toMatch(/s\.phase === 'Lobby'[\s\S]+window\.__hostGmMissionReset\(\)/);
    expect(SERVER_HTML).toMatch(/id="gm-mission-panel" role="region"/);
    expect(SERVER_HTML).toMatch(/id="gm-mission-events" role="list"/);
    expect(SERVER_HTML).toMatch(/id="gm-mission-log" role="log"[^>]+aria-live="polite"/);
    // The Fire control's label is authored by the panel from the String Table,
    // so the markup must NOT carry player-visible English of its own.
    expect(SERVER_HTML).toMatch(/id="gm-mission-heading" data-i18n="server\.gm\.mission\.heading"/);
  });
});
