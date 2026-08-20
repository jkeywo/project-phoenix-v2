import { JSDOM } from 'jsdom';
import { describe, expect, it, vi } from 'vitest';
import { heroBarHealthState, heroBarKeyTarget, heroBarModel, renderHeroBarDom } from '../../gui/hero-bar.js';

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
  expect(model.tabs[0]).toMatchObject({ mode: 'direct', rating: 'Detailed' });
  expect(model.tabs[1]).toMatchObject({ mode: 'visiting', rating: 'Floor' });
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
    expect.objectContaining({ id: 'repair', mode: 'direct' }),
    expect.objectContaining({ id: 'power', mode: 'visiting', rating: 'Std' }),
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

function heroDom() {
  const dom = new JSDOM(
    '<div id="tabs"></div><div id="title"></div><div id="meta"></div><div id="ai"></div><div id="health"></div>',
    { url: 'https://phoenix.test/' },
  );
  const byId = id => dom.window.document.getElementById(id);
  return {
    dom,
    elements: {
      tabsEl: byId('tabs'), titleEl: byId('title'), metaEl: byId('meta'),
      aiEl: byId('ai'), healthEl: byId('health'),
    },
  };
}

function translate(id, values = {}) {
  if (id === 'client.hero.meta') return `${values.owner} / ${values.rating}`;
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

it('shows a persistent per-tab health cue that survives an importance alert', () => {
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

  // Every tab carries a non-colour health cue and a legible state token.
  const cueOf = id => elements.tabsEl
    .querySelector(`[data-station="${id}"] .station-tab-health`);
  expect(cueOf('helm').textContent).toBe('damaged');
  expect(cueOf('navigation').textContent).toBe('healthy');
  expect(cueOf('comms').textContent).toBe('none');
  for (const id of ['helm', 'navigation', 'comms']) {
    expect(elements.tabsEl.querySelector(`[data-station="${id}"]`).dataset.health)
      .toBeTruthy();
  }
  // Selected-Station readout comes from the host figure (40%).
  expect(elements.healthEl.textContent).toBe('Hull 40%');
  expect(elements.healthEl.dataset.health).toBe('damaged');

  // An importance alert painted elsewhere on the tab must not hide the cue.
  const helmTab = elements.tabsEl.querySelector('[data-station="helm"]');
  helmTab.dataset.alert = 'true';
  const badge = helmTab.ownerDocument.createElement('span');
  badge.className = 'tab-alert';
  helmTab.append(badge);

  renderHeroBarDom({ ...elements, model: build(), translate, onActivate: vi.fn() });

  expect(helmTab.dataset.alert).toBe('true');
  expect(helmTab.querySelector('.tab-alert')).not.toBeNull();
  expect(cueOf('helm').textContent).toBe('damaged');
});

it('renders the neutral no-damage-model readout for a Station with no damage', () => {
  const { elements } = heroDom();
  const model = heroBarModel({
    directStation: 'comms', stations,
    stationHosts: {},
    stationHealth: { comms: null },
    stationRatings: {}, activeStation: 'comms',
  });
  renderHeroBarDom({ ...elements, model, translate, onActivate: vi.fn() });
  expect(elements.healthEl.textContent).toBe('No damage model');
  expect(elements.healthEl.dataset.health).toBe('none');
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
