import { JSDOM } from 'jsdom';
import { describe, expect, it, vi } from 'vitest';
import { heroBarHealthState, heroBarImportanceState, heroBarKeyTarget, heroBarModel, renderHeroBarDom } from '../../gui/hero-bar.js';
import { reconcileActiveConsole } from '../../gui/lobby-state.js';

const stations = [
  { id: 'helm', name: 'Helm' },
  { id: 'navigation', name: 'Navigation', human_seeking: true, visiting_rating: 'Std' },
  { id: 'comms', name: 'Comms', human_seeking: true, visiting_rating: 'Simple' },
];

it('pins the direct Station first and keeps visitors in hull order', () => {
  const model = heroBarModel({
    directStation: 'helm', stations,
    stationSystems: { navigation: ['navigation'], comms: ['comms'] },
    stationHosts: {
      navigation: { station: 'navigation', host: 'helm', rating: 'Floor' },
      comms: { station: 'comms', host: 'helm', rating: 'Simple' },
    },
    stationRatings: { helm: 'Detailed' }, activeStation: 'comms',
  });
  expect(model.tabs.map(tab => tab.id)).toEqual(['helm', 'navigation', 'comms']);
  expect(model.tabs[0]).toMatchObject({ rating: 'Detailed' });
  expect(model.tabs[1]).toMatchObject({ rating: 'Floor' });
  expect(model.selected).toBe('comms');
});

it('states direct, visiting and AI ownership independently of colour', () => {
  const model = heroBarModel({
    directStation: 'helm', stations,
    stationSystems: { navigation: ['navigation'], comms: ['comms'] },
    stationHosts: {
      navigation: { station: 'navigation', host: 'helm', rating: 'Std' },
      comms: { station: 'comms', host: null, rating: 'Backfill' },
    },
    stationRatings: {}, activeStation: 'helm',
  });
  expect(model.ownership).toEqual({ navigation: 'visiting', comms: 'ai', helm: 'direct' });
  expect(model.aiStations).toEqual([{ id: 'comms', name: 'Comms' }]);
});

it('hosts a generic Power Station on Repair without a family blackboard', () => {
  const model = heroBarModel({
    directStation: 'repair',
    stations: [
      { id: 'repair', name: 'Repair' },
      { id: 'power', name: 'Power', human_seeking: true, visiting_rating: 'Simplified' },
    ],
    stationSystems: { power: ['power-reactor', 'power-battery'] },
    blackboards: {},
    stationHosts: {
      power: { station: 'power', host: 'repair', rating: 'Std' },
    },
    stationRatings: { repair: 'Std' },
    activeStation: 'power',
  });
  expect(model.tabs).toEqual([
    expect.objectContaining({ id: 'repair' }),
    expect.objectContaining({ id: 'power', rating: 'Std' }),
  ]);
  expect(model.ownership.power).toBe('visiting');
});

it('sources per-tab health from the authoritative host map, not damage rows', () => {
  const model = heroBarModel({
    directStation: 'helm', stations,
    // stationSystems/blackboards are present but MUST NOT feed health (AC #3).
    stationSystems: { navigation: ['navigation'], comms: ['comms'] },
    blackboards: {},
    stationHosts: {
      navigation: { station: 'navigation', host: 'helm', rating: 'Std' },
      comms: { station: 'comms', host: 'helm', rating: 'Simple' },
    },
    // helm damaged, navigation healthy, comms has no damage model (explicit null).
    stationHealth: { helm: 0.4, navigation: 1, comms: null },
    stationRatings: {}, activeStation: 'helm',
  });
  const byId = Object.fromEntries(model.tabs.map(tab => [tab.id, tab]));
  expect(byId.helm).toMatchObject({ health: 0.4, healthState: 'damaged' });
  expect(byId.navigation).toMatchObject({ health: 1, healthState: 'healthy' });
  // Explicit no-damage-model AND simply absent both normalise to the neutral state.
  expect(byId.comms).toMatchObject({ health: null, healthState: 'none' });
});

