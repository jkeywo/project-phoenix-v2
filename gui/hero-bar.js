/** Shared complete-Station Hero Bar model (issue #1097). */

/**
 * Whether a `Command`-kind blackboard reports a directable target — the one
 * fact both the hero bar (this file) and the captain's visiting-system
 * payload (`gui/console-state.js`'s `withVisitingSystems`) need, kept as one
 * exported predicate so the two never drift on what "directable" means.
 *
 * @param {{directed_station_ai?: boolean}|null} board a `Command`-kind
 *   blackboard entry (or any value — non-boards simply read as not directable)
 * @returns {boolean}
 */
export function isCommandBoardDirectable(board) {
  return !!board && board.directed_station_ai === true;
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
  blackboards, consoleHealth, stationImportance, stationRatings, activeStation,
  consoleTabs, activeOverlay, consoles, consoleHosts, consoleImportance }) {
  return consoleHeroBarModel({ directStation, stations, consoles: consoles || [], consoleHosts,
    stationRatings, consoleHealth, consoleImportance, activeStation, consoleTabs, activeOverlay, blackboards });
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
  if (key === 'ArrowRight' || key === 'ArrowDown') return ids[(index + 1) % ids.length];
  if (key === 'ArrowLeft' || key === 'ArrowUp') return ids[(index - 1 + ids.length) % ids.length];
  return null;
}

function textIfChanged(node, value) {
  if (node.textContent !== value) node.textContent = value;
}

function attributeIfChanged(node, name, value) {
  if (node.getAttribute(name) === value) return;
  if (value == null) node.removeAttribute(name);
  else node.setAttribute(name, value);
}

function propertyIfChanged(node, name, value) {
  if (node[name] !== value) node[name] = value;
}

function styleIfChanged(node, name, value) {
  if (node.style.getPropertyValue(name) !== value) node.style.setProperty(name, value);
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
    attributeIfChanged(button, 'aria-selected', tab.id === model.selected ? 'true' : 'false');
    propertyIfChanged(button, 'tabIndex', tab.id === model.selected ? 0 : -1);
    // Phone bars show the hull's authored short code; anything with room shows
    // the name. A Station whose hull authored no code keeps its name rather
    // than rendering an empty tab.
    const showCode = labelMode === 'code' && !!tab.code;
    const visibleLabel = button.children[0];
    textIfChanged(visibleLabel, showCode ? tab.code : tab.name);
    attributeIfChanged(visibleLabel, 'aria-hidden', showCode ? 'true' : null);
    textIfChanged(button.children[3], showCode ? tab.name : '');
    propertyIfChanged(button, 'title', tab.name);
    const healthEl = button.children[1];
    const healthFill = healthEl.querySelector('.station-tab-health-fill');
    const healthLabel = healthEl.querySelector('.station-tab-health-label');
    const healthPct = typeof tab.health === 'number'
      ? Math.round(Math.max(0, Math.min(1, tab.health)) * 100)
      : null;
    propertyIfChanged(healthFill, 'hidden', healthPct == null);
    styleIfChanged(healthFill, 'width', healthPct === 0 ? '2px' : `${healthPct || 0}%`);
    styleIfChanged(healthFill, '--station-health-pct', `${healthPct || 0}%`);
    styleIfChanged(healthFill, '--station-health-loss-pct', `${100 - (healthPct || 0)}%`);
    textIfChanged(healthLabel, healthPct == null
      ? translate('client.hero.health.none')
      : translate('client.hero.health.readout', { pct: healthPct }));
    attributeIfChanged(button, 'data-health', tab.healthState);
    attributeIfChanged(button, 'data-health-value', healthPct == null ? 'none' : String(healthPct));
    // Persistent per-tab importance cue on EVERY tab (AC4): its own glyph token
    // and its own `data-importance`, set UNCONDITIONALLY (even 'none') so health
    // and importance always coexist and neither can suppress the other. Never a
    // sort key — the tab order above is untouched by importance.
    textIfChanged(button.children[2], translate('client.hero.importance.cue.' + tab.importanceState));
    attributeIfChanged(button, 'data-importance', tab.importanceState);
    // Unread badge (issue #1373). Hidden outright at zero rather than drawn
    // empty, and `data-badge` states the number for anything asserting on it.
    const badgeEl = button.children[4];
    const badge = Number.isFinite(tab.badge) ? Math.max(0, Math.trunc(tab.badge)) : 0;
    propertyIfChanged(badgeEl, 'hidden', badge === 0);
    textIfChanged(badgeEl.querySelector('.station-tab-badge-count'), badge === 0 ? '' : String(badge));
    textIfChanged(badgeEl.querySelector('.station-tab-badge-label'), badge === 0
      ? '' : translate('client.hero.badge.unread', { count: badge }));
    attributeIfChanged(button, 'data-badge', String(badge));
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
  if (!selected) { textIfChanged(titleEl, ''); propertyIfChanged(ratingEl, 'hidden', true); textIfChanged(aiEl, ''); propertyIfChanged(aiEl, 'hidden', true); return; }
  textIfChanged(titleEl, selected.name);
  propertyIfChanged(ratingEl, 'hidden', !selected.rating);
  textIfChanged(ratingEl, selected.rating
    ? translate('client.hero.rating', { rating: selected.rating })
    : '');
  const aiNames = model.aiStations.map(station => station.name).join(', ');
  propertyIfChanged(aiEl, 'hidden', !aiNames);
  textIfChanged(aiEl, aiNames ? translate('client.hero.ai_status', { stations: aiNames }) : '');
}

