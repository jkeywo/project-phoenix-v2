import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { JSDOM } from 'jsdom';
import { describe, expect, it, vi } from 'vitest';
import {
  HERO_BAR_CODE_QUERY, heroBarHealthState, heroBarImportanceState, heroBarKeyTarget,
  heroBarLabelMode, heroBarModel, heroChromeSlot, renderHeroBarDom,
} from '../../gui/hero-bar.js';
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
  if (id === 'client.hero.badge.unread') return `${values.count} unread`;
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
  // send('StationVisited', { station }); pinning the argument pins that
  // contract. The second argument is which KIND of tab was pressed (issue
  // #1373) — the shell routes a Station to the console switch and an overlay
  // to the console's own set-overlay hook.
  elements.tabsEl.querySelector('[data-station="navigation"]').click();
  expect(onActivate).toHaveBeenCalledWith('navigation', 'station');
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

// ── The bar as the page's only chrome (issue #1372) ──────────────────────────
//
// The bar gained the settings cog as its first item and a help button as its
// last, and its tabs shrink to the hull's authored short codes when the bar is
// phone-sized. Three separate contracts, tested separately: what the MODEL
// carries, what the DOM DRAWS at each label mode, and where the chrome LIVES.

describe('short codes', () => {
  const coded = [
    { id: 'helm', name: 'Helm', short_code: 'HLM' },
    { id: 'navigation', name: 'Navigation', human_seeking: true, short_code: 'NAV' },
    // Authored with no code at all — the TOML default is an empty string.
    { id: 'comms', name: 'Comms', human_seeking: true, short_code: '' },
  ];
  const model = () => heroBarModel({
    directStation: 'helm', stations: coded,
    stationHosts: {
      navigation: { station: 'navigation', host: 'helm', rating: 'Std' },
      comms: { station: 'comms', host: 'helm', rating: 'Simple' },
    },
    stationRatings: {}, activeStation: 'helm',
  });

  it('carries the hull-authored code verbatim rather than truncating the name', () => {
    const tabs = model().tabs;
    expect(tabs.map(tab => tab.code)).toEqual(['HLM', 'NAV', '']);
    // Never a derived abbreviation: an unauthored code stays empty so the
    // renderer can fall back to the name instead of inventing one.
    expect(tabs.map(tab => tab.name)).toEqual(['Helm', 'Navigation', 'Comms']);
  });

  it('draws codes in code mode and names in name mode, off one shared model', () => {
    const { elements } = heroDom();
    const labels = () => [...elements.tabsEl.querySelectorAll('button[data-station]')]
      .map(button => button.children[0].textContent);

    renderHeroBarDom({ ...elements, model: model(), translate, onActivate: vi.fn(),
      labelMode: 'code' });
    // The code-less Station keeps its name — a blank tab is not a label.
    expect(labels()).toEqual(['HLM', 'NAV', 'Comms']);

    renderHeroBarDom({ ...elements, model: model(), translate, onActivate: vi.fn(),
      labelMode: 'name' });
    expect(labels()).toEqual(['Helm', 'Navigation', 'Comms']);
  });

  it('defaults to names when no label mode is given', () => {
    const { elements } = heroDom();
    renderHeroBarDom({ ...elements, model: model(), translate, onActivate: vi.fn() });
    expect(elements.tabsEl.querySelector('[data-station="helm"]').children[0].textContent)
      .toBe('Helm');
  });

  it('announces the full Station name in both modes', () => {
    const { elements } = heroDom();
    const helm = () => elements.tabsEl.querySelector('[data-station="helm"]');

    renderHeroBarDom({ ...elements, model: model(), translate, onActivate: vi.fn(),
      labelMode: 'code' });
    // The glyph group is hidden from assistive technology and the full name
    // rides a visually-hidden span, so shrinking the bar changes what the tab
    // looks like and nothing about what it is announced as.
    expect(helm().children[0].getAttribute('aria-hidden')).toBe('true');
    expect(helm().querySelector('.station-tab-name').textContent).toBe('Helm');
    expect(helm().title).toBe('Helm');
    // Health still reads from the same tab: the label change takes nothing.
    expect(helm().querySelector('.station-tab-health')).not.toBeNull();

    renderHeroBarDom({ ...elements, model: model(), translate, onActivate: vi.fn(),
      labelMode: 'name' });
    expect(helm().children[0].hasAttribute('aria-hidden')).toBe(false);
    expect(helm().querySelector('.station-tab-name').textContent).toBe('');
  });
});

