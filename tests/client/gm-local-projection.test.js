// @vitest-environment jsdom

import { beforeEach, describe, expect, it } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  createGmLocalProjection,
  parseGmEntityProjection,
} from '../../gui/gm-local-projection.js';

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), '../..');
const read = (file) => fs.readFileSync(path.join(root, file), 'utf8');

function payload(overrides = {}) {
  return JSON.stringify({
    entity: {
      entity_id: '00000000-0000-0000-0000-000000000001',
      name: 'Axiom Station',
      status: { hull_percent: 73, destroyed: false },
      ...overrides,
    },
  });
}

describe('rendererless GM local projection', () => {
  let projection;

  beforeEach(() => {
    document.body.innerHTML = `
      <p id="gm-entity-pending"></p>
      <article id="gm-entity-card" hidden>
        <h2 id="gm-entity-name"></h2>
        <span id="gm-entity-identity"></span>
        <span id="gm-entity-status"></span>
        <meter id="gm-entity-hull" min="0" max="100" value="0"></meter>
      </article>`;
    projection = createGmLocalProjection({
      doc: document,
      t: (id, params = {}) => `${id}:${params.percent ?? ''}`,
    });
  });

  it('renders stable authoritative identity and status updates', () => {
    expect(projection.update(payload())).toBe(true);
    const card = document.getElementById('gm-entity-card');
    expect(card.hidden).toBe(false);
    expect(card.dataset.entityId).toBe('00000000-0000-0000-0000-000000000001');
    expect(document.getElementById('gm-entity-name').textContent).toBe('Axiom Station');
    expect(document.getElementById('gm-entity-hull').value).toBe(73);

    projection.update(payload({ status: { hull_percent: 41, destroyed: false } }));
    expect(card.dataset.entityId).toBe('00000000-0000-0000-0000-000000000001');
    expect(document.getElementById('gm-entity-hull').value).toBe(41);
    expect(document.getElementById('gm-entity-status').textContent)
      .toBe('server.gm.entity.hull:41');
  });

  it('explicit null clears a removed entity so stale state cannot survive', () => {
    projection.update(payload());
    expect(projection.update('{"entity":null}')).toBe(true);
    const card = document.getElementById('gm-entity-card');
    expect(card.hidden).toBe(true);
    expect(card.dataset.entityId).toBeUndefined();
    expect(document.getElementById('gm-entity-identity').textContent).toBe('');
    expect(document.getElementById('gm-entity-pending').hidden).toBe(false);
  });

  it('rejects malformed payloads without changing the current entity', () => {
    projection.update(payload());
    expect(projection.update('{bad')).toBe(false);
    expect(parseGmEntityProjection({ entity: { entity_id: 'x' } })).toBeUndefined();
    expect(document.getElementById('gm-entity-card').dataset.entityId)
      .toBe('00000000-0000-0000-0000-000000000001');
  });
});

describe('GM projection transport separation', () => {
  it('exists only in the Host Channel, never in peer or lockstep vocabularies', () => {
    for (const file of [
      'src/core/messages.rs',
      'src/lockstep/frame.rs',
      'src/server_app/components.rs',
    ]) {
      const source = read(file);
      expect(source).not.toContain('GmEntityProjection');
      expect(source).not.toContain('gm_entity');
    }

    const bridge = read('src/server/bridge.rs');
    const outbound = bridge.slice(
      bridge.indexOf('fn flush_outbound('),
      bridge.indexOf('fn flush_host_channels('),
    );
    expect(outbound).not.toContain('GmEntityProjection');
    expect(outbound).not.toContain('GM_ENTITY');
    expect(outbound).not.toContain('gm_entity');
    expect(bridge).toContain('host_channels::GM_ENTITY');
    expect(bridge).toContain('HOST_CHANNEL_CB');
  });
});
