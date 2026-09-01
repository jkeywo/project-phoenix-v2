/** Peer-local one-entity renderer for the explicit browser GM profile (#1291). */

export function parseGmEntityProjection(payload) {
  let value = payload;
  if (typeof value === 'string') {
    try { value = JSON.parse(value); } catch (_) { return undefined; }
  }
  if (!value || typeof value !== 'object' || !Object.hasOwn(value, 'entity')) return undefined;
  if (value.entity === null) return null;
  const entity = value.entity;
  const status = entity && entity.status;
  if (!entity || typeof entity !== 'object'
      || typeof entity.entity_id !== 'string' || entity.entity_id.length === 0
      || typeof entity.name !== 'string'
      || !status || typeof status !== 'object'
      || !Number.isInteger(status.hull_percent)
      || status.hull_percent < 0 || status.hull_percent > 100
      || typeof status.destroyed !== 'boolean') return undefined;
  return entity;
}

export function createGmLocalProjection({ doc = document, t = (id) => id } = {}) {
  const pending = doc.getElementById('gm-entity-pending');
  const card = doc.getElementById('gm-entity-card');
  const name = doc.getElementById('gm-entity-name');
  const identity = doc.getElementById('gm-entity-identity');
  const statusText = doc.getElementById('gm-entity-status');
  const hull = doc.getElementById('gm-entity-hull');

  function clear() {
    if (!card || !pending) return false;
    card.hidden = true;
    pending.hidden = false;
    delete card.dataset.entityId;
    delete card.dataset.destroyed;
    if (name) name.textContent = '';
    if (identity) identity.textContent = '';
    if (statusText) statusText.textContent = '';
    if (hull) hull.value = 0;
    return true;
  }

  function update(payload) {
    const entity = parseGmEntityProjection(payload);
    if (entity === undefined) return false;
    if (entity === null) return clear();
    if (!card || !pending || !name || !identity || !statusText || !hull) return false;
    card.dataset.entityId = entity.entity_id;
    card.dataset.destroyed = String(entity.status.destroyed);
    name.textContent = entity.name;
    identity.textContent = entity.entity_id;
    hull.value = entity.status.hull_percent;
    statusText.textContent = entity.status.destroyed
      ? t('server.gm.entity.destroyed')
      : t('server.gm.entity.hull', { percent: entity.status.hull_percent });
    pending.hidden = true;
    card.hidden = false;
    return true;
  }

  return { update, clear };
}
