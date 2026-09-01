import { wireText } from './strings.js';

/** Strict page-local adapter for the bounded GM damage/destruction feed. */

const CATEGORIES = new Set(['damage', 'destruction']);
const VICTIM_KINDS = new Set(['ship', 'asteroid']);

function normaliseReference(value) {
  if (!value || typeof value !== 'object'
      || typeof value.entity_id !== 'string' || value.entity_id.length === 0
      || typeof value.name !== 'string') return undefined;
  return { entity_id: value.entity_id, name: value.name };
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

function normaliseEntry(value) {
  if (!value || typeof value !== 'object'
      || !Number.isSafeInteger(value.tick) || value.tick < 0
      || !CATEGORIES.has(value.category)) return undefined;
  const victim = normaliseReference(value.victim);
  const source = value.source === null ? null : normaliseReference(value.source);
  const damage = value.damage === null ? null : normaliseDamage(value.damage);
  if (!victim || source === undefined || damage === undefined
      || (value.category === 'damage') !== (damage !== null)) return undefined;
  return { tick: value.tick, category: value.category, victim, source, damage };
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

/** Category plus exact involved-identity filtering, oldest first. */
export function filterGmActivityEntries(
  entries,
  { category = 'all', identity = 'all' } = {},
) {
  return entries.filter((entry) => (
    (category === 'all' || entry.category === category)
      && (identity === 'all'
        || entry.victim.entity_id === identity
        || entry.source?.entity_id === identity)
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
  const identityFilter = doc && doc.getElementById('gm-activity-identity-filter');
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

  function paintIdentityButton(button) {
    const available = isAvailable(button.dataset.entityId);
    button.disabled = !available;
    button.setAttribute('aria-disabled', available ? 'false' : 'true');
    button.title = t(available
      ? 'server.gm.activity.select_available'
      : 'server.gm.activity.select_unavailable');
  }

  function identityButton(reference, role) {
    const button = doc.createElement('button');
    button.type = 'button';
    button.className = 'gm-activity-identity';
    button.dataset.entityId = reference.entity_id;
    button.dataset.involvement = role;
    button.textContent = shownName(reference);
    button.setAttribute('aria-label', t('server.gm.activity.select_entity', {
      name: shownName(reference),
    }));
    paintIdentityButton(button);
    button.addEventListener('click', () => {
      // Re-check at activation: a gm_entity replacement may have raced this
      // paint. A failed selection is a no-op and cannot clear another target.
      if (!isAvailable(reference.entity_id)) {
        paintIdentityButton(button);
        return;
      }
      let selected = false;
      try { selected = selectEntity(reference.entity_id) === true; } catch (_) { selected = false; }
      if (!selected) paintIdentityButton(button);
    });
    return button;
  }

  function rebuildIdentityFilter() {
    if (!identityFilter) return;
    const wanted = identityFilter.value || 'all';
    const identities = new Map();
    for (const entry of state.entries) {
      identities.set(entry.victim.entity_id, entry.victim);
      if (entry.source) identities.set(entry.source.entity_id, entry.source);
    }
    identityFilter.replaceChildren();
    const all = doc.createElement('option');
    all.value = 'all';
    all.textContent = t('server.gm.activity.filter.identity_all');
    identityFilter.appendChild(all);
    for (const reference of [...identities.values()]
      .sort((left, right) => left.entity_id.localeCompare(right.entity_id))) {
      const option = doc.createElement('option');
      option.value = reference.entity_id;
      option.textContent = shownName(reference);
      identityFilter.appendChild(option);
    }
    identityFilter.value = identities.has(wanted) ? wanted : 'all';
  }

  function appendEntry(entry) {
    const row = doc.createElement('li');
    row.className = 'gm-activity-entry';
    row.dataset.tick = String(entry.tick);
    row.dataset.category = entry.category;
    row.dataset.victimId = entry.victim.entity_id;
    if (entry.source) row.dataset.sourceId = entry.source.entity_id;

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

    const involved = doc.createElement('div');
    involved.className = 'gm-activity-involved';
    if (entry.source) involved.appendChild(identityButton(entry.source, 'source'));
    else {
      const environment = doc.createElement('span');
      environment.className = 'gm-activity-environment';
      environment.textContent = t('server.gm.activity.environment');
      involved.appendChild(environment);
    }
    const direction = doc.createElement('span');
    direction.className = 'gm-activity-direction';
    direction.textContent = t('server.gm.activity.direction');
    involved.append(direction, identityButton(entry.victim, 'victim'));
    row.appendChild(involved);

    if (entry.damage) {
      const detail = doc.createElement('div');
      detail.className = 'gm-activity-detail';
      detail.textContent = t('server.gm.activity.damage_detail', {
        amount: String(entry.damage.amount),
        shield: String(entry.damage.shield_absorbed),
        hull: String(entry.damage.hull_damage),
        weapon: entry.damage.weapon,
        kind: t(`server.gm.activity.victim_kind.${entry.damage.victim_kind}`),
      });
      row.appendChild(detail);
      if (entry.damage.system_hit) {
        const system = doc.createElement('div');
        system.className = 'gm-activity-system';
        system.textContent = t('server.gm.activity.system_hit', {
          system: entry.damage.system_hit,
        });
        row.appendChild(system);
      }
    }
    list.appendChild(row);
  }

  function render({ rebuildIdentities = true } = {}) {
    if (rebuildIdentities) rebuildIdentityFilter();
    const filtered = filterGmActivityEntries(state.entries, {
      category: categoryFilter?.value || 'all',
      identity: identityFilter?.value || 'all',
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

  function reconcileAvailability() {
    if (!list) return;
    for (const button of list.querySelectorAll('.gm-activity-identity')) {
      paintIdentityButton(button);
    }
  }

  const onFilter = () => render({ rebuildIdentities: false });
  categoryFilter?.addEventListener('change', onFilter);
  identityFilter?.addEventListener('change', onFilter);
  render();

  return {
    update,
    reconcileAvailability,
    state: () => ({ capacity: state.capacity, entries: [...state.entries] }),
    destroy: () => {
      categoryFilter?.removeEventListener('change', onFilter);
      identityFilter?.removeEventListener('change', onFilter);
    },
  };
}