describe('label mode', () => {
  const stubWindow = matches => ({ matchMedia: query => ({ query, matches }) });

  it('asks for codes on a phone-sized bar and names anywhere with room', () => {
    expect(heroBarLabelMode(stubWindow(true))).toBe('code');
    expect(heroBarLabelMode(stubWindow(false))).toBe('name');
  });

  it('falls back to names where matchMedia is unavailable', () => {
    expect(heroBarLabelMode(null)).toBe('name');
    expect(heroBarLabelMode({})).toBe('name');
  });

  it('watches the two shapes the bar actually has', () => {
    // Portrait strip narrower than a phone, and the left rail on a phone held
    // sideways. client.html's stylesheet narrows the bar on the same pair.
    expect(HERO_BAR_CODE_QUERY).toContain('(orientation: portrait) and (max-width: 599px)');
    expect(HERO_BAR_CODE_QUERY).toContain('(orientation: landscape) and (max-height: 500px)');
  });
});

describe('where the cog and help live', () => {
  it('joins the bar only when the bar is the surface a player can see', () => {
    expect(heroChromeSlot({ heroVisible: true, prePlaySurface: null, gameOverVisible: false }))
      .toBe('bar');
  });

  it('goes back to the page body in the lobby', () => {
    expect(heroChromeSlot({ heroVisible: false, prePlaySurface: null, gameOverVisible: false }))
      .toBe('body');
  });

  it('goes back to the page body under every full-viewport surface', () => {
    // The game shell stays mounted through GameOver and under the pre-play
    // surfaces, so `heroVisible` alone would strand the cog beneath a panel it
    // is supposed to stack above — the failure issue #939 shipped on the host.
    for (const surface of ['waiting-overlay', 'scenario-picker-overlay', 'asset-loading']) {
      expect(heroChromeSlot({ heroVisible: true, prePlaySurface: surface, gameOverVisible: false }),
        'cog buried under ' + surface).toBe('body');
    }
    expect(heroChromeSlot({ heroVisible: true, prePlaySurface: null, gameOverVisible: true }))
      .toBe('body');
  });

  it('keeps roving arrows on the Station tabs and off the chrome', () => {
    const { dom, elements } = heroDom();
    const doc = dom.window.document;
    // The bar as client.html assembles it: cog first, tab list, help last, with
    // only the tabs inside the roving container.
    const bar = doc.createElement('nav');
    const cog = doc.createElement('button');
    cog.id = 'settings-btn';
    const help = doc.createElement('button');
    help.id = 'help-btn';
    elements.tabsEl.replaceWith(bar);
    bar.append(cog, elements.tabsEl, help);

    const model = heroBarModel({
      directStation: 'helm', stations,
      stationHosts: {
        navigation: { station: 'navigation', host: 'helm', rating: 'Std' },
        comms: { station: 'comms', host: 'helm', rating: 'Simple' },
      },
      stationRatings: {}, activeStation: 'comms',
    });
    const onActivate = vi.fn();
    renderHeroBarDom({ ...elements, model, translate, onActivate, labelMode: 'code' });

    // The chrome is not a tab: the reconcile neither adopts nor removes it.
    expect(bar.children[0]).toBe(cog);
    expect(bar.children[2]).toBe(help);
    expect(elements.tabsEl.querySelectorAll('button[data-station]').length).toBe(3);

    // Arrow-right off the last tab wraps to the first tab, never onto help.
    const last = elements.tabsEl.querySelector('[data-station="comms"]');
    last.onkeydown({ key: 'ArrowRight', preventDefault() {} });
    expect(onActivate).toHaveBeenCalledWith('helm', 'station');
    expect(doc.activeElement).toBe(elements.tabsEl.querySelector('[data-station="helm"]'));
  });
});

