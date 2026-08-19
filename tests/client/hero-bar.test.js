import { JSDOM } from 'jsdom';
import { describe, expect, it, vi } from 'vitest';
import { heroBarKeyTarget, heroBarModel, renderHeroBarDom } from '../../gui/hero-bar.js';

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

function heroDom() {
  const dom = new JSDOM(
    '<div id="tabs"></div><div id="title"></div><div id="meta"></div><div id="ai"></div>',
    { url: 'https://phoenix.test/' },
  );
  const byId = id => dom.window.document.getElementById(id);
  return {
    dom,
    elements: { tabsEl: byId('tabs'), titleEl: byId('title'), metaEl: byId('meta'), aiEl: byId('ai') },
  };
}

function translate(id, values = {}) {
  if (id === 'client.hero.meta') return `${values.owner} / ${values.rating}`;
  if (id === 'client.hero.ai_status') return `AI: ${values.stations}`;
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
