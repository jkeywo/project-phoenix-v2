import { wireText } from './strings.js';

/** Strict page-local adapter for the one bounded GM activity feed. */

const CATEGORIES = new Set([
  'damage',
  'destruction',
  'objective',
  'trigger',
  'red_alert',
  'connection',
  'gm_action',
]);
const LINK_ROLES = new Set(['source', 'victim', 'target', 'ship']);
const VICTIM_KINDS = new Set(['ship', 'asteroid']);
const OBJECTIVE_STATES = new Set(['active', 'completed', 'failed']);
const CONNECTION_ROLES = new Set(['crew', 'spectator', 'game_master']);
const CONNECTION_STATES = new Set(['connected', 'disconnected']);
const ACTION_OUTCOMES = new Set(['applied', 'no-op', 'refused']);

function normaliseReference(value) {
  if (!value || typeof value !== 'object'
      || typeof value.entity_id !== 'string' || value.entity_id.length === 0
      || typeof value.name !== 'string') return undefined;
  return { entity_id: value.entity_id, name: value.name };
}

function normaliseLink(value) {
  if (!value || typeof value !== 'object' || !LINK_ROLES.has(value.role)) return undefined;
  const entity = normaliseReference(value.entity);
  return entity ? { role: value.role, entity } : undefined;
}

function normaliseDamage(value) {
  if (!value || typeof value !== 'object'
      || !VICTIM_KINDS.has(value.victim_kind)
      || typeof value.weapon !== 'string' || value.weapon.length === 0
      || !Number.isFinite(value.amount)
      || !Number.isFinite(value.shield_absorbed)
      || !Number.isFinite(value.hull_damage)
      || (value.system_hit !== null && typeof value.system_hit !== 'string')) return undefined;
  return {
    victim_kind: value.victim_kind,
    weapon: value.weapon,
    amount: value.amount,
    shield_absorbed: value.shield_absorbed,
    hull_damage: value.hull_damage,
    system_hit: value.system_hit,
  };
}

function normalisePublicIdentity(value) {
  if (!value || typeof value !== 'object'
      || typeof value.id !== 'string' || value.id.length === 0
      || typeof value.name !== 'string') return undefined;
  return { id: value.id, name: value.name };
}

function normaliseAction(value) {
  if (!value || typeof value !== 'object' || typeof value.type !== 'string') return undefined;
  if (value.type === 'force_start') return { type: value.type };
  if (value.type === 'set_session_paused' && typeof value.active === 'boolean') {
    return { type: value.type, active: value.active };
  }
  return undefined;
}

function normaliseOrder(value) {
  if (value === null) return null;
  if (!value || typeof value !== 'object'
      || !Number.isSafeInteger(value.sequence) || value.sequence < 0
      || !Number.isSafeInteger(value.origin) || value.origin < 0) return undefined;
  return { sequence: value.sequence, origin: value.origin };
}

function normaliseDetail(category, value) {
  if (!value || typeof value !== 'object' || value.type !== category) return undefined;
  const data = value.data;
  if (category === 'damage') {
    const damage = normaliseDamage(data);
    return damage ? { type: category, data: damage } : undefined;
  }
  if (category === 'destruction') return { type: category };
  if (category === 'objective') {
    if (!data || typeof data !== 'object'
        || typeof data.objective_id !== 'string' || data.objective_id.length === 0
        || !OBJECTIVE_STATES.has(data.status)) return undefined;
    return { type: category, data: { objective_id: data.objective_id, status: data.status } };
  }
  if (category === 'trigger') {
    if (!data || typeof data !== 'object'
        || typeof data.trigger_id !== 'string' || data.trigger_id.length === 0
        || typeof data.origin !== 'string' || data.origin.length === 0) return undefined;
    return { type: category, data: { trigger_id: data.trigger_id, origin: data.origin } };
  }
  if (category === 'red_alert') {
    if (!data || typeof data !== 'object' || typeof data.active !== 'boolean') return undefined;
    return { type: category, data: { active: data.active } };
  }
  if (category === 'connection') {
    const identity = normalisePublicIdentity(data?.identity);
    const ship = data?.ship === null ? null : normaliseReference(data?.ship);
    if (!identity || !CONNECTION_ROLES.has(data?.role)
        || !CONNECTION_STATES.has(data?.state) || ship === undefined) return undefined;
    return { type: category, data: { identity, role: data.role, state: data.state, ship } };
  }
  if (category === 'gm_action') {
    const operator = normalisePublicIdentity(data?.operator);
    const action = normaliseAction(data?.action);
    const order = normaliseOrder(data?.order);
    if (!operator || typeof data?.correlation !== 'string' || data.correlation.length === 0
        || !action || !ACTION_OUTCOMES.has(data?.outcome)
        || (data.reason !== null && typeof data.reason !== 'string')
        || order === undefined) return undefined;
    return {
      type: category,
      data: {
        operator,
        correlation: data.correlation,
        action,
        outcome: data.outcome,
        reason: data.reason,
        order,
      },
    };
  }
  return undefined;
}

