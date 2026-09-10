// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import {
  GM_HEALTH_STATES,
  createGmHealthBanner,
  healthStateLabelId,
  parseGmHealthProjection,
  worstHealthState,
} from '../../gui/gm-health-banner.js';
import { createGmHealthPanel } from '../../gui/gm-health-panel.js';
import { createGmAttentionPanel } from '../../gui/gm-attention-panel.js';
import { createGmAttentionFilters } from '../../gui/gm-attention-filters.js';
import { createHostChannel } from '../../gui/host-channel.js';

/** The exact ids server.html carries for both panels. */
const MARKUP = `
  <section id="gm-attention-panel">
    <h2 id="gm-attention-heading"></h2>
    <div id="gm-attention-banners" hidden></div>
    <div id="gm-attention-filters">
      <select id="gm-attention-filter-band"></select>
      <select id="gm-attention-filter-category"></select>
      <select id="gm-attention-filter-ship"></select>
    </div>
    <p id="gm-attention-status"></p>
    <button id="gm-attention-live" hidden></button>
    <div id="gm-attention-list"></div>
    <p id="gm-attention-empty"></p>
  </section>
  <section id="gm-health-panel">
    <h2 id="gm-health-heading"></h2>
    <p id="gm-health-summary"></p>
    <p id="gm-health-tick"></p>
    <div id="gm-health-groups"></div>
    <p id="gm-health-empty"></p>
  </section>`;

/** Every lookup this stub answers, so a sentence built from a resolved WORD can
 * be told apart from one that merely happens to mention a state. */
const lookups = [];
const t = (id, params = {}) => {
  lookups.push([id, params]);
  return Object.entries(params)
    .reduce((text, [key, value]) => text.replaceAll(`{${key}}`, String(value)), id);
};
const has = (id) => typeof id === 'string' && id.startsWith('server.');

const SHIP = { entity_id: 'ship-a', name: 'Valiant' };

function alert(id, extra = {}) {
  return {
    id,
    kind: 'station_disconnected',
    severity: 'disconnected',
    reason: {
      id: 'server.gm.health.reason.station_disconnected',
      params: { operator: 'Morgan', station: 'Helm', ship: 'Valiant' },
    },
    ship: SHIP,
    station: 'helm',
    first_seen_tick: 120,
    ...extra,
  };
}

function projection(extra = {}) {
  return {
    tick: 480,
    paused: false,
    input_delay_ticks: 6,
    recovery: null,
    peers: [
      { id: 'ship:ship-a', ship: SHIP, operators: [], state: 'live', behind_ticks: null, local: true },
      { id: 'ship:ship-b', ship: { entity_id: 'ship-b', name: 'Kestrel' }, operators: [], state: 'stale', behind_ticks: 9, local: false },
    ],
    stations: [
      { id: 'ship-a/helm', station_id: 'helm', name: 'Helm', ship: SHIP, operator: 'Morgan', state: 'live' },
    ],
    operators: [{ id: 'gm-1', name: 'Robin', state: 'live' }],
    alerts: [],
    ...extra,
  };
}

/** Mount both halves exactly as gui/gm-workspace.js wires them. */
function mount({ onSelectShip = vi.fn() } = {}) {
  document.body.innerHTML = MARKUP;
  let healthPanelRef = null;
  const banner = createGmHealthBanner({
    doc: document,
    t,
    onAction: (row) => {
      if (row.ship) { onSelectShip(row.ship); return; }
      healthPanelRef?.focus();
    },
  });
  const filters = createGmAttentionFilters();
  const attention = createGmAttentionPanel({
    doc: document,
    t,
    has,
    filters,
    renderBanners: (rows, container) => banner.render(rows, container),
    schedule: () => 0,
    cancelSchedule: () => {},
  });
  const health = createGmHealthPanel({
    doc: document,
    t,
    has,
    banners: (alerts) => attention.banners(alerts),
  });
  healthPanelRef = health;
  return { attention, health, filters, banner, onSelectShip };
}

const bannerIds = () => [...document.querySelectorAll('#gm-attention-banners [data-banner-id]')]
  .map((row) => row.dataset.bannerId);