// ── Overlay tabs (issue #1373) ───────────────────────────────────────────────
//
// A console's overlay panels ride the same bar as its Stations, because they
// are the same choice ("what am I looking at") — but they are not Stations.
// These pin where they sit, what they carry that a Station tab does not, and
// what they must NOT carry that a Station tab does.

describe('overlay tabs', () => {
  const overlays = [
    { id: 'security-overlay', code: 'SEC', name: 'Security', badge: 0 },
    { id: 'intel-overlay', code: 'INTL', name: 'Intel', badge: 0 },
  ];

  const build = (extra = {}) => heroBarModel({
    directStation: 'helm', stations,
    stationHosts: {
      navigation: { station: 'navigation', host: 'helm', rating: 'Std' },
      comms: { station: 'comms', host: 'helm', rating: 'Simple' },
    },
    stationHealth: { helm: 0.5 },
    stationRatings: {}, activeStation: 'helm',
    consoleTabs: overlays,
    ...extra,
  });

  it('sits between the direct Station and its visitors', () => {
    expect(build().tabs.map(tab => tab.id))
      .toEqual(['helm', 'security-overlay', 'intel-overlay', 'navigation', 'comms']);
  });

  it('carries no health, rating, importance or ownership', () => {
    const model = build();
    const intel = model.tabs.find(tab => tab.id === 'intel-overlay');
    expect(intel).toMatchObject({
      kind: 'overlay', name: 'Intel', code: 'INTL',
      rating: '', health: null, healthState: 'none', importance: null,
      importanceState: 'none',
    });
    // An overlay is not a Station: it never appears in the ownership roll-call
    // and can never be reported as AI-operated.
    expect(model.ownership['intel-overlay']).toBeUndefined();
    expect(model.aiStations.map(s => s.id)).not.toContain('intel-overlay');
  });

  it('marks Station tabs as such, so the shell can route an activation', () => {
    expect(build().tabs.filter(tab => tab.kind === 'station').map(tab => tab.id))
      .toEqual(['helm', 'navigation', 'comms']);
  });

  it('is the selected tab while its panel is open, and not otherwise', () => {
    expect(build().selected).toBe('helm');
    expect(build({ activeOverlay: 'intel-overlay' }).selected).toBe('intel-overlay');
    // An id naming no declared overlay selects nothing new.
    expect(build({ activeOverlay: 'nav-overlay' }).selected).toBe('helm');
  });

  it('shows nothing when the console declared none, or there is no seat', () => {
    expect(build({ consoleTabs: [] }).tabs.map(t => t.id))
      .toEqual(['helm', 'navigation', 'comms']);
    expect(build({ consoleTabs: undefined }).tabs.map(t => t.id))
      .toEqual(['helm', 'navigation', 'comms']);
    // A spectator has no console for an overlay to be drawn inside.
    expect(heroBarModel({
      directStation: null, stations, stationRatings: {}, activeStation: null,
      consoleTabs: overlays,
    }).tabs).toEqual([]);
  });

  it('draws an overlay tab with its own data attributes, never data-station', () => {
    const { elements } = heroDom();
    renderHeroBarDom({ ...elements, model: build(), translate, onActivate: vi.fn() });
    const intel = elements.tabsEl.querySelector('[data-tab-id="intel-overlay"]');
    expect(intel.dataset.tabKind).toBe('overlay');
    expect(intel.dataset.overlay).toBe('intel-overlay');
    expect(intel.dataset.station).toBeUndefined();
    // …and the Station tabs keep theirs, so "the tabs that are Stations" is
    // still one selector.
    expect([...elements.tabsEl.querySelectorAll('button[data-station]')]
      .map(b => b.dataset.station)).toEqual(['helm', 'navigation', 'comms']);
  });

  it('tells the shell which kind of tab was pressed', () => {
    const { elements } = heroDom();
    const onActivate = vi.fn();
    renderHeroBarDom({ ...elements, model: build(), translate, onActivate });
    elements.tabsEl.querySelector('[data-tab-id="intel-overlay"]').click();
    expect(onActivate).toHaveBeenCalledWith('intel-overlay', 'overlay');
  });

  it('roves the keyboard across overlay tabs as well as Station tabs', () => {
    const { dom, elements } = heroDom();
    const onActivate = vi.fn();
    renderHeroBarDom({ ...elements, model: build(), translate, onActivate });
    const helm = elements.tabsEl.querySelector('[data-tab-id="helm"]');
    helm.onkeydown({ key: 'ArrowRight', preventDefault() {} });
    expect(onActivate).toHaveBeenCalledWith('security-overlay', 'overlay');
    expect(dom.window.document.activeElement)
      .toBe(elements.tabsEl.querySelector('[data-tab-id="security-overlay"]'));
  });

  it('shows the short code on a phone bar and the panel name where there is room', () => {
    const { elements } = heroDom();
    const label = () => elements.tabsEl
      .querySelector('[data-tab-id="intel-overlay"]').children[0].textContent;
    renderHeroBarDom({ ...elements, model: build(), translate, onActivate: vi.fn(),
      labelMode: 'code' });
    expect(label()).toBe('INTL');
    renderHeroBarDom({ ...elements, model: build(), translate, onActivate: vi.fn(),
      labelMode: 'name' });
    expect(label()).toBe('Intel');
  });

  it('leaves the neutral empty health track on an overlay tab', () => {
    const { elements } = heroDom();
    renderHeroBarDom({ ...elements, model: build(), translate, onActivate: vi.fn() });
    const intel = elements.tabsEl.querySelector('[data-tab-id="intel-overlay"]');
    expect(intel.dataset.health).toBe('none');
    expect(intel.querySelector('.station-tab-health-fill').hidden).toBe(true);
    // The seat's OWN health is unaffected by the overlay beside it.
    expect(elements.tabsEl.querySelector('[data-tab-id="helm"]').dataset.healthValue).toBe('50');
  });

  it('preserves tab identity across a re-render, overlays included', () => {
    const { elements } = heroDom();
    const args = { ...elements, translate, onActivate: vi.fn() };
    renderHeroBarDom({ ...args, model: build() });
    const intel = elements.tabsEl.querySelector('[data-tab-id="intel-overlay"]');
    intel.focus();
    renderHeroBarDom({ ...args, model: build({ activeOverlay: 'intel-overlay' }) });
    expect(elements.tabsEl.querySelector('[data-tab-id="intel-overlay"]')).toBe(intel);
    expect(intel.getAttribute('aria-selected')).toBe('true');
  });
});