function normaliseEntry(value) {
  if (!value || typeof value !== 'object'
      || !Number.isSafeInteger(value.tick) || value.tick < 0
      || !CATEGORIES.has(value.category)
      || !Array.isArray(value.ships) || !Array.isArray(value.links)) return undefined;
  const ships = value.ships.map(normaliseReference);
  const links = value.links.map(normaliseLink);
  const detail = normaliseDetail(value.category, value.detail);
  if (ships.some((ship) => !ship) || links.some((link) => !link) || !detail) return undefined;
  return { tick: value.tick, category: value.category, ships, links, detail };
}

/** Parse one absolute payload without retaining partial or recursively localised data. */
export function parseGmActivityFeed(payload) {
  let value = payload;
  if (typeof value === 'string') {
    try { value = JSON.parse(value); } catch (_) { return undefined; }
  }
  if (!value || typeof value !== 'object'
      || !Number.isSafeInteger(value.capacity) || value.capacity < 0
      || !Array.isArray(value.entries)) return undefined;
  const entries = [];
  for (const candidate of value.entries) {
    const entry = normaliseEntry(candidate);
    if (!entry) return undefined;
    entries.push(entry);
  }
  return { capacity: value.capacity, entries };
}

/**
 * Pure absolute reduction. The producer is already bounded, but this second
 * local bound makes malformed/racing page state unable to grow and preserves
 * every repeated occurrence by slicing only on position.
 */
export function reduceGmActivityFeed(_previous, payload) {
  const parsed = parseGmActivityFeed(payload);
  if (!parsed) return undefined;
  return {
    capacity: parsed.capacity,
    entries: parsed.capacity === 0 ? [] : parsed.entries.slice(-parsed.capacity),
  };
}

/** Category and semantic ship filters compose as a strict AND. */
export function filterGmActivityEntries(
  entries,
  { category = 'all', ship = 'all' } = {},
) {
  return entries.filter((entry) => (
    (category === 'all' || entry.category === category)
      && (ship === 'all' || entry.ships.some((candidate) => candidate.entity_id === ship))
  ));
}

