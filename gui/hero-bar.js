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
 * Direct Station first, then the active console's own overlay tabs, then
 * visiting Stations in hull-authored order.
 * AI assignments are returned in `ownership` even though they are not tabs,
 * allowing the shell to state the outcome without encoding it as colour.
 *
 * `consoleTabs` (issue #1373) are the overlay panels the ACTIVE console
 * declared — Intel and Security on the destroyer's Tactical seat. They are
 * tabs on the same bar because they are the same choice the player is making
 * ("what am I looking at"), but they are not Stations: they carry no health
 * (`health: null`, so the strip renders the neutral no-damage-model state),
 * no rating and no ownership, and they never appear in `ownership` or
 * `aiStations`. They sit between the direct Station and its visitors because
 * they belong to the direct Station's own console, so the bar reads
 * "this seat, this seat's other surfaces, then who else is riding here".
 *
 * `activeOverlay` names the one currently open. An open overlay is the bar's
 * `selected` tab: exactly one thing on a tablist can be selected, and what the
 * player is looking at is the panel, not the console behind it.
 */
export function heroBarModel({ directStation, stations, stationSystems,
  blackboards, stationHosts, stationHealth, stationImportance, stationRatings, activeStation,
  consoleTabs, activeOverlay }) {
  const defs = stations || [];
  const byId = Object.fromEntries(defs.filter(Boolean).map(st => [st.id, st]));
  const placements = Object.fromEntries(defs.filter(st => st?.human_seeking).map(st => [
    st.id,
    stationHosts?.[st.id] || legacyPlacement(st, stationSystems, blackboards),
  ]));
  const visiting = directStation ? defs
    .filter(st => st?.human_seeking && st.id !== directStation)
    .filter(st => placements[st.id]?.host === directStation) : [];
  // An overlay tab without a direct Station has no console to open it in.
  const overlays = directStation
    ? (Array.isArray(consoleTabs) ? consoleTabs : []).filter(tab => tab && tab.id)
    : [];
  const overlayIds = overlays.map(tab => tab.id);
  const stationIds = directStation ? [directStation, ...visiting.map(st => st.id)] : [];
  const tabIds = directStation
    ? [directStation, ...overlayIds, ...visiting.map(st => st.id)]
    : [];
  const openOverlay = overlayIds.includes(activeOverlay) ? activeOverlay : null;
  const selected = openOverlay
    || (stationIds.includes(activeStation) ? activeStation : (stationIds[0] || null));
  const ownership = {};
  for (const st of defs.filter(st => st?.human_seeking)) {
    const host = placements[st.id]?.host || null;
    ownership[st.id] = host === st.id ? 'direct' : (host ? 'visiting' : 'ai');
  }
  if (directStation) ownership[directStation] = 'direct';
  const overlayById = Object.fromEntries(overlays.map(tab => [tab.id, tab]));
  return {
    selected,
    tabs: tabIds.map(id => {
      // An overlay tab is built here rather than folded into the Station
      // branch below: every field that branch derives — rating, health,
      // importance, ownership — is a fact about a Station, and inventing a
      // neutral value for each one inside that code would read as though an
      // overlay were a Station with nothing wrong with it.
      const overlay = overlayById[id];
      if (overlay) {
        return {
          id,
          kind: 'overlay',
          name: overlay.name || id,
          code: typeof overlay.code === 'string' ? overlay.code : '',
          rating: '',
          health: null,
          healthState: heroBarHealthState(null),
          importance: null,
          importanceState: heroBarImportanceState(null),
          // How many things the surface has that the seat has not seen. Zero
          // (or absent) draws nothing at all — a badge reading "0" is a badge
          // that never goes away.
          badge: Number.isFinite(overlay.badge) ? Math.max(0, Math.trunc(overlay.badge)) : 0,
          selected: id === selected,
        };
      }
      const st = byId[id] || { id, name: id };
      const rating = id !== directStation
        ? (placements[id]?.rating || st.visiting_rating || stationRatings?.[id] || '')
        : (stationRatings?.[id] || '');
      // Authoritative host figure (issue #1100). A number is the summed hull
      // fraction; `null` — explicit no-damage-model, or simply absent because
      // the Station owns no damageable capacity — is the neutral state. Never
      // derived from blackboards/stationSystems: AC #3 forbids inferring a
      // Station's health from the recipient-scoped damage rows a client holds.
      const rawHealth = stationHealth ? stationHealth[id] : undefined;
      const health = typeof rawHealth === 'number' ? rawHealth : null;
      // Authoritative host importance figure (issue #1101), kept SEPARATE from
      // health: a one-off `unread` event and a continuing `critical` condition,
      // each with its own lifecycle. Absent (Station resolved / never marked) is
      // the neutral state. Never derived from health or blackboards.
      const importance = (stationImportance && stationImportance[id]) || null;
      // The hull's authored abbreviation (`[[station]] short_code`), already on
      // the wire inside `shipStations.stations`. Carried verbatim — never
      // derived by truncating the name, which would invent a label the hull did
      // not author and would differ between hulls that share a Station id.
      // Empty when the hull authored none; `renderHeroBarDom` falls back to the
      // full name rather than showing a blank tab.
      const code = typeof st.short_code === 'string' ? st.short_code : '';
      return { id, kind: 'station', name: st.name || id, code, rating, health,
        healthState: heroBarHealthState(health), importance,
        importanceState: heroBarImportanceState(importance), badge: 0,
        selected: id === selected };
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

/**
 * Classify a Station's authoritative importance into a discrete state for a
 * persistent non-colour cue per tab (issue #1101). Mirrors
 * `heroBarHealthState`, but for the SEPARATE importance stream so the two never
 * share a data attribute or a glyph.
 *
 * Four states, none needing a tunable threshold — the flags are already
 * booleans decided authoritatively on the host:
 *   - `none`     — no importance (neutral/resolved).
 *   - `unread`   — a one-off off-screen event, awaiting a visit.
 *   - `critical` — a continuing condition.
 *   - `both`     — a one-off event AND a continuing condition at once.
 * The two lifecycles are independent, so `both` is a real, distinct state, not
 * a precedence collapse.
 */
export function heroBarImportanceState(importance) {
  const unread = !!(importance && importance.unread);
  const critical = !!(importance && importance.critical);
  if (unread && critical) return 'both';
  if (critical) return 'critical';
  if (unread) return 'unread';
  return 'none';
}

/**
 * The one media query that decides whether the bar has room for full Station
 * names, kept HERE so the JS read and the stylesheet's own rules cannot drift:
 * `client.html` writes the same two conditions and this string is what its
 * `matchMedia` listener watches.
 *
 * Two conditions, because the bar has two shapes:
 *   - portrait, where the bar is a horizontal strip as wide as the viewport and
 *     a phone (<600px) cannot fit six full names without scrolling;
 *   - landscape, where the bar is a left rail and a phone held sideways
 *     (<=500px tall — the same threshold client.html already uses to drop the
 *     lobby's Station descriptions) gets the narrow 96px rail.
 * A tablet or desktop matches neither and shows names.
 */
export const HERO_BAR_CODE_QUERY =
  '(orientation: portrait) and (max-width: 599px),'
  + ' (orientation: landscape) and (max-height: 500px)';

/**
 * Which label a tab shows: the hull's `short_code` on a phone-sized bar, the
 * full Station name anywhere with room. One answer for the whole bar, read from
 * the viewport — never two hidden twins in the DOM, which would double the
 * accessible name of every tab.
 *
 * @param {{matchMedia?: function}|null} win
 * @returns {'code'|'name'}
 */
export function heroBarLabelMode(win) {
  const query = win && typeof win.matchMedia === 'function'
    ? win.matchMedia(HERO_BAR_CODE_QUERY) : null;
  return query && query.matches ? 'code' : 'name';
}

/**
 * Where the shell chrome — the settings cog and the help button — lives this
 * frame.
 *
 * The cog is ONE node that moves, not two that hide each other: in game it is
 * the bar's first item, and everywhere else it is the page-body `position:
 * fixed` control it has always been, stacked above the full-viewport surfaces.
 *
 * `heroVisible` alone is not enough to say "the bar is the chrome". The game
 * shell stays mounted through `GameOver`, and the waiting / scenario-picker /
 * asset-loading surfaces cover it while the phase is still in play — a cog
 * parented into a bar underneath one of those is a cog nobody can reach, which
 * is the exact failure issue #939 shipped on the host page.
 *
 * @param {{heroVisible: boolean, prePlaySurface: (string|null),
 *          gameOverVisible: boolean}} state
 * @returns {'bar'|'body'}
 */
export function heroChromeSlot({ heroVisible, prePlaySurface, gameOverVisible } = {}) {
  if (!heroVisible) return 'body';
  if (prePlaySurface) return 'body';
  if (gameOverVisible) return 'body';
  return 'bar';
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
 *
 * `onActivate(id, kind)` is called with the tab's id and either `'station'` or
 * `'overlay'` (issue #1373), so the shell routes a Station tab to the console
 * switch and an overlay tab to the console's own set-overlay hook without
 * having to re-derive which it was looking at.
 */
export function renderHeroBarDom({ tabsEl, titleEl, ratingEl, aiEl, model,
  translate, onActivate, labelMode = 'name' }) {
  // Keyed on `data-tab-id`, which EVERY tab carries, rather than on
  // `data-station`, which only a Station tab does (issue #1373). The two
  // attributes are kept apart on purpose: the reconcile needs an identity for
  // each button, and callers that mean "the tabs that are Stations" — the
  // responsive smoke spec, the shell's own selectors — need to be able to
  // still say exactly that without an overlay tab answering to it.
  const existing = new Map(
    [...tabsEl.querySelectorAll('button[data-tab-id]')]
      .map(button => [button.dataset.tabId, button]),
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
      button.dataset.tabId = tab.id;
      button.dataset.tabKind = tab.kind === 'overlay' ? 'overlay' : 'station';
      // A Station tab keeps `data-station`; an overlay tab names its panel with
      // `data-overlay`, the same attribute the console's own toggle used to
      // carry for it. Nothing carries both, so a selector still means one kind.
      if (tab.kind === 'overlay') button.dataset.overlay = tab.id;
      else button.dataset.station = tab.id;
      button.append(
        tabsEl.ownerDocument.createElement('span'),
        tabsEl.ownerDocument.createElement('span'),
        tabsEl.ownerDocument.createElement('span'),
        tabsEl.ownerDocument.createElement('span'),
        tabsEl.ownerDocument.createElement('span'),
      );
      // The damage indicator remains separate from importance, but becomes a
      // compact progress strip instead of a visible text row. Its hidden label
      // preserves the percentage/no-model fact for assistive technology.
      button.children[1].className = 'station-tab-health';
      const fill = tabsEl.ownerDocument.createElement('span');
      fill.className = 'station-tab-health-fill';
      fill.setAttribute('aria-hidden', 'true');
      const healthLabel = tabsEl.ownerDocument.createElement('span');
      healthLabel.className = 'station-tab-health-label visually-hidden';
      button.children[1].append(fill, healthLabel);
      // A SEPARATE span for the importance cue (issue #1101), with its own
      // `data-importance` attribute — never sharing health's element or
      // attribute, so the two streams coexist on one tab (AC4).
      button.children[2].className = 'station-tab-importance';
      // The full Station name, always in the tree and never drawn. In code mode
      // the visible glyph group is hidden from assistive technology and this
      // carries the name instead, so shrinking the bar changes what the tab
      // LOOKS like and nothing about what it is ANNOUNCED as.
      button.children[3].className = 'station-tab-name visually-hidden';
      // The unread badge (issue #1373). Its own child, like health and
      // importance before it, so a count and a cue can sit on one tab without
      // either overwriting the other. The digits are decoration — the hidden
      // label beside them is what a screen reader announces, because "3" on
      // its own says nothing about what there are three of.
      button.children[4].className = 'station-tab-badge';
      const badgeCount = tabsEl.ownerDocument.createElement('span');
      badgeCount.className = 'station-tab-badge-count';
      badgeCount.setAttribute('aria-hidden', 'true');
      const badgeLabel = tabsEl.ownerDocument.createElement('span');
      badgeLabel.className = 'station-tab-badge-label visually-hidden';
      button.children[4].append(badgeCount, badgeLabel);
    }
    existing.delete(tab.id);
    button.setAttribute('aria-selected', tab.id === model.selected ? 'true' : 'false');
    button.tabIndex = tab.id === model.selected ? 0 : -1;
    // Phone bars show the hull's authored short code; anything with room shows
    // the name. A Station whose hull authored no code keeps its name rather
    // than rendering an empty tab.
    const showCode = labelMode === 'code' && !!tab.code;
    const visibleLabel = button.children[0];
    visibleLabel.textContent = showCode ? tab.code : tab.name;
    if (showCode) visibleLabel.setAttribute('aria-hidden', 'true');
    else visibleLabel.removeAttribute('aria-hidden');
    button.children[3].textContent = showCode ? tab.name : '';
    button.title = tab.name;
    const healthEl = button.children[1];
    const healthFill = healthEl.querySelector('.station-tab-health-fill');
    const healthLabel = healthEl.querySelector('.station-tab-health-label');
    const healthPct = typeof tab.health === 'number'
      ? Math.round(Math.max(0, Math.min(1, tab.health)) * 100)
      : null;
    healthFill.hidden = healthPct == null;
    healthFill.style.width = healthPct === 0 ? '2px' : `${healthPct || 0}%`;
    healthFill.style.setProperty('--station-health-pct', `${healthPct || 0}%`);
    healthFill.style.setProperty('--station-health-loss-pct', `${100 - (healthPct || 0)}%`);
    healthLabel.textContent = healthPct == null
      ? translate('client.hero.health.none')
      : translate('client.hero.health.readout', { pct: healthPct });
    button.dataset.health = tab.healthState;
    button.dataset.healthValue = healthPct == null ? 'none' : String(healthPct);
    // Persistent per-tab importance cue on EVERY tab (AC4): its own glyph token
    // and its own `data-importance`, set UNCONDITIONALLY (even 'none') so health
    // and importance always coexist and neither can suppress the other. Never a
    // sort key — the tab order above is untouched by importance.
    button.children[2].textContent = translate('client.hero.importance.cue.' + tab.importanceState);
    button.dataset.importance = tab.importanceState;
    // Unread badge (issue #1373). Hidden outright at zero rather than drawn
    // empty, and `data-badge` states the number for anything asserting on it.
    const badgeEl = button.children[4];
    const badge = Number.isFinite(tab.badge) ? Math.max(0, Math.trunc(tab.badge)) : 0;
    badgeEl.hidden = badge === 0;
    badgeEl.querySelector('.station-tab-badge-count').textContent = badge === 0 ? '' : String(badge);
    badgeEl.querySelector('.station-tab-badge-label').textContent = badge === 0
      ? '' : translate('client.hero.badge.unread', { count: badge });
    button.dataset.badge = String(badge);
    button.onclick = () => onActivate(tab.id, tab.kind || 'station');
    button.onkeydown = event => {
      const target = heroBarKeyTarget(ids, tab.id, event.key);
      if (!target) return;
      event.preventDefault();
      const targetTab = model.tabs.find(candidate => candidate.id === target);
      onActivate(target, (targetTab && targetTab.kind) || 'station');
      [...tabsEl.querySelectorAll('button[data-tab-id]')]
        .find(candidate => candidate.dataset.tabId === target)?.focus();
    };
    // Moving an already-correct child through appendChild can itself blur it.
    // Only touch tree position when the authored tab order actually changed.
    const childAtIndex = tabsEl.children[index];
    if (childAtIndex !== button) tabsEl.insertBefore(button, childAtIndex || null);
  }
  const selected = model.tabs.find(tab => tab.id === model.selected) || model.tabs[0];
  titleEl.textContent = selected.name;
  ratingEl.hidden = !selected.rating;
  ratingEl.textContent = selected.rating
    ? translate('client.hero.rating', { rating: selected.rating })
    : '';
  const aiNames = model.aiStations.map(station => station.name).join(', ');
  aiEl.hidden = !aiNames;
  aiEl.textContent = aiNames ? translate('client.hero.ai_status', { stations: aiNames }) : '';
}

if (typeof window !== 'undefined') {
  window.heroBarModel = heroBarModel;
  window.heroBarKeyTarget = heroBarKeyTarget;
  window.renderHeroBarDom = renderHeroBarDom;
  window.heroBarLabelMode = heroBarLabelMode;
  window.heroChromeSlot = heroChromeSlot;
  window.HERO_BAR_CODE_QUERY = HERO_BAR_CODE_QUERY;
}