it('classifies health states without a tunable threshold', () => {
  expect(heroBarHealthState(null)).toBe('none');
  expect(heroBarHealthState(undefined)).toBe('none');
  expect(heroBarHealthState(1)).toBe('healthy');
  expect(heroBarHealthState(0.999)).toBe('damaged');
  expect(heroBarHealthState(0)).toBe('damaged');
});

it('classifies importance into four independent-lifecycle states', () => {
  expect(heroBarImportanceState(null)).toBe('none');
  expect(heroBarImportanceState(undefined)).toBe('none');
  expect(heroBarImportanceState({ unread: false, critical: false })).toBe('none');
  expect(heroBarImportanceState({ unread: true, critical: false })).toBe('unread');
  expect(heroBarImportanceState({ unread: false, critical: true })).toBe('critical');
  // A one-off event AND a continuing condition at once is its own state, not a
  // precedence collapse — the two lifecycles are independent.
  expect(heroBarImportanceState({ unread: true, critical: true })).toBe('both');
});

it('sources per-tab importance from the host map, held apart from health', () => {
  const model = heroBarModel({
    directStation: 'helm', stations,
    stationHosts: {
      navigation: { station: 'navigation', host: 'helm', rating: 'Std' },
      comms: { station: 'comms', host: 'helm', rating: 'Simple' },
    },
    // helm is damaged AND has a critical condition; navigation is healthy with a
    // one-off unread event; comms has neither health damage nor importance.
    stationHealth: { helm: 0.4, navigation: 1, comms: null },
    stationImportance: {
      helm: { unread: false, critical: true },
      navigation: { unread: true, critical: false },
    },
    stationRatings: {}, activeStation: 'helm',
  });
  const byId = Object.fromEntries(model.tabs.map(tab => [tab.id, tab]));
  // Health and importance are carried as separate fields with separate states.
  expect(byId.helm).toMatchObject({ healthState: 'damaged', importanceState: 'critical' });
  expect(byId.navigation).toMatchObject({ healthState: 'healthy', importanceState: 'unread' });
  expect(byId.comms).toMatchObject({ healthState: 'none', importanceState: 'none' });
});

function heroDom() {
  const dom = new JSDOM(
    '<div id="tabs"></div><div id="title"></div><div id="rating"></div><div id="ai"></div>',
    { url: 'https://phoenix.test/' },
  );
  const byId = id => dom.window.document.getElementById(id);
  return {
    dom,
    elements: {
      tabsEl: byId('tabs'), titleEl: byId('title'), ratingEl: byId('rating'),
      aiEl: byId('ai'),
    },
  };
}

function translate(id, values = {}) {
  if (id === 'client.hero.rating') return `Rating: ${values.rating}`;
  if (id === 'client.hero.ai_status') return `AI: ${values.stations}`;
  if (id === 'client.hero.health.readout') return `Hull ${values.pct}%`;
  if (id === 'client.hero.health.none') return 'No damage model';
  return id.split('.').at(-1);
}

it('preserves the focused tab node across routine state renders', () => {
  const { dom, elements } = heroDom();
  const onActivate = vi.fn();
  const args = {
    ...elements,
    model: heroBarModel({
      directStation: 'helm', stations,
      stationSystems: { navigation: ['navigation'], comms: ['comms'] },
      stationHosts: {
        navigation: { station: 'navigation', host: 'helm', rating: 'Std' },
        comms: { station: 'comms', host: 'helm', rating: 'Simple' },
      },
      stationRatings: { helm: 'Detailed' }, activeStation: 'comms',
    }),
    translate,
    onActivate,
  };
  renderHeroBarDom(args);
  const focused = elements.tabsEl.querySelector('[data-station="comms"]');
  focused.focus();

  renderHeroBarDom({
    ...args,
    model: heroBarModel({
      directStation: 'helm', stations,
      stationSystems: { navigation: ['navigation'], comms: ['comms'] },
      stationHosts: {
        navigation: { station: 'navigation', host: null, rating: 'Backfill' },
        comms: { station: 'comms', host: 'helm', rating: 'Simple' },
      },
      stationRatings: { helm: 'Detailed' }, activeStation: 'comms',
    }),
  });

  expect(elements.tabsEl.querySelector('[data-station="comms"]')).toBe(focused);
  expect(dom.window.document.activeElement).toBe(focused);
});

