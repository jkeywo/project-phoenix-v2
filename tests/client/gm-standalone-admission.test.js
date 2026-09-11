/**
 * A standalone game master's desk is admitted (defect: every action dead).
 *
 * The reported symptom was a GM console whose action verbs were permanently
 * disabled. Every panel gates its controls on `!!operator()` and `operator()`
 * is `window.__hostLocalGm()`, so the whole defect reduces to one page
 * question — does `localGm()` resolve for a peer with no fleet? These tests
 * compile the real seam out of `server.html` and ask it, then prove the panels
 * that read it enable and disable accordingly.
 */
// @vitest-environment jsdom

import { readFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it, vi } from 'vitest';

import { createGmSessionControls } from '../../gui/gm-session-controls.js';
import { t } from '../../gui/strings.js';

// Resolved from the process cwd, not `import.meta.url`: under the jsdom
// environment this module's URL is an http one and `new URL(...)` cannot be
// handed to `readFileSync`.
const ROOT = process.cwd();
const SERVER_HTML = readFileSync(path.join(ROOT, 'server.html'), 'utf8');
const STRINGS = readFileSync(path.join(ROOT, 'assets', 'strings', 'strings.csv'), 'utf8');

/**
 * Compile the page's own `normaliseFleetRole`/`standaloneGm`/`localGm` block
 * with its free variables injected, so the test exercises the shipped source
 * rather than a restatement of it.
 */
function compileLocalGm({
  standalone = false,
  readBack = () => '',
  fleetHandle = null,
  fleetRoster = null,
  fleetRole = 'gm',
} = {}) {
  const start = SERVER_HTML.indexOf('    function normaliseFleetRole(role) {');
  const marker = '    function gmName(operatorId) {';
  const end = SERVER_HTML.indexOf(marker, start);
  if (start < 0 || end < 0) throw new Error('local GM seam not found in server.html');
  const source = SERVER_HTML.slice(start, end);
  const win = { wasm_local_gm_operator: vi.fn(readBack) };
  const t = vi.fn((id) => `t:${id}`);
  const factory = new Function(
    'window', 't', 'gmStandalone', 'fleetHandle', 'fleetRole', 'fleetRoster',
    `${source}\nreturn { localGm, standaloneGm };`,
  );
  return {
    win,
    t,
    ...factory(win, t, () => standalone, fleetHandle, fleetRole, fleetRoster),
  };
}

const BOUND = JSON.stringify({ id: 'gm-1', name: '', connected: true, ready: false });

describe('standalone game master admission', () => {
  it('admits the operator the simulation bound, under a String Table display name', () => {
    const { localGm, win, t } = compileLocalGm({ standalone: true, readBack: () => BOUND });

    expect(localGm()).toEqual({
      id: 'gm-1',
      name: 't:server.gm.operator.standalone',
      connected: true,
      ready: false,
    });
    expect(win.wasm_local_gm_operator).toHaveBeenCalled();
    expect(t).toHaveBeenCalledWith('server.gm.operator.standalone');
    // The id is the simulation's, never the page's.
    expect(STRINGS).toContain('server.gm.operator.standalone,');
  });

  it('stays unadmitted when the simulation bound nothing, or bound someone absent', () => {
    expect(compileLocalGm({ standalone: true, readBack: () => '' }).localGm()).toBeNull();
    expect(compileLocalGm({ standalone: true, readBack: () => 'not json' }).localGm()).toBeNull();
    expect(
      compileLocalGm({
        standalone: true,
        readBack: () => JSON.stringify({ id: 'gm-1', name: '', connected: false, ready: false }),
      }).localGm(),
    ).toBeNull();
    expect(
      compileLocalGm({
        standalone: true,
        readBack: () => JSON.stringify({ id: '', name: '', connected: true, ready: false }),
      }).localGm(),
    ).toBeNull();
  });

  it('never reads back an identity for a peer that is not standalone', () => {
    const { localGm, win } = compileLocalGm({ standalone: false, readBack: () => BOUND });
    expect(localGm()).toBeNull();
    expect(win.wasm_local_gm_operator).not.toHaveBeenCalled();
  });

  it('leaves a fleet game master on its own roster identity', () => {
    const { localGm, win } = compileLocalGm({
      standalone: false,
      readBack: () => BOUND,
      fleetHandle: { role: 'gm', operatorId: 'gm-2', gmJoinCandidate: false },
      fleetRoster: { gms: [{ id: 'gm-2', name: 'Morgan', connected: true, ready: true }] },
    });

    expect(localGm()).toEqual({ id: 'gm-2', name: 'Morgan', connected: true, ready: true });
    expect(win.wasm_local_gm_operator).not.toHaveBeenCalled();
  });
});

describe('the panels that read it', () => {
  function mountSessionControls(getOperator) {
    document.body.innerHTML = `
      <section id="gm-session-controls">
        <h2 id="gm-session-heading"></h2>
        <button id="gm-session-pause"></button>
        <button id="gm-session-resume"></button>
        <p id="gm-session-state"></p>
        <p id="gm-session-feedback"></p>
        <h3 id="gm-session-log-heading"></h3>
        <ol id="gm-session-log" aria-labelledby="gm-session-log-heading"></ol>
      </section>`;
    return createGmSessionControls({
      doc: document,
      win: window,
      t,
      submitSessionPaused: () => true,
      getOperator,
      getOperatorName: (id) => id,
      correlation: () => 'gm-session-1',
      now: () => 101,
    });
  }

  it('enables the session verbs for a bound standalone operator', () => {
    const { localGm } = compileLocalGm({ standalone: true, readBack: () => BOUND });
    const controls = mountSessionControls(() => localGm());
    expect(controls.refreshAdmission()).toBe(true);
    expect(document.getElementById('gm-session-pause').disabled).toBe(false);
    expect(document.getElementById('gm-session-resume').disabled).toBe(false);
    expect(document.getElementById('gm-session-controls').dataset.admitted).toBe('true');
  });

  it('still disables them when the simulation bound no operator at all', () => {
    const { localGm } = compileLocalGm({ standalone: true, readBack: () => '' });
    const controls = mountSessionControls(() => localGm());
    expect(controls.refreshAdmission()).toBe(false);
    expect(document.getElementById('gm-session-pause').disabled).toBe(true);
    expect(document.getElementById('gm-session-controls').dataset.admitted).toBe('false');
  });
});
