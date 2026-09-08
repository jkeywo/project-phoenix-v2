import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const SERVER_HTML = readFileSync(path.join(root, 'server.html'), 'utf8');
const WORKSPACE = readFileSync(new URL('../../gui/gm-workspace.js', import.meta.url), 'utf8');

describe('first-time GM join host surface (issue #1293)', () => {
  it('puts visible Accept Reject and pause state on the shared host-class page', () => {
    for (const id of [
      'gm-join-controls',
      'gm-join-state',
      'gm-join-pause-state',
      'gm-join-accept',
      'gm-join-reject',
    ]) {
      expect(SERVER_HTML).toContain(`id="${id}"`);
    }
    expect(SERVER_HTML).toContain("'server.gm.join.pause_active'");
    expect(SERVER_HTML).toContain("'server.gm.join.pause_inactive'");
  });

  it('queues acceptance into Rust and commits transport only from terminal progress', () => {
    expect(SERVER_HTML).toContain('window.wasm_begin_gm_join = wasmBindings.wasm_begin_gm_join');
    expect(SERVER_HTML).toContain('window.wasm_gm_join_status = wasmBindings.wasm_gm_join_status');
    expect(SERVER_HTML).toContain('onBeginGmJoin: beginFleetGmJoin');
    expect(SERVER_HTML).toContain('BigInt(request.id),');
    expect(SERVER_HTML).toContain('wasm_prepare_gm_join_candidate(BigInt(id), encoded)');
    expect(SERVER_HTML).toContain('wasm_refuse_gm_join(BigInt(id), reason)');
    expect(SERVER_HTML).toContain("progress.status === 'committed' || progress.status === 'refused'");
    expect(SERVER_HTML).toContain('fleetHandle.completeGmJoin(id, progress.status');
  });

  it('wires the same request and status callbacks on owner and member handles', () => {
    expect(SERVER_HTML.match(/onGmJoinRequest: receiveFleetGmJoinRequest/g)).toHaveLength(2);
    expect(SERVER_HTML.match(/onGmJoinStatus: receiveFleetGmJoinStatus/g)).toHaveLength(2);
  });
});

describe('known GM reconnect host surface (issue #1294)', () => {
  it('auto-starts the typed reconnect transaction without offering a decision', () => {
    expect(SERVER_HTML).toContain("fleetGmJoinRequest.kind !== 'reconnect'");
    expect(SERVER_HTML).toContain("request.kind || 'first-time'");
    expect(SERVER_HTML).toContain('|| fleetHandle.gmJoinCandidate) return null;');
  });

  it('keeps the role preset in the private stored identity only', () => {
    expect(SERVER_HTML).toContain('reconnectCredential: identity.reconnectCredential,');
    expect(SERVER_HTML).not.toContain('operator.rolePreset');
    expect(SERVER_HTML).not.toContain('rolePreset: identity.rolePreset,');
  });
});