it('renders accessible per-tab progress bars that survive an importance alert', () => {
  const { elements } = heroDom();
  const build = () => heroBarModel({
    directStation: 'helm', stations,
    stationHosts: {
      navigation: { station: 'navigation', host: 'helm', rating: 'Std' },
      comms: { station: 'comms', host: 'helm', rating: 'Simple' },
    },
    stationHealth: { helm: 0.4, navigation: 1, comms: null },
    stationRatings: {}, activeStation: 'helm',
  });
  renderHeroBarDom({ ...elements, model: build(), translate, onActivate: vi.fn() });

  const barOf = id => elements.tabsEl
    .querySelector(`[data-station="${id}"] .station-tab-health`);
  const fillOf = id => barOf(id).querySelector('.station-tab-health-fill');
  const labelOf = id => barOf(id).querySelector('.station-tab-health-label');
  expect(fillOf('helm').style.width).toBe('40%');
  expect(fillOf('helm').style.getPropertyValue('--station-health-pct')).toBe('40%');
  expect(fillOf('helm').style.getPropertyValue('--station-health-loss-pct')).toBe('60%');
  expect(fillOf('navigation').style.width).toBe('100%');
  expect(fillOf('comms').hidden).toBe(true);
  expect(labelOf('helm').textContent).toBe('Hull 40%');
  expect(labelOf('navigation').textContent).toBe('Hull 100%');
  expect(labelOf('comms').textContent).toBe('No damage model');
  for (const id of ['helm', 'navigation', 'comms']) {
    expect(elements.tabsEl.querySelector(`[data-station="${id}"]`).dataset.health)
      .toBeTruthy();
  }
  expect(elements.tabsEl.querySelector('.station-tab-owner')).toBeNull();
  expect(elements.ratingEl.textContent).toBe('');

  // An importance alert painted elsewhere on the tab must not hide the cue.
  const helmTab = elements.tabsEl.querySelector('[data-station="helm"]');
  helmTab.dataset.alert = 'true';
  const badge = helmTab.ownerDocument.createElement('span');
  badge.className = 'tab-alert';
  helmTab.append(badge);

  renderHeroBarDom({ ...elements, model: build(), translate, onActivate: vi.fn() });

  expect(helmTab.dataset.alert).toBe('true');
  expect(helmTab.querySelector('.tab-alert')).not.toBeNull();
  expect(fillOf('helm').style.width).toBe('40%');
});

it('keeps a red endpoint at zero health and shows rating without ownership text', () => {
  const { elements } = heroDom();
  const model = heroBarModel({
    directStation: 'helm', stations,
    stationHealth: { helm: 0 },
    stationRatings: { helm: 'Detailed' }, activeStation: 'helm',
  });

  renderHeroBarDom({ ...elements, model, translate, onActivate: vi.fn() });

  const tab = elements.tabsEl.querySelector('[data-station="helm"]');
  const fill = tab.querySelector('.station-tab-health-fill');
  expect(fill.style.width).toBe('2px');
  expect(fill.style.getPropertyValue('--station-health-pct')).toBe('0%');
  expect(fill.style.getPropertyValue('--station-health-loss-pct')).toBe('100%');
  expect(tab.dataset.healthValue).toBe('0');
  expect(tab.querySelector('.station-tab-health-label').textContent).toBe('Hull 0%');
  expect(elements.ratingEl.textContent).toBe('Rating: Detailed');
  expect(tab.querySelector('.station-tab-owner')).toBeNull();
});