const rowStates = (group) => [...document.querySelectorAll(`#gm-health-groups li`)]
  .filter((row) => row.dataset.rowId.startsWith(group))
  .map((row) => [row.dataset.rowId, row.querySelector('.gm-health-state').textContent]);

beforeEach(() => { lookups.length = 0; });
afterEach(() => { document.body.innerHTML = ''; });

describe('reading the projection', () => {
  it('rejects a payload that is not a health projection and keeps the last honest one', () => {
    const { health } = mount();
    expect(health.update(projection())).toBe(true);
    expect(health.state().projection.peers).toHaveLength(2);
    for (const junk of [null, 'not json', '{"peers":[]}', { peers: [] }, { alerts: [] }, 42]) {
      expect(health.update(junk)).toBe(false);
    }
    expect(health.state().projection.peers).toHaveLength(2);
  });

  it('drops a malformed row rather than rendering it as undefined', () => {
    const { health } = mount();
    health.update(projection({
      peers: [
        { id: 'ship:ship-a', ship: SHIP, operators: [], state: 'live', behind_ticks: null, local: true },
        { id: 'ship:ship-a', ship: SHIP, operators: [], state: 'live', behind_ticks: null, local: false },
        { id: 'bad-state', state: 'exploded' },
        { state: 'live' },
      ],
      alerts: [alert('a'), { id: 'no-reason', kind: 'x', severity: 'disconnected' }],
    }));
    expect(health.state().projection.peers.map((peer) => peer.id)).toEqual(['ship:ship-a']);
    expect(health.state().projection.alerts.map((row) => row.id)).toEqual(['a']);
  });

  it('reports the worst condition present, with a pause ranked above a healthy fleet', () => {
    expect(worstHealthState(parseGmHealthProjection(projection({ peers: [] })))).toBe('live');
    expect(worstHealthState(parseGmHealthProjection(projection()))).toBe('stale');
    expect(worstHealthState(parseGmHealthProjection(projection({ paused: true, peers: [] })))).toBe('paused');
    expect(worstHealthState(parseGmHealthProjection(projection({ alerts: [alert('a')] })))).toBe('disconnected');
    expect(GM_HEALTH_STATES).toEqual(['live', 'paused', 'stale', 'recovering', 'disconnected']);
  });
});