describe('presentation-only GM role presets (issue #1319)', () => {
  it('never sends a role preset to Rust — the roster publish and GM action payloads stay clean', () => {
    expect(SERVER_HTML).not.toMatch(/wasm_set_gm_roster[\s\S]{0,400}rolePreset/);
    expect(SERVER_HTML).not.toMatch(/wasm_submit_gm_action[\s\S]{0,400}rolePreset/);
  });

  it('seeds the selector from storage before the identity even reconnects, and keys live switches to it', () => {
    expect(SERVER_HTML).toContain('gmRolePresetIdentityCode = identityCode;');
    expect(SERVER_HTML).toContain(
      "if (priorIdentity && typeof priorIdentity.rolePreset === 'string'\n"
      + "          && typeof window.__hostGmRolePresetsRestore === 'function') {\n"
      + '        window.__hostGmRolePresetsRestore(priorIdentity.rolePreset);\n'
      + '      }',
    );
  });

  it('never resets the live selection back to All when there is nothing stored to restore -- a mistyped code, a different fleet, or a crew-role join must leave the operator\'s own current choice alone', () => {
    // Guards the exact regression a bare
    // `window.__hostGmRolePresetsRestore(priorIdentity ? priorIdentity.rolePreset : null)`
    // reintroduces: that call form always fires, so any join attempt with no
    // stored GM identity -- including one later refused -- would silently
    // stomp this operator's own live selection back to All.
    expect(SERVER_HTML).not.toMatch(
      /__hostGmRolePresetsRestore\(priorIdentity \? priorIdentity\.rolePreset : null\)/,
    );
  });

  it('merges this browser\'s current selection into every automatic identity re-emission, never stomping a restored value back to null', () => {
    expect(SERVER_HTML).toContain(
      'rememberGmIdentity(identityCode, { ...identity, rolePreset: currentGmRolePresetId() });',
    );
    expect(SERVER_HTML).toContain('rolePreset: currentGmRolePresetId(),');
  });

  it('persists a live switch only when this browser holds a reconnectable GM identity', () => {
    expect(SERVER_HTML).toMatch(
      /window\.__hostGmRolePresetChanged = function\(presetId\) \{\s*if \(!fleetHandle \|\| fleetHandle\.role !== 'gm' \|\| !gmRolePresetIdentityCode\) return;/,
    );
  });

  it('gives every panel it can hide its own [hidden] CSS override, since their own display:flex outranks the UA default', () => {
    // gm-activity-feed.js/gm-local-projection.js only ever toggle `hidden` on
    // CHILDREN of these three sections, never the section element itself, so
    // this override did not exist before this issue and toggling `hidden` on
    // the section would otherwise be a silent visual no-op (issue #1319).
    expect(SERVER_HTML).toContain(
      '#gm-map-panel[hidden], #gm-inspector[hidden], #gm-activity[hidden] { display: none; }',
    );
  });

  it('wires the pure module into the GM page and exposes its state for tests/wiring', () => {
    expect(WORKSPACE).toContain("import { createGmRolePresets } from './gm-role-presets.js'");
    expect(WORKSPACE).toContain('win.__hostGmRolePresetsSetAvailable = gmRolePresets.setAvailablePresets');
    expect(WORKSPACE).toContain('win.__hostGmRolePresetsRestore = gmRolePresets.restore');
    expect(WORKSPACE).toContain('win.__hostGmRolePresetsState = gmRolePresets.state');
    expect(SERVER_HTML).toContain('id="gm-role-preset-select"');
  });
});

describe('storedGmIdentity/rememberGmIdentity role-preset round-trip (issue #1319)', () => {
  const GM_IDENTITY_KEY = 'phoenix.fleet.gm-identity.v1';

  function compileIdentityStorageSeam(storage) {
    const startMarker = 'function storedGmIdentity(code) {';
    const endMarker = 'function publishGmRoster(roster) {';
    const start = SERVER_HTML.indexOf(startMarker);
    const end = SERVER_HTML.indexOf(endMarker, start);
    if (start < 0 || end < 0) throw new Error('GM identity storage seam not found');
    const source = SERVER_HTML.slice(start, end);
    const wrapped = `
      let fleetIdentityStorage = null;
      try { fleetIdentityStorage = window.localStorage; } catch (_) {}
      const FLEET_GM_IDENTITY_KEY = ${JSON.stringify(GM_IDENTITY_KEY)};
      ${source}
      return { storedGmIdentity, rememberGmIdentity };
    `;
    return new Function('window', wrapped)({ localStorage: storage });
  }

  function fakeStorage() {
    const data = new Map();
    return {
      getItem: (key) => (data.has(key) ? data.get(key) : null),
      setItem: (key, value) => data.set(key, value),
      raw: data,
    };
  }

  it('names the exact stored identity key this suite and the reconnect smoke spec both assume', () => {
    expect(SERVER_HTML).toContain(`const FLEET_GM_IDENTITY_KEY = '${GM_IDENTITY_KEY}';`);
  });

  it('round-trips an explicit role preset choice across a rememberGmIdentity/storedGmIdentity pair', () => {
    const storage = fakeStorage();
    const { storedGmIdentity, rememberGmIdentity } = compileIdentityStorageSeam(storage);

    rememberGmIdentity('ABCD', {
      role: 'gm',
      operatorId: 'gm-1',
      reconnectCredential: 'secret',
      rolePreset: 'tactical',
    });

    expect(storedGmIdentity('ABCD')).toEqual({
      code: 'ABCD',
      operatorId: 'gm-1',
      reconnectCredential: 'secret',
      rolePreset: 'tactical',
    });
  });

  it('stores and restores null for a GM that never opened the selector — unchanged from before this issue', () => {
    const storage = fakeStorage();
    const { storedGmIdentity, rememberGmIdentity } = compileIdentityStorageSeam(storage);

    rememberGmIdentity('ABCD', {
      role: 'gm',
      operatorId: 'gm-1',
      reconnectCredential: 'secret',
      rolePreset: null,
    });

    expect(storedGmIdentity('ABCD').rolePreset).toBeNull();
  });

  it('falls back to null for a legacy stored blob with no rolePreset field at all', () => {
    const storage = fakeStorage();
    storage.setItem(GM_IDENTITY_KEY, JSON.stringify({
      code: 'ABCD',
      operatorId: 'gm-1',
      reconnectCredential: 'secret',
    }));
    const { storedGmIdentity } = compileIdentityStorageSeam(storage);

    expect(storedGmIdentity('ABCD').rolePreset).toBeNull();
  });

  it('never persists for a fleet code the stored blob was not issued for', () => {
    const storage = fakeStorage();
    const { storedGmIdentity, rememberGmIdentity } = compileIdentityStorageSeam(storage);
    rememberGmIdentity('ABCD', {
      role: 'gm',
      operatorId: 'gm-1',
      reconnectCredential: 'secret',
      rolePreset: 'tactical',
    });

    expect(storedGmIdentity('WXYZ')).toBeNull();
  });
});