it('renders a persistent per-tab importance cue on every tab, coexisting with health', () => {
  const { elements } = heroDom();
  const model = heroBarModel({
    directStation: 'helm', stations,
    stationHosts: {
      navigation: { station: 'navigation', host: 'helm', rating: 'Std' },
      comms: { station: 'comms', host: 'helm', rating: 'Simple' },
    },
    stationHealth: { helm: 0.4, navigation: 1, comms: null },
    stationImportance: {
      helm: { unread: true, critical: true },
      navigation: { unread: false, critical: true },
    },
    stationRatings: {}, activeStation: 'helm',
  });
  renderHeroBarDom({ ...elements, model, translate, onActivate: vi.fn() });

  const healthCue = id => elements.tabsEl.querySelector(`[data-station="${id}"] .station-tab-health`);
  const importanceCue = id => elements.tabsEl.querySelector(`[data-station="${id}"] .station-tab-importance`);

  // Health and importance are SEPARATE spans with SEPARATE data attributes, both
  // present on every tab — neither suppresses the other (AC4).
  for (const id of ['helm', 'navigation', 'comms']) {
    const tab = elements.tabsEl.querySelector(`[data-station="${id}"]`);
    expect(healthCue(id)).not.toBeNull();
    expect(importanceCue(id)).not.toBeNull();
    expect(tab.dataset.health).toBeTruthy();
    expect(tab.dataset.importance).toBeTruthy();
  }
  // The importance cue reflects each tab's own state, unconditionally (even 'none').
  expect(elements.tabsEl.querySelector('[data-station="helm"]').dataset.importance).toBe('both');
  expect(elements.tabsEl.querySelector('[data-station="navigation"]').dataset.importance).toBe('critical');
  expect(elements.tabsEl.querySelector('[data-station="comms"]').dataset.importance).toBe('none');
  // Health cue is unchanged and legible beside it.
  expect(elements.tabsEl.querySelector('[data-station="helm"]').dataset.health).toBe('damaged');
  expect(importanceCue('comms').textContent).toBe('none');
});

it('never lets importance reorder the tabs', () => {
  const { elements } = heroDom();
  const build = importance => heroBarModel({
    directStation: 'helm', stations,
    stationHosts: {
      navigation: { station: 'navigation', host: 'helm', rating: 'Std' },
      comms: { station: 'comms', host: 'helm', rating: 'Simple' },
    },
    stationImportance: importance,
    stationRatings: {}, activeStation: 'helm',
  });
  const order = () => [...elements.tabsEl.querySelectorAll('button[data-station]')]
    .map(b => b.dataset.station);

  renderHeroBarDom({ ...elements, model: build({}), translate, onActivate: vi.fn() });
  const before = order();
  expect(before).toEqual(['helm', 'navigation', 'comms']);

  // Marking a later tab critical must NOT hoist it — order is authored, never
  // an importance sort key (AC4).
  renderHeroBarDom({
    ...elements,
    model: build({ comms: { unread: true, critical: true } }),
    translate,
    onActivate: vi.fn(),
  });
  expect(order()).toEqual(before);
});

it('reports the visited Station id to onActivate (the StationVisited contract)', () => {
  const { elements } = heroDom();
  const onActivate = vi.fn();
  const model = heroBarModel({
    directStation: 'helm', stations,
    stationHosts: { navigation: { station: 'navigation', host: 'helm', rating: 'Std' } },
    stationImportance: { navigation: { unread: true, critical: false } },
    stationRatings: {}, activeStation: 'helm',
  });
  renderHeroBarDom({ ...elements, model, translate, onActivate });
  // client.html's onActivate forwards this exact id verbatim into
  // send('StationVisited', { station }); pinning the argument pins that contract.
  elements.tabsEl.querySelector('[data-station="navigation"]').click();
  expect(onActivate).toHaveBeenCalledWith('navigation');
});

