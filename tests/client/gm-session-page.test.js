import { readFileSync } from 'node:fs';
import { describe, expect, it, vi } from 'vitest';

const SERVER_HTML = readFileSync(new URL('../../server.html', import.meta.url), 'utf8');

function compileClassicSeam({ localGm, wasmSubmit }) {
  const marker = 'window.__hostSetSessionPaused = function(active, correlation) {';
  const start = SERVER_HTML.indexOf(marker);
  const end = SERVER_HTML.indexOf('\n    };', start);
  if (start < 0 || end < 0) throw new Error('GM session classic seam not found');
  const source = SERVER_HTML.slice(start, end + '\n    };'.length);
  const win = {
    wasm_submit_gm_action: wasmSubmit,
    wasm_toggle_pause: vi.fn(() => { throw new Error('legacy toggle must not be called'); }),
  };
  new Function('window', 'localGm', source)(win, localGm);
  return { win, source };
}

describe('server GM session page seam', () => {
  it('submits the exact attributed absolute-state command for an admitted local GM', () => {
    const wasmSubmit = vi.fn();
    const { win, source } = compileClassicSeam({
      localGm: () => ({ id: 'gm-alex' }),
      wasmSubmit,
    });

    expect(win.__hostSetSessionPaused(1, 'gm-correlation-7')).toBe(true);
    expect(wasmSubmit).toHaveBeenCalledWith(JSON.stringify({
      operator_id: 'gm-alex',
      correlation: 'gm-correlation-7',
      action: 'set_session_paused',
      active: true,
    }));
    expect(win.wasm_toggle_pause).not.toHaveBeenCalled();
    expect(source).not.toContain('wasm_toggle_pause');
  });

  it('refuses locally without an admitted GM and never reaches either WASM mutation', () => {
    const wasmSubmit = vi.fn();
    const { win } = compileClassicSeam({ localGm: () => null, wasmSubmit });

    expect(win.__hostSetSessionPaused(false, 'gm-correlation-8')).toBe(false);
    expect(wasmSubmit).not.toHaveBeenCalled();
    expect(win.wasm_toggle_pause).not.toHaveBeenCalled();
  });

  it('propagates a bounded WASM-ingress refusal instead of showing Pending', () => {
    const wasmSubmit = vi.fn(() => false);
    const { win } = compileClassicSeam({
      localGm: () => ({ id: 'gm-alex' }),
      wasmSubmit,
    });

    expect(win.__hostSetSessionPaused(true, 'gm-correlation-full')).toBe(false);
    expect(wasmSubmit).toHaveBeenCalledOnce();
    expect(win.wasm_toggle_pause).not.toHaveBeenCalled();
  });

  it('wires the gm_session Host Channel into accessible page controls', () => {
    expect(SERVER_HTML).toContain("import { createGmSessionControls } from './gui/gm-session-controls.js'");
    expect(SERVER_HTML).toMatch(/gm_session:\s+function\(p\) \{ gmSessionControls\.update\(p\); \}/);
    expect(SERVER_HTML).toContain('window.__hostSemanticActions = hostSemanticActions');
    expect(SERVER_HTML).toContain('window.__hostActionFeedback = hostActionFeedback');
    expect(SERVER_HTML).toContain('window.__hostGmSessionReset = gmSessionControls.reset');
    expect(SERVER_HTML).toMatch(/s\.phase === 'Lobby'[\s\S]+window\.__hostGmSessionReset\(\)/);
    expect(SERVER_HTML).toContain('window.wasm_submit_gm_action = wasmBindings.wasm_submit_gm_action');
    expect(SERVER_HTML).toMatch(/id="gm-session-controls" role="region"/);
    expect(SERVER_HTML).toMatch(/id="gm-session-state" role="status" aria-live="polite"/);
    expect(SERVER_HTML).toMatch(/id="gm-session-log" role="log"[^>]+aria-live="polite"/);
  });
});