describe('the unread badge', () => {
  const withBadge = badge => heroBarModel({
    directStation: 'tactical',
    stations: [{ id: 'tactical', name: 'Tactical' }],
    stationRatings: {}, activeStation: 'tactical',
    consoleTabs: [{ id: 'intel-overlay', code: 'INTL', name: 'Intel', badge }],
  });

  it('draws nothing at zero — a badge reading 0 never goes away', () => {
    const { elements } = heroDom();
    renderHeroBarDom({ ...elements, model: withBadge(0), translate, onActivate: vi.fn() });
    const intel = elements.tabsEl.querySelector('[data-tab-id="intel-overlay"]');
    expect(intel.querySelector('.station-tab-badge').hidden).toBe(true);
    expect(intel.dataset.badge).toBe('0');
  });

  it('draws the count, and reads it out with what it counts', () => {
    const { elements } = heroDom();
    renderHeroBarDom({ ...elements, model: withBadge(3), translate, onActivate: vi.fn() });
    const badge = elements.tabsEl
      .querySelector('[data-tab-id="intel-overlay"] .station-tab-badge');
    expect(badge.hidden).toBe(false);
    expect(badge.querySelector('.station-tab-badge-count').textContent).toBe('3');
    // The digits are decoration; the hidden label is what is announced.
    expect(badge.querySelector('.station-tab-badge-count').getAttribute('aria-hidden'))
      .toBe('true');
    expect(badge.querySelector('.station-tab-badge-label').textContent).toBe('3 unread');
    // …and the tab states the count on itself. That attribute is what the
    // stylesheet's `:not([data-badge="0"])` padding reservation selects on, so
    // a badged tab holds a column back for the pill instead of letting it land
    // on the label — without it "INTL" reads "INT" under the '3'.
    const intel = elements.tabsEl.querySelector('[data-tab-id="intel-overlay"]');
    expect(intel.dataset.badge).toBe('3');
    expect(intel.matches('button:not([data-badge="0"])')).toBe(true);
  });

  it('clears back to nothing on the same button when the count drops to zero', () => {
    const { elements } = heroDom();
    const args = { ...elements, translate, onActivate: vi.fn() };
    renderHeroBarDom({ ...args, model: withBadge(2) });
    const intel = elements.tabsEl.querySelector('[data-tab-id="intel-overlay"]');
    renderHeroBarDom({ ...args, model: withBadge(0) });
    expect(elements.tabsEl.querySelector('[data-tab-id="intel-overlay"]')).toBe(intel);
    expect(intel.querySelector('.station-tab-badge').hidden).toBe(true);
    expect(intel.querySelector('.station-tab-badge-count').textContent).toBe('');
  });

  it('never appears on a Station tab', () => {
    const { elements } = heroDom();
    renderHeroBarDom({ ...elements, model: withBadge(4), translate, onActivate: vi.fn() });
    const tactical = elements.tabsEl.querySelector('[data-tab-id="tactical"]');
    expect(tactical.querySelector('.station-tab-badge').hidden).toBe(true);
    expect(tactical.dataset.badge).toBe('0');
  });
});