export function createGmActivityFeed({
  doc = globalThis.document,
  t = (id) => id,
  displayText = wireText,
  containsEntity = () => false,
  selectEntity = () => false,
} = {}) {
  const region = doc && doc.getElementById('gm-activity');
  const heading = doc && doc.getElementById('gm-activity-heading');
  const categoryFilter = doc && doc.getElementById('gm-activity-category-filter');
  const shipFilter = doc && doc.getElementById('gm-activity-ship-filter');
  const clearButton = doc && doc.getElementById('gm-activity-clear-filters');
  const status = doc && doc.getElementById('gm-activity-status');
  const list = doc && doc.getElementById('gm-activity-list');
  const empty = doc && doc.getElementById('gm-activity-empty');
  let state = { capacity: 0, entries: [] };

  if (region) {
    region.setAttribute('role', 'region');
    if (heading) region.setAttribute('aria-labelledby', heading.id);
  }
  if (status) {
    status.setAttribute('role', 'status');
    status.setAttribute('aria-live', 'polite');
    status.setAttribute('aria-atomic', 'true');
  }
  if (list) {
    list.setAttribute('role', 'log');
    list.setAttribute('aria-live', 'polite');
    list.setAttribute('aria-relevant', 'additions text');
  }
  if (empty) empty.textContent = t('server.gm.activity.empty');
  if (clearButton) clearButton.textContent = t('server.gm.activity.filter.clear');
  if (categoryFilter) {
    for (const option of categoryFilter.options) {
      const suffix = option.value === 'all' ? 'filter.category_all' : `category.${option.value}`;
      option.textContent = t(`server.gm.activity.${suffix}`);
    }
  }

  function shownName(reference) {
    const authoredName = reference.name || reference.entity_id;
    return displayText(authoredName, authoredName);
  }

  function isAvailable(id) {
    try { return containsEntity(id) === true; } catch (_) { return false; }
  }

  function paintLink(button) {
    const available = isAvailable(button.dataset.entityId);
    button.disabled = !available;
    button.setAttribute('aria-disabled', available ? 'false' : 'true');
    button.title = t(available
      ? 'server.gm.activity.select_available'
      : 'server.gm.activity.select_unavailable');
  }

  function linkButton(link) {
    const button = doc.createElement('button');
    button.type = 'button';
    button.className = 'gm-activity-link';
    button.dataset.entityId = link.entity.entity_id;
    button.dataset.involvement = link.role;
    button.textContent = shownName(link.entity);
    button.setAttribute('aria-label', t('server.gm.activity.select_entity', {
      name: shownName(link.entity),
    }));
    paintLink(button);
    button.addEventListener('click', () => {
      if (!isAvailable(link.entity.entity_id)) {
        paintLink(button);
        return;
      }
      let selected = false;
      try { selected = selectEntity(link.entity.entity_id) === true; } catch (_) { selected = false; }
      if (!selected) paintLink(button);
    });
    return button;
  }

  function rebuildShipFilter() {
    if (!shipFilter) return;
    const wanted = shipFilter.value || 'all';
    const ships = new Map();
    for (const entry of state.entries) {
      for (const ship of entry.ships) {
        if (isAvailable(ship.entity_id)) ships.set(ship.entity_id, ship);
      }
    }
    shipFilter.replaceChildren();
    const all = doc.createElement('option');
    all.value = 'all';
    all.textContent = t('server.gm.activity.filter.ship_all');
    shipFilter.appendChild(all);
    for (const ship of [...ships.values()]
      .sort((left, right) => left.entity_id.localeCompare(right.entity_id))) {
      const option = doc.createElement('option');
      option.value = ship.entity_id;
      option.textContent = shownName(ship);
      shipFilter.appendChild(option);
    }
    shipFilter.value = ships.has(wanted) ? wanted : 'all';
  }

  function appendLinks(row, entry) {
    if (entry.links.length === 0) return;
    const involved = doc.createElement('div');
    involved.className = 'gm-activity-involved';
    for (const link of entry.links) {
      const role = doc.createElement('span');
      role.className = 'gm-activity-link-role';
      role.textContent = t(`server.gm.activity.link_role.${link.role}`);
      involved.append(role, linkButton(link));
    }
    row.appendChild(involved);
  }

  function detailText(entry) {
    const detail = entry.detail.data;
    switch (entry.category) {
      case 'damage':
        return t('server.gm.activity.damage_detail', {
          amount: String(detail.amount),
          shield: String(detail.shield_absorbed),
          hull: String(detail.hull_damage),
          weapon: detail.weapon,
          kind: t(`server.gm.activity.victim_kind.${detail.victim_kind}`),
        });
      case 'destruction':
        return t('server.gm.activity.destruction_detail');
      case 'objective':
        return t('server.gm.activity.objective_detail', {
          objective: displayText(detail.objective_id, detail.objective_id),
          status: t(`server.gm.activity.objective_status.${detail.status}`),
        });
      case 'trigger':
        return t('server.gm.activity.trigger_detail', {
          trigger: detail.trigger_id,
          origin: detail.origin,
        });
      case 'red_alert':
        return t(`server.gm.activity.red_alert.${detail.active ? 'active' : 'inactive'}`);
      case 'connection':
        return t('server.gm.activity.connection_detail', {
          name: detail.identity.name || detail.identity.id,
          role: t(`server.gm.activity.connection_role.${detail.role}`),
          state: t(`server.gm.activity.connection_state.${detail.state}`),
        });
      case 'gm_action': {
        const action = detail.action.type === 'force_start'
          ? t('server.gm.activity.action.force_start')
          : t(`server.gm.activity.action.${detail.action.active ? 'pause' : 'resume'}`);
        return t('server.gm.activity.gm_action_detail', {
          operator: detail.operator.name || detail.operator.id,
          action,
          outcome: t(`server.gm.activity.action_outcome.${detail.outcome}`),
          correlation: detail.correlation,
        });
      }
      default:
        return '';
    }
  }

  function appendEntry(entry) {
    const row = doc.createElement('li');
    row.className = 'gm-activity-entry';
    row.dataset.tick = String(entry.tick);
    row.dataset.category = entry.category;

    const metadata = doc.createElement('div');
    metadata.className = 'gm-activity-metadata';
    const tick = doc.createElement('span');
    tick.className = 'gm-activity-tick';
    tick.textContent = t('server.gm.activity.tick', { tick: String(entry.tick) });
    const category = doc.createElement('span');
    category.className = 'gm-activity-category';
    category.textContent = t(`server.gm.activity.category.${entry.category}`);
    metadata.append(tick, category);
    row.appendChild(metadata);

    appendLinks(row, entry);

    const detail = doc.createElement('div');
    detail.className = 'gm-activity-detail';
    detail.textContent = detailText(entry);
    row.appendChild(detail);
    if (entry.category === 'damage' && entry.detail.data.system_hit) {
      const system = doc.createElement('div');
      system.className = 'gm-activity-system';
      system.textContent = t('server.gm.activity.system_hit', {
        system: entry.detail.data.system_hit,
      });
      row.appendChild(system);
    }
    if (entry.category === 'gm_action' && entry.detail.data.reason) {
      const reason = doc.createElement('div');
      reason.className = 'gm-activity-reason';
      reason.textContent = t('server.gm.activity.action_reason', {
        reason: t(`server.gm.activity.action_reason.${entry.detail.data.reason}`),
      });
      row.appendChild(reason);
    }
    list.appendChild(row);
  }

  function render({ rebuildShips = true } = {}) {
    if (rebuildShips) rebuildShipFilter();
    const filtered = filterGmActivityEntries(state.entries, {
      category: categoryFilter?.value || 'all',
      ship: shipFilter?.value || 'all',
    });
    if (list) {
      list.replaceChildren();
      for (const entry of filtered) appendEntry(entry);
    }
    if (empty) empty.hidden = filtered.length !== 0;
    if (status) {
      status.textContent = t('server.gm.activity.status', {
        shown: String(filtered.length),
        total: String(state.entries.length),
        capacity: String(state.capacity),
      });
    }
  }

  function update(payload) {
    const next = reduceGmActivityFeed(state, payload);
    if (!next) return false;
    state = next;
    render();
    return true;
  }

  function clearFilters() {
    if (categoryFilter) categoryFilter.value = 'all';
    if (shipFilter) shipFilter.value = 'all';
    render();
  }

  function reconcileAvailability() {
    rebuildShipFilter();
    if (list) {
      for (const button of list.querySelectorAll('.gm-activity-link')) paintLink(button);
    }
    render({ rebuildShips: false });
  }

  const onFilter = () => render({ rebuildShips: false });
  categoryFilter?.addEventListener('change', onFilter);
  shipFilter?.addEventListener('change', onFilter);
  clearButton?.addEventListener('click', clearFilters);
  render();

  return {
    update,
    clearFilters,
    reconcileAvailability,
    state: () => ({ capacity: state.capacity, entries: [...state.entries] }),
    destroy: () => {
      categoryFilter?.removeEventListener('change', onFilter);
      shipFilter?.removeEventListener('change', onFilter);
      clearButton?.removeEventListener('click', clearFilters);
    },
  };
}