if (typeof window !== 'undefined') {
  window.heroBarModel = heroBarModel;
  window.isCommandBoardDirectable = isCommandBoardDirectable;
  window.heroBarKeyTarget = heroBarKeyTarget;
  window.renderHeroBarDom = renderHeroBarDom;
  window.heroBarLabelMode = heroBarLabelMode;
  window.heroChromeSlot = heroChromeSlot;
  window.HERO_BAR_CODE_QUERY = HERO_BAR_CODE_QUERY;
}


/** Console tabs use instance identity; Station identity remains the occupied seat. */
function consoleHeroBarModel({ directStation, stations, consoles, consoleHosts, stationRatings,
  consoleHealth, consoleImportance, activeStation, consoleTabs, activeOverlay, blackboards }) {
  const seats = Object.fromEntries((stations || []).map(s => [s.id, s]));
  const placement = c => consoleHosts?.[c.id] || { host: c.human_seeking ? null : c.home_station, rating: '' };
  const visible = consoles.filter(c => {
    if (!c.screen) return false;
    const commandBoards = (c.systems || []).map(id => blackboards?.[id])
      .filter(board => typeof board?.directed_station_ai === 'boolean');
    return c.command_target ? commandBoards.some(isCommandBoardDirectable)
      : commandBoards.length === 0 || commandBoards.some(isCommandBoardDirectable);
  });
  const hosted = directStation ? visible.filter(c => placement(c).host === directStation) : [];
  const selectedConsole = hosted.some(c => c.id === activeStation) ? activeStation : hosted[0]?.id || null;
  const overlays = selectedConsole && selectedConsole === activeStation ? (consoleTabs || []).filter(t => t?.id) : [];
  const selected = overlays.some(t => t.id === activeOverlay) ? activeOverlay : selectedConsole;
  const tabs = hosted.map(c => {
    const seat = seats[c.home_station] || {};
    const health = typeof consoleHealth?.[c.id] === 'number' ? consoleHealth[c.id] : null;
    const importance = consoleImportance?.[c.id] || null;
    return { id: c.id, kind: 'station', name: c.name || seat.name || c.id, code: c.short_code || seat.short_code || '',
      rating: placement(c).rating || stationRatings?.[c.home_station] || '', health,
      healthState: heroBarHealthState(health), importance, importanceState: heroBarImportanceState(importance),
      badge: 0, selected: c.id === selected };
  });
  const overlayTabs = overlays.map(t => ({ ...t, kind: 'overlay', code: t.code || '', rating: '', health: null,
    healthState: 'none', importance: null, importanceState: 'none', badge: Math.max(0, Number(t.badge) || 0), selected: t.id === selected }));
  const index = tabs.findIndex(t => t.id === selectedConsole);
  tabs.splice(index + 1, 0, ...overlayTabs);
  return { selected, tabs, ownership: Object.fromEntries(visible.map(c => [c.id,
    !placement(c).host ? 'ai' : placement(c).host === c.home_station ? 'direct' : 'visiting'])),
    aiStations: visible.filter(c => c.human_seeking && !placement(c).host).map(c => ({ id: c.id, name: c.name || c.id })) };
}