// ── The shape client.html gives the bar (issue #1372) ────────────────────────
//
// The label decision above is a JS read of a media query; the bar's width is a
// stylesheet rule. They are one decision split across two files, so this reads
// client.html and checks the halves say the same thing. Source text, not
// layout — tests/smoke/hero-bar-responsive.spec.js measures the real boxes.

describe('client.html gives the bar the shape the labels assume', () => {
  const CLIENT_HTML = fs.readFileSync(
    path.join(path.dirname(fileURLToPath(import.meta.url)), '../../client.html'),
    'utf-8',
  );

  it('narrows on exactly the query gui/hero-bar.js switches labels on', () => {
    // Both conditions, in the same stylesheet block, so a phone can never get
    // a bar too narrow for the names it is still being told to draw.
    for (const condition of HERO_BAR_CODE_QUERY.split(',').map((part) => part.trim())) {
      expect(CLIENT_HTML, `no stylesheet rule for ${condition}`).toContain(condition);
    }
  });

  it('is a 52px strip in portrait: a 44px touch floor between 4px margins', () => {
    const rule = CLIENT_HTML.match(/#station-hero\s*\{([^}]*)\}/);
    expect(rule, 'no #station-hero rule').not.toBeNull();
    expect(rule[1]).toMatch(/min-height:\s*52px/);
    expect(rule[1]).toMatch(/padding:\s*4px/);
    // …and counts the padding and the border INSIDE that number. `box-sizing`
    // is set on `body` alone and does not inherit, so a bar that leaves it
    // unsaid reads 52px as content and draws a 61px strip.
    expect(rule[1], 'the strip does not count its own padding').toMatch(
      /box-sizing:\s*border-box/,
    );
  });

  it('is a left rail of 132px, narrowing to 96px on a phone held sideways', () => {
    const landscape = CLIENT_HTML.match(
      /@media \(orientation: landscape\)\s*\{[\s\S]*?#station-hero\s*\{([^}]*)\}/,
    );
    expect(landscape, 'no landscape rail rule').not.toBeNull();
    expect(landscape[1]).toMatch(/width:\s*132px/);
    const phone = CLIENT_HTML.match(
      /@media \(orientation: landscape\) and \(max-height: 500px\)\s*\{\s*#station-hero\s*\{([^}]*)\}/,
    );
    expect(phone, 'no narrow rail rule').not.toBeNull();
    expect(phone[1]).toMatch(/width:\s*96px/);
    // The rail is pressed against the notch and the home indicator on exactly
    // the devices this query selects, so its padding keeps the env() guards
    // the wider rail declares. A flat shorthand here — or, as it was, in the
    // block the two narrow shapes share — draws the cog under the notch.
    expect(phone[1], 'the narrow rail lost its safe-area insets').toMatch(
      /env\(safe-area-inset-top\)/,
    );
    expect(phone[1]).toMatch(/env\(safe-area-inset-bottom\)/);
    expect(phone[1]).toMatch(/env\(safe-area-inset-left\)/);
  });

  it('drops the title and rating on a narrow bar but keeps the AI live region', () => {
    const narrow = CLIENT_HTML.match(
      /@media \(orientation: portrait\) and \(max-width: 599px\),[\s\S]*?\n    \}\n/,
    );
    expect(narrow, 'no narrow-bar block').not.toBeNull();
    expect(narrow[0]).toMatch(/#station-hero-title,\s*#station-hero-rating\s*\{\s*display:\s*none/);
    // The AI roll-call is the only channel naming the Stations the ship flies
    // itself, so it is hidden from sight and NOT from a screen reader.
    const ai = narrow[0].match(/#station-hero-ai\s*\{([^}]*)\}/);
    expect(ai, 'the AI live region is not handled on a narrow bar').not.toBeNull();
    expect(ai[1]).toMatch(/clip:\s*rect\(0, 0, 0, 0\)/);
    expect(ai[1]).not.toMatch(/display:\s*none/);
    // Padding is NOT written here. This block matches a phone held sideways
    // too, at the same specificity as the rail's own rule and later in the
    // sheet, so a `padding` shorthand in it silently replaces the rail's
    // env() guards with flat pixels.
    const bar = narrow[0].match(/#station-hero\s*\{([^}]*)\}/);
    expect(bar ? bar[1] : '', 'the shared narrow block overrides the rail padding')
      .not.toMatch(/padding/);
  });

  // ── The overlay-tab seam's shell half (issue #1373) ───────────────────────
  //
  // The model above takes `consoleTabs` already chosen; choosing them is
  // client.html's job, and it is the part AC3 is about. Its inline script is
  // not importable, so these read the source for the two decisions that make
  // the difference between a bar that recovers and one that shows a background
  // console's tabs. tests/smoke/console-tabs.spec.js drives the real thing.

  it('stores each console declaration under its own console name', () => {
    // One iframe per Station is mounted at once and every one of them runs
    // initConsole, so a single slot would let whichever posted last win.
    expect(CLIENT_HTML).toMatch(/consoleTabsByConsole\[name\]\s*=/);
    expect(CLIENT_HTML).toMatch(/consoleHullByConsole\[name\]\s*=/);
  });

  it("renders only the active console's declaration", () => {
    expect(CLIENT_HTML).toMatch(/consoleTabs:\s*\(consoleTabsByConsole\[activeConsole\]/);
  });

  it('resolves the two tab labels through the string table, not raw', () => {
    // The console posts strings.csv ids; no English crosses the seam.
    const call = CLIENT_HTML.match(/consoleTabs:\s*\(consoleTabsByConsole\[activeConsole\][\s\S]*?\}\)\),/);
    expect(call, 'no consoleTabs mapping').not.toBeNull();
    expect(call[0]).toMatch(/code:\s*wireText\(tab\.code/);
    expect(call[0]).toMatch(/name:\s*wireText\(tab\.name/);
  });

  it("lets the console's own declaration settle which panel is open", () => {
    // The document is the truth about what is covering the console; the bar
    // only lights a tab optimistically on the tap. A console reporting no open
    // panel clears its OWN selection and says nothing about another's.
    expect(CLIENT_HTML).toMatch(/const open = event\.data\.open \|\| null;/);
    expect(CLIENT_HTML).toMatch(/if \(open\) openConsoleOverlay = \{ console: name, id: open \};/);
    expect(CLIENT_HTML).toMatch(/else if \(openConsoleOverlay\.console === name\)/);
  });

  it("drops a console's selection when its iframe reloads", () => {
    // A reloaded document has every panel closed, so a selection held over it
    // would show a tab selected with nothing behind it.
    const onLoad = CLIENT_HTML.match(/function _attachIframeLoadListener[\s\S]*?\n    \}\n/);
    expect(onLoad, 'no iframe load listener').not.toBeNull();
    expect(onLoad[0]).toMatch(/openConsoleOverlay\.console === consoleName/);
  });

  it('draws the unread badge as a hideable corner mark on the tab', () => {
    const rule = CLIENT_HTML.match(/\.station-tab-badge\s*\{([^}]*)\}/);
    expect(rule, 'no badge rule').not.toBeNull();
    expect(rule[1]).toMatch(/position:\s*absolute/);
    // The tab is a flex container, so `hidden` needs saying explicitly.
    expect(CLIENT_HTML).toMatch(/\.station-tab-badge\[hidden\]\s*\{\s*display:\s*none/);
  });

  it('reserves a column for the badge rather than drawing it over the label', () => {
    // The pill is absolutely positioned, and in the portrait strip the tab is
    // `width: auto` — shrink-wrapped to its label with `overflow: hidden`. With
    // nothing held back the pill lands ON the label: "INTL" renders "INT" under
    // a '3' and a two-digit count eats two glyphs. `data-badge` is written on
    // EVERY tab (renderHeroBarDom above), so `:not([data-badge="0"])` is exactly
    // the badged ones and unbadged tabs keep their own padding.
    const reserve = CLIENT_HTML.match(
      /#station-hero-tabs button:not\(\[data-badge="0"\]\)\s*\{([^}]*)\}/,
    );
    expect(reserve, 'nothing reserves room for the unread badge').not.toBeNull();
    const px = reserve[1].match(/padding-right:\s*(\d+)px/);
    expect(px, 'the reservation is not a padding-right').not.toBeNull();
    // Wide enough for a two-digit pill (~20px) plus its 2px offset — the badge
    // counts dossier subjects, so two digits is the realistic ceiling.
    expect(Number(px[1])).toBeGreaterThanOrEqual(24);
  });

  it('keeps the fixed chrome corner off the strip while the bar is the header', () => {
    // #top-bar is z-index 20 against the bar's 16 and shares its top-right
    // corner, which is where the help button now lives. Pushed below the strip
    // in portrait; in landscape the bar is a left rail and needs no offset.
    const offset = CLIENT_HTML.match(
      /@media \(orientation: portrait\)\s*\{\s*#console-container\.station-hero-visible ~ #top-bar\s*\{([^}]*)\}/,
    );
    expect(offset, 'nothing moves #top-bar off the Station strip').not.toBeNull();
    // Derived from the touch floor that sets the strip's height, so raising
    // the floor moves the corner with it rather than leaving it half-buried.
    expect(offset[1]).toMatch(/top:\s*calc\(var\(--control-hit-min\)/);
  });
});