it('renders a neutral empty track for a Station with no damage model', () => {
  const { elements } = heroDom();
  const model = heroBarModel({
    directStation: 'comms', stations,
    stationHosts: {},
    stationHealth: { comms: null },
    stationRatings: {}, activeStation: 'comms',
  });
  renderHeroBarDom({ ...elements, model, translate, onActivate: vi.fn() });
  const tab = elements.tabsEl.querySelector('[data-station="comms"]');
  expect(tab.dataset.health).toBe('none');
  expect(tab.dataset.healthValue).toBe('none');
  expect(tab.querySelector('.station-tab-health-fill').hidden).toBe(true);
  expect(tab.querySelector('.station-tab-health-label').textContent).toBe('No damage model');
});

it('renders AI ownership as visible live status without making an AI tab', () => {
  const { elements } = heroDom();
  const model = heroBarModel({
    directStation: 'helm', stations,
    stationSystems: { navigation: ['navigation'], comms: ['comms'] },
    blackboards: { navigation: { host_station: 'helm' }, comms: { host_station: null } },
    stationRatings: {}, activeStation: 'helm',
  });

  renderHeroBarDom({ ...elements, model, translate, onActivate: vi.fn() });

  expect(elements.tabsEl.querySelector('[data-station="comms"]')).toBeNull();
  expect(elements.aiEl.hidden).toBe(false);
  expect(elements.aiEl.textContent).toBe('AI: Comms');
});

it('falls back to the direct tab when a selected visitor leaves', () => {
  const model = heroBarModel({
    directStation: 'helm', stations, stationSystems: { navigation: ['navigation'] },
    blackboards: { navigation: { host_station: null } }, stationRatings: {},
    activeStation: 'navigation',
  });
  expect(model.selected).toBe('helm');
  expect(model.tabs.map(tab => tab.id)).toEqual(['helm']);
});

// Issue #1099 AC4: when the selected visiting Station leaves, focus returns to
// the primary tab; a later return restores the visitor's context WITHOUT
// stealing focus. This exercises the exact heroBarModel → reconcileActiveConsole
// chain client.html runs on every reconcile (client.html:1651-1667).
it('returns a visitor without stealing focus from the primary tab', () => {
  const host = h => ({
    navigation: h ? { station: 'navigation', host: 'helm', rating: 'Std' } : null,
  });
  const build = (activeStation, present) => heroBarModel({
    directStation: 'helm', stations, stationHosts: host(present),
    stationRatings: {}, activeStation,
  });

  // Player is looking at the visiting Navigation tab.
  let active = 'navigation';
  expect(build(active, true).selected).toBe('navigation');

  // Navigation leaves: it drops from the tabs, so the model falls back to the
  // primary, and the reconciler moves the active console there too.
  const gone = build(active, false);
  expect(gone.tabs.map(t => t.id)).toEqual(['helm']);
  expect(gone.selected).toBe('helm');
  active = reconcileActiveConsole(active, gone.tabs.map(t => t.id));
  expect(active).toBe('helm');

  // Navigation returns. The active console is now the primary and still present,
  // so focus must STAY on the primary — the returning visitor does not grab it —
  // while its tab (and thus its persistent context) is back and available.
  const back = build(active, true);
  expect(reconcileActiveConsole(active, back.tabs.map(t => t.id))).toBe('helm');
  expect(back.selected).toBe('helm');
  expect(back.tabs.map(t => t.id)).toEqual(['helm', 'navigation']);
});

describe('keyboard roving focus', () => {
  const ids = ['helm', 'navigation', 'comms'];
  it('wraps arrow keys', () => {
    expect(heroBarKeyTarget(ids, 'comms', 'ArrowRight')).toBe('helm');
    expect(heroBarKeyTarget(ids, 'helm', 'ArrowLeft')).toBe('comms');
  });
  it('supports Home and End', () => {
    expect(heroBarKeyTarget(ids, 'navigation', 'Home')).toBe('helm');
    expect(heroBarKeyTarget(ids, 'navigation', 'End')).toBe('comms');
  });
});