describe('the readable panel', () => {
  it('renders peers, Stations and Game Masters with their state as a WORD, not a colour', () => {
    const { health } = mount();
    health.update(projection());
    // Every state is a translated noun in the row's own text. A monochrome
    // display or forced colours removes the emphasis and keeps the meaning.
    expect(rowStates('ship:')).toEqual([
      ['ship:ship-a', t(healthStateLabelId('live'))],
      ['ship:ship-b', t(healthStateLabelId('stale'))],
    ]);
    expect(rowStates('ship-a/helm')).toEqual([['ship-a/helm', t(healthStateLabelId('live'))]]);
    expect(rowStates('gm-1')).toEqual([['gm-1', t(healthStateLabelId('live'))]]);
    // Freshness is stated in the barrier's own units, never as a wall clock.
    const kestrel = document.querySelector('li[data-row-id="ship:ship-b"] .gm-health-detail');
    expect(kestrel.textContent).toBe(t('server.gm.health.behind', { ticks: 9 }));
    expect(document.querySelector('li[data-row-id="ship:ship-a"] .gm-health-detail').textContent)
      .toBe(t('server.gm.health.this_desk'));
    // The summary names the worst condition as a resolved word, and only
    // decorates it with data-state afterwards.
    const summary = document.getElementById('gm-health-summary');
    expect(summary.dataset.state).toBe('stale');
    expect(lookups.some(([id, params]) => id === 'server.gm.health.summary'
      && params.state === healthStateLabelId('stale')
      && params.peers === 2
      && !('tick' in params))).toBe(true);
    // The sample tick is still on the desk - beside the announced line, not in it.
    expect(document.getElementById('gm-health-tick').textContent)
      .toBe(t('server.gm.health.sample_tick', { tick: 480 }));
    expect(document.getElementById('gm-health-empty').hidden).toBe(true);
  });

  it('does not re-announce its summary while only the tick and a peer lag move', () => {
    const { health } = mount();
    const summary = document.getElementById('gm-health-summary');
    const tickLine = document.getElementById('gm-health-tick');
    // What the panel last asked the String Table to render into the live
    // region. The stub `t` returns the id, so the SENTENCE is only as stable as
    // its inputs - those are what a real table would interpolate.
    const announcedWith = () => lookups
      .filter(([id]) => id === 'server.gm.health.summary')
      .map(([, params]) => ({ ...params }))
      .at(-1);
    health.update(projection());
    const announced = summary.textContent;
    const inputs = announcedWith();
    expect(announced).not.toBe('');
    expect(inputs).toBeDefined();
    expect('tick' in inputs).toBe(false);
    // Six ordinary republishes: the sample tick advances and the remote peer's
    // lag jitters, exactly as a running multi-peer fleet republishes. Nothing a
    // Game Master is being TOLD has changed, so the live region's sentence must
    // be byte-identical every time - otherwise `aria-live` re-reads the whole
    // line on every simulation tick.
    for (let step = 1; step <= 6; step += 1) {
      health.update(projection({
        tick: 480 + step,
        peers: [
          { id: 'ship:ship-a', ship: SHIP, operators: [], state: 'live', behind_ticks: null, local: true },
          {
            id: 'ship:ship-b',
            ship: { entity_id: 'ship-b', name: 'Kestrel' },
            operators: [],
            state: 'stale',
            behind_ticks: 9 + step,
            local: false,
          },
        ],
      }));
      expect(summary.textContent).toBe(announced);
      expect(announcedWith()).toEqual(inputs);
      // The tick IS still reported, just outside the announced region.
      expect(tickLine.textContent).toBe(t('server.gm.health.sample_tick', { tick: 480 + step }));
    }
    // A real change to what the panel says still rewrites the line.
    health.update(projection({ tick: 500, alerts: [alert('a')] }));
    expect(announcedWith()).not.toEqual(inputs);
    expect(announcedWith().warnings).toBe(1);
  });

  it('offers no control that could remove a peer or a Station', () => {
    const { health } = mount();
    health.update(projection({ alerts: [] }));
    // This issue observes membership; it introduces no removal policy, so the
    // panel has nothing to press.
    expect(document.querySelectorAll('#gm-health-groups button, #gm-health-groups input')).toHaveLength(0);
  });

  it('says so when it has nothing to report yet', () => {
    const { health } = mount();
    health.update(projection({ peers: [], stations: [], operators: [] }));
    expect(document.getElementById('gm-health-empty').hidden).toBe(false);
    expect(document.querySelectorAll('#gm-health-groups li')).toHaveLength(0);
  });
});

