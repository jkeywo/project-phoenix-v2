import { wireText } from './strings.js';

/**
 * Minimal reusable entity inspector shell (issue #1295).
 *
 * The shell knows only the intentionally narrow GM/M6 projection DTO. It has
 * no ECS vocabulary, no command adapter and no authority of its own. Later M6
 * panels can add sections around this identity/status/target spine without
 * replacing the selection contract.
 */

export function createEntityInspector({
  doc = document,
  t = (id) => id,
  displayText = wireText,
  onSelect = () => false,
} = {}) {
  const empty = doc.getElementById('gm-inspector-empty');
  const card = doc.getElementById('gm-entity-card');
  const name = doc.getElementById('gm-entity-name');
  const identity = doc.getElementById('gm-entity-identity');
  const kind = doc.getElementById('gm-entity-kind');
  const position = doc.getElementById('gm-entity-position');
  const faction = doc.getElementById('gm-entity-faction');
  const status = doc.getElementById('gm-entity-status');
  const hull = doc.getElementById('gm-entity-hull');
  const target = doc.getElementById('gm-entity-target');
  let current = null;

  if (target) {
    target.addEventListener('click', () => {
      const targetId = current && current.current_target && current.current_target.entity_id;
      if (targetId) onSelect(targetId);
    });
  }

  function clear() {
    current = null;
    if (!card || !empty) return false;
    card.hidden = true;
    empty.hidden = false;
    delete card.dataset.entityId;
    delete card.dataset.kind;
    delete card.dataset.destroyed;
    for (const field of [name, identity, kind, position, faction, status, target]) {
      if (field) field.textContent = '';
    }
    if (target) {
      target.disabled = true;
      delete target.dataset.targetId;
    }
    if (hull) {
      hull.value = 0;
      hull.hidden = true;
      hull.removeAttribute('aria-valuetext');
    }
    return true;
  }

  function render(entity) {
    if (!entity) return clear();
    if (!card || !empty || !name || !identity || !kind || !position
        || !faction || !status || !hull || !target) return false;
    current = entity;
    card.dataset.entityId = entity.entity_id;
    card.dataset.kind = entity.kind;
    card.dataset.destroyed = String(entity.status.destroyed);
    name.textContent = displayText(entity.name);
    identity.textContent = entity.entity_id;
    kind.textContent = t(`server.gm.entity.kind.${entity.kind}`);
    position.textContent = t('server.gm.entity.position_value', {
      x: entity.position[0].toFixed(1),
      y: entity.position[1].toFixed(1),
      z: entity.position[2].toFixed(1),
    });
    faction.textContent = entity.faction
      ? displayText(entity.faction.name)
      : t('server.gm.entity.faction_none');
    const hullPercent = entity.status.hull_percent;
    const conditionPercent = entity.status.condition_percent;
    if (entity.status.destroyed) {
      status.textContent = t('server.gm.entity.destroyed');
    } else if (hullPercent !== null && conditionPercent !== null) {
      status.textContent = t('server.gm.entity.hull_and_condition', {
        hull: hullPercent,
        condition: conditionPercent,
      });
    } else if (hullPercent !== null) {
      status.textContent = t('server.gm.entity.hull', { percent: hullPercent });
    } else if (conditionPercent !== null) {
      status.textContent = t('server.gm.entity.condition', { percent: conditionPercent });
    } else {
      status.textContent = t('server.gm.entity.active');
    }
    hull.hidden = hullPercent === null;
    hull.value = hullPercent === null ? 0 : hullPercent;
    if (hullPercent === null) hull.removeAttribute('aria-valuetext');
    else hull.setAttribute('aria-valuetext', status.textContent);
    if (entity.current_target) {
      target.textContent = displayText(entity.current_target.name);
      target.dataset.targetId = entity.current_target.entity_id;
      target.disabled = false;
    } else {
      target.textContent = t('server.gm.entity.target_none');
      delete target.dataset.targetId;
      target.disabled = true;
    }
    empty.hidden = true;
    card.hidden = false;
    return true;
  }

  return {
    clear,
    render,
    current: () => current,
  };
}
