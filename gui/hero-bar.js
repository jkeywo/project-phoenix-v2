/** Shared complete-Station Hero Bar model (issue #1097). */

function legacyPlacement(station, stationSystems, blackboards) {
  const boards = (stationSystems?.[station.id] || []).map(id => blackboards?.[id]);
  const board = boards.find(value => value?.host_station != null) || boards.find(Boolean);
  return board ? {
    station: station.id,
    host: board.host_station || null,
    rating: board.station_rating || station.visiting_rating || '',
  } : null;
}

/**
 * Direct Station first, then visiting Stations in hull-authored order.
 * AI assignments are returned in `ownership` even though they are not tabs,
 * allowing the shell to state the outcome without encoding it as colour.
 */
export function heroBarModel({ directStation, stations, stationSystems,
  blackboards, stationHosts, stationHealth, stationRatings, activeStation }) {
  const defs = stations || [];
  const byId = Object.fromEntries(defs.filter(Boolean).map(st => [st.id, st]));
  const placements = Object.fromEntries(defs.filter(st => st?.human_seeking).map(st => [
    st.id,
    stationHosts?.[st.id] || legacyPlacement(st, stationSystems, blackboards),
  ]));
  const visiting = directStation ? defs
    .filter(st => st?.human_seeking && st.id !== directStation)
    .filter(st => placements[st.id]?.host === directStation) : [];
  const tabIds = directStation ? [directStation, ...visiting.map(st => st.id)] : [];
  const selected = tabIds.includes(activeStation) ? activeStation : (tabIds[0] || null);
  const ownership = {};
  for (const st of defs.filter(st => st?.human_seeking)) {
    const host = placements[st.id]?.host || null;
    ownership[st.id] = host === st.id ? 'direct' : (host ? 'visiting' : 'ai');
  }
  if (directStation) ownership[directStation] = 'direct';
  return {
    selected,
    tabs: tabIds.map(id => {
      const st = byId[id] || { id, name: id };
      const mode = id === directStation ? 'direct' : 'visiting';
      const rating = mode === 'visiting'
        ? (placements[id]?.rating || st.visiting_rating || stationRatings?.[id] || '')
        : (stationRatings?.[id] || '');
      // Authoritative host figure (issue #1100). A number is the summed hull
      // fraction; `null` — explicit no-damage-model, or simply absent because
      // the Station owns no damageable capacity — is the neutral state. Never
      // derived from blackboards/stationSystems: AC #3 forbids inferring a
      // Station's health from the recipient-scoped damage rows a client holds.
      const rawHealth = stationHealth ? stationHealth[id] : undefined;
      const health = typeof rawHealth === 'number' ? rawHealth : null;
      return { id, name: st.name || id, mode, rating, health,
        healthState: heroBarHealthState(health), selected: id === selected };
    }),
    ownership,
    aiStations: defs
      .filter(st => st?.human_seeking && ownership[st.id] === 'ai')
      .map(st => ({ id: st.id, name: st.name || st.id })),
  };
}

/**
 * Classify a Station's authoritative health fraction into a discrete state,
 * used to pick a persistent non-colour cue per tab (issue #1100).
 *
 * Three states, none of which needs a tunable threshold — a threshold would be
 * a hardcoded gameplay value (AGENTS.md rule 11):
 *   - `none`    — `null`/absent: the neutral no-damage-model state.
 *   - `healthy` — full hull (fraction at or above 1): no damage at all.
 *   - `damaged` — any hull loss (fraction below 1).
 */
export function heroBarHealthState(health) {
  if (typeof health !== 'number') return 'none';
  return health >= 1 ? 'healthy' : 'damaged';
}

/** Roving-tab keyboard rule used by the DOM shell and unit tests. */
export function heroBarKeyTarget(ids, current, key) {
  if (!ids?.length) return null;
  const index = Math.max(0, ids.indexOf(current));
  if (key === 'Home') return ids[0];
  if (key === 'End') return ids[ids.length - 1];
  if (key === 'ArrowRight') return ids[(index + 1) % ids.length];
  if (key === 'ArrowLeft') return ids[(index - 1 + ids.length) % ids.length];
  return null;
}