describe('the technical banner', () => {
  it('cannot be hidden by any filter, snooze or reading hold', () => {
    const { health, attention, filters } = mount();
    health.update(projection({ alerts: [alert('station-disconnected:ship-a/helm#1')] }));
    expect(bannerIds()).toEqual(['station-disconnected:ship-a/helm#1']);
    expect(document.getElementById('gm-attention-banners').hidden).toBe(false);

    // Everything an operator can do to the queue, done: narrow every filter to
    // something the technical row is not, snooze the queue's rows, and hold the
    // list for reading.
    for (const [kind, value] of [['band', 'background'], ['category', 'pending_comms'], ['ship', 'ship-b']]) {
      const select = document.getElementById(`gm-attention-filter-${kind}`);
      select.value = value;
      select.dispatchEvent(new window.Event('change'));
    }
    filters.snooze('health:station-disconnected:ship-a/helm#1', 'urgent');
    attention.update({ occurrences: [] });
    health.update(projection({ alerts: [alert('station-disconnected:ship-a/helm#1')] }));
    expect(bannerIds()).toEqual(['station-disconnected:ship-a/helm#1']);
    expect(document.getElementById('gm-attention-banners').hidden).toBe(false);
    attention.dispose();
  });

  it('is actionable: the hull it names is one keyboard press away', () => {
    const { health, onSelectShip } = mount();
    health.update(projection({ alerts: [alert('a')] }));
    const action = document.querySelector('#gm-attention-banners button[data-action="focus"]');
    expect(action).not.toBeNull();
    // A real button, in the tab ring, so Enter and Space both work.
    expect(action.type).toBe('button');
    expect(action.getAttribute('tabindex')).toBeNull();
    expect(action.textContent).toBe(t('server.gm.health.focus_ship', { ship: 'Valiant' }));
    action.click();
    expect(onSelectShip).toHaveBeenCalledWith(SHIP);
  });

  it('takes a warning with no hull of its own to the panel that explains it', () => {
    const { health } = mount();
    health.update(projection({
      recovery: { divergence_tick: 240, boundary_tick: 260, recovering_peers: 1, failed: false },
      alerts: [alert('recovery:260#1', {
        kind: 'recovery_in_progress',
        severity: 'recovering',
        reason: { id: 'server.gm.health.reason.recovery_in_progress', params: { tick: 260, peers: 1 } },
        ship: null,
        station: null,
      })],
    }));
    const banner = document.querySelector('#gm-attention-banners [data-banner-id]');
    expect(banner.dataset.state).toBe('recovering');
    // Recovering is its own word, distinct from a lost peer.
    expect(banner.querySelector('.gm-health-banner-state').textContent)
      .toBe(t(healthStateLabelId('recovering')));
    expect(banner.querySelector('.gm-health-banner-message').textContent)
      .toBe('server.gm.health.reason.recovery_in_progress');
    const action = banner.querySelector('button[data-action="focus"]');
    expect(action.textContent).toBe(t('server.gm.health.focus_panel'));
    action.click();
    expect(document.activeElement).toBe(document.getElementById('gm-health-summary'));
  });

  it('keeps the keyboard on its action while unrelated peers keep updating', () => {
    const { health } = mount();
    health.update(projection({ alerts: [alert('a')] }));
    const action = document.querySelector('#gm-attention-banners button[data-action="focus"]');
    action.focus();
    expect(document.activeElement).toBe(action);
    // Six ordinary projections: the fleet is running, watermarks are moving,
    // and none of it is news about this warning.
    for (let step = 1; step <= 6; step += 1) {
      health.update(projection({
        tick: 480 + step,
        peers: [
          { id: 'ship:ship-a', ship: SHIP, operators: [], state: 'live', behind_ticks: null, local: true },
          { id: 'ship:ship-b', ship: { entity_id: 'ship-b', name: 'Kestrel' }, operators: [], state: 'live', behind_ticks: step, local: false },
        ],
        alerts: [alert('a')],
      }));
    }
    expect(document.activeElement).toBe(action);
    expect(bannerIds()).toEqual(['a']);
  });

  it('never opens a dialog for routine attention, and clears when the condition resolves', () => {
    const alerted = vi.spyOn(window, 'alert').mockImplementation(() => {});
    const confirmed = vi.spyOn(window, 'confirm').mockImplementation(() => true);
    const { health } = mount();
    health.update(projection({ alerts: [alert('a'), alert('b', { kind: 'ship_peer_lost', station: null })] }));
    expect(bannerIds()).toEqual(['a', 'b']);
    expect(document.querySelectorAll('[role="dialog"]')).toHaveLength(0);
    expect(alerted).not.toHaveBeenCalled();
    expect(confirmed).not.toHaveBeenCalled();

    health.update(projection({ alerts: [] }));
    expect(bannerIds()).toEqual([]);
    expect(document.getElementById('gm-attention-banners').hidden).toBe(true);
    alerted.mockRestore();
    confirmed.mockRestore();
  });

  it('renders into any container, which is the seam a live restore reuses', () => {
    // #1446/#1447 mount the same component somewhere that is not the GM desk's
    // attention panel and get identical wording, state words and actions.
    document.body.innerHTML = '<div id="elsewhere" hidden></div>';
    const banner = createGmHealthBanner({ doc: document, t });
    const elsewhere = document.getElementById('elsewhere');
    expect(banner.render([alert('a')], elsewhere)).toBe(1);
    expect(elsewhere.hidden).toBe(false);
    expect(elsewhere.querySelector('[data-banner-id="a"] .gm-health-banner-state').textContent)
      .toBe(t(healthStateLabelId('disconnected')));
    banner.reset(elsewhere);
    expect(elsewhere.hidden).toBe(true);
    expect(elsewhere.children).toHaveLength(0);
  });
});

