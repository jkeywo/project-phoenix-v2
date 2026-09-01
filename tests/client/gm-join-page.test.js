import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const SERVER_HTML = readFileSync(path.join(root, 'server.html'), 'utf8');

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
    expect(SERVER_HTML).toContain("progress.status === 'committed' || progress.status === 'refused'");
    expect(SERVER_HTML).toContain('fleetHandle.completeGmJoin(id, progress.status');
  });

  it('wires the same request and status callbacks on owner and member handles', () => {
    expect(SERVER_HTML.match(/onGmJoinRequest: receiveFleetGmJoinRequest/g)).toHaveLength(2);
    expect(SERVER_HTML.match(/onGmJoinStatus: receiveFleetGmJoinStatus/g)).toHaveLength(2);
  });
});