/**
 * Reconcile the Hero Bar without replacing unchanged tab buttons. Simulation
 * snapshots render the shell frequently, so preserving button identity is what
 * keeps keyboard focus stable while unrelated blackboard values change.
 */
export function renderHeroBarDom({ tabsEl, titleEl, metaEl, aiEl, healthEl, model,
  translate, onActivate }) {
  const existing = new Map(
    [...tabsEl.querySelectorAll('button[data-station]')]
      .map(button => [button.dataset.station, button]),
  );
  const ids = model.tabs.map(tab => tab.id);
  const desired = new Set(ids);
  for (const [id, button] of existing) {
    if (!desired.has(id)) {
      button.remove();
      existing.delete(id);
    }
  }

  for (const [index, tab] of model.tabs.entries()) {
    let button = existing.get(tab.id);
    if (!button) {
      button = tabsEl.ownerDocument.createElement('button');
      button.type = 'button';
      button.role = 'tab';
      button.dataset.station = tab.id;
      button.append(
        tabsEl.ownerDocument.createElement('span'),
        tabsEl.ownerDocument.createElement('span'),
        tabsEl.ownerDocument.createElement('span'),
      );
      button.children[1].className = 'station-tab-owner';
      // A dedicated span for the persistent non-colour health cue (issue #1100).
      // Being its own element is what lets it coexist with an importance alert
      // painted elsewhere on the tab, and stay a text/shape token, not colour.
      button.children[2].className = 'station-tab-health';
    }
    existing.delete(tab.id);
    button.setAttribute('aria-selected', tab.id === model.selected ? 'true' : 'false');
    button.tabIndex = tab.id === model.selected ? 0 : -1;
    button.children[0].textContent = tab.name;
    button.children[1].textContent = translate('client.hero.owner.' + tab.mode);
    // Persistent per-tab health cue on EVERY tab (AC #3/#4): a text/shape token,
    // never colour, and set unconditionally so an alert cannot suppress it.
    button.children[2].textContent = translate('client.hero.health.cue.' + tab.healthState);
    button.dataset.health = tab.healthState;
    button.onclick = () => onActivate(tab.id);
    button.onkeydown = event => {
      const target = heroBarKeyTarget(ids, tab.id, event.key);
      if (!target) return;
      event.preventDefault();
      onActivate(target);
      [...tabsEl.querySelectorAll('button[data-station]')]
        .find(candidate => candidate.dataset.station === target)?.focus();
    };
    // Moving an already-correct child through appendChild can itself blur it.
    // Only touch tree position when the authored tab order actually changed.
    const childAtIndex = tabsEl.children[index];
    if (childAtIndex !== button) tabsEl.insertBefore(button, childAtIndex || null);
  }
  const selected = model.tabs.find(tab => tab.id === model.selected) || model.tabs[0];
  titleEl.textContent = selected.name;
  metaEl.textContent = translate('client.hero.meta', {
    owner: translate('client.hero.owner.' + selected.mode), rating: selected.rating,
  });
  const aiNames = model.aiStations.map(station => station.name).join(', ');
  aiEl.hidden = !aiNames;
  aiEl.textContent = aiNames ? translate('client.hero.ai_status', { stations: aiNames }) : '';
  // Selected-Station health readout (issue #1100), from the authoritative host
  // figure. `none`/absent renders the neutral no-damage-model label; otherwise
  // the summed hull percentage. Carries the same discrete state as a data
  // attribute so it, too, is legible without relying on colour.
  if (healthEl) {
    healthEl.dataset.health = selected.healthState;
    healthEl.textContent = selected.healthState === 'none'
      ? translate('client.hero.health.none')
      : translate('client.hero.health.readout', { pct: Math.round(selected.health * 100) });
  }
}

if (typeof window !== 'undefined') {
  window.heroBarModel = heroBarModel;
  window.heroBarKeyTarget = heroBarKeyTarget;
  window.renderHeroBarDom = renderHeroBarDom;
}