describe('the host channel boundary', () => {
  it('hands gm_health to its panel with the reason ids still raw', () => {
    const { health } = mount();
    const seen = [];
    const dispatch = createHostChannel({
      handlers: { gm_health: (payload) => { seen.push(payload); health.update(payload); } },
      strings: {
        localiseTree: () => { throw new Error('gm_health must not be localised at the dispatcher'); },
      },
    });
    dispatch('gm_health', projection({ alerts: [alert('a')] }));
    expect(seen).toHaveLength(1);
    expect(seen[0].alerts[0].reason.id).toBe('server.gm.health.reason.station_disconnected');
    expect(health.state().projection.alerts[0].reason.params.operator).toBe('Morgan');
  });

  it('carries no session token, fleet slot or transport id in the shape the page reads', () => {
    // The Rust side is tested against the encoder in tests/gm_health.rs; this
    // is the browser half of the same contract — the adapter has nowhere to
    // put such a field even if a future payload carried one.
    const { health } = mount();
    health.update(projection({
      peers: [{
        id: 'ship:ship-a', ship: SHIP, operators: ['gm-1'], state: 'live',
        behind_ticks: 0, local: true,
        slot: 2, token: 'session-token-abcdef', leg: 7,
      }],
      alerts: [alert('a')],
    }));
    const text = JSON.stringify(health.state().projection);
    for (const forbidden of ['session-token-abcdef', '"slot"', '"token"', '"leg"']) {
      expect(text).not.toContain(forbidden);
    }
    expect(text).toContain('gm-1');
  });
});

describe('the desk carries the markup both halves need', () => {
  const html = readFileSync('server.html', 'utf8');

  it('gives the health panel a labelled region and a polite status line', () => {
    expect(html).toContain('id="gm-health-panel"');
    expect(html).toContain('aria-labelledby="gm-health-heading"');
    expect(html).toContain('id="gm-health-summary"');
    // The sample tick is a SIBLING of the live region, never inside it, so a
    // routine republish is readable without being announced.
    expect(html).toContain('id="gm-health-tick"');
    expect(html.indexOf('id="gm-health-tick"')).toBeGreaterThan(html.indexOf('id="gm-health-summary"'));
    expect(html).toContain('id="gm-health-groups"');
    expect(html).toContain('id="gm-health-empty"');
  });

  it('keeps the banner region assertive and outside the filtered list', () => {
    // #gm-attention-banners is announced (role="alert"), and it is a SIBLING of
    // the list, not a row in it.
    expect(html).toMatch(/id="gm-attention-banners"[^>]*role="alert"/);
    const bannersAt = html.indexOf('id="gm-attention-banners"');
    const listAt = html.indexOf('id="gm-attention-list"');
    expect(bannersAt).toBeGreaterThan(-1);
    expect(bannersAt).toBeLessThan(listAt);
  });
});

describe('authored advisory config cannot reach the technical treatment', () => {
  it('leaves both technical regions out of the closed role-preset panel list', async () => {
    const { GM_ROLE_PRESET_PANEL_IDS } = await import('../../gui/gm-role-presets.js');
    // `[[gm_role_preset]] panels` is authored config, and the ids it can act on
    // are a closed list. Neither the queue that carries the Urgent Station rows
    // nor the health panel is on it, so no authored preset can hide either —
    // and the banner region lives inside the queue.
    expect(GM_ROLE_PRESET_PANEL_IDS).not.toContain('gm-attention-panel');
    expect(GM_ROLE_PRESET_PANEL_IDS).not.toContain('gm-health-panel');
  });

  it('wires the desk so the queue owns the region and the health component owns the drawing', () => {
    const workspace = readFileSync('gui/gm-workspace.js', 'utf8');
    expect(workspace).toContain('renderBanners: (rows, container) => gmHealthBanner.render(rows, container)');
    expect(workspace).toContain('banners: (alerts) => gmAttentionPanel.banners(alerts)');
    // The live restore (issue #1446) reads the SAME projection rather than
    // opening a second channel for its own state.
    expect(workspace).toContain(
      'gm_health:    function(p) { gmHealthPanel.update(p); gmRestoreControl.update(p); }');
    // The banner's hull action is the SAME selection the map already answers to.
    expect(workspace).toContain('gmProjection.select(alert.ship.entity_id)');
  });
});
