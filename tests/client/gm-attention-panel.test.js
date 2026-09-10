// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import {
  createGmAttentionPanel,
  formatAttentionAge,
  parseGmAttentionProjection,
} from '../../gui/gm-attention-panel.js';
import {
  GM_ATTENTION_SNOOZE_MS,
  createGmAttentionFilters,
} from '../../gui/gm-attention-filters.js';
import { createHostChannel } from '../../gui/host-channel.js';
import { createGmCommsPanel } from '../../gui/gm-comms-panel.js';

/** The exact ids server.html carries for the panel. */
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
  </section>`;

const t = (id, params = {}) => Object.entries(params)
  .reduce((text, [key, value]) => text.replaceAll(`{${key}}`, String(value)), id);

function occurrence(id, extra = {}) {
  return {
    id,
    band: 'attention',
    category: 'pending_comms',
    first_seen_tick: 100,
    age_ms: 0,
    reason: { id: 'server.gm.attention.reason.pending_comms', params: { sender: 'Axiom', ship: 'Alpha' } },
    target: {
      route: 'default-route',
      ship: { entity_id: 'ship-a', name: 'Alpha' },
      sender: { entity_id: 'speaker', name: 'Axiom' },
      conversation: `thread-${id}`,
    },
    ...extra,
  };
}

const payload = (...rows) => ({ occurrences: rows });

function memoryStorage() {
  const map = new Map();
  return {
    getItem: (key) => (map.has(key) ? map.get(key) : null),
    setItem: (key, value) => map.set(key, String(value)),
    dump: () => [...map.entries()],
  };
}

function mount({ filters, onOpen = vi.fn(), doc = document } = {}) {
  doc.body.innerHTML = MARKUP;
  const panel = createGmAttentionPanel({
    doc,
    t,
    has: (id) => typeof id === 'string' && id.startsWith('server.'),
    filters: filters || createGmAttentionFilters(),
    onOpen,
  });
  return { panel, onOpen };
}

const rowIds = (doc = document) => [...doc.querySelectorAll('#gm-attention-list li')]
  .map((row) => row.dataset.occurrenceId);
const bandOf = (id, doc = document) => doc
  .querySelector(`#gm-attention-list li[data-occurrence-id="${id}"]`)?.dataset.band;

beforeEach(() => { vi.useFakeTimers(); vi.setSystemTime(new Date('2026-09-10T10:00:00Z')); });
afterEach(() => { vi.useRealTimers(); });

describe('GM attention projection parsing', () => {
  it('drops malformed rows and refuses a payload that is not a queue', () => {
    expect(parseGmAttentionProjection('not json')).toBe(null);
    expect(parseGmAttentionProjection({ rows: [] })).toBe(null);
    const parsed = parseGmAttentionProjection(JSON.stringify(payload(
      occurrence('a'),
      occurrence('a'),
      occurrence('b', { band: 'critical' }),
      occurrence('c', { first_seen_tick: -1 }),
      { id: 'd' },
      occurrence('e', { target: {} }),
    )));
    expect(parsed.occurrences.map((row) => row.id)).toEqual(['a', 'e']);
    expect(parsed.occurrences[1].target).toEqual({ route: null, ship: null, sender: null, conversation: null });
  });

  it('reaches the panel through the host channel with its String Table ids intact', () => {
    const { panel } = mount();
    const dispatch = createHostChannel({
      handlers: { gm_attention: (p) => panel.update(p) },
      strings: { localiseTree: () => ({ occurrences: [] }) },
    });
    dispatch('gm_attention', JSON.stringify(payload(occurrence('comms:m1'))));
    // A localised payload would have arrived empty; the raw one carries the row.
    expect(rowIds()).toEqual(['comms:m1']);
    expect(document.querySelector('#gm-attention-list li p').textContent)
      .toBe('server.gm.attention.reason.pending_comms');
    panel.dispose();
  });
});

describe('bands, ordering and reading stability', () => {
  it('groups three bands and orders each oldest first with a stable-id tie-break', () => {
    const { panel } = mount();
    panel.update(payload(
      occurrence('comms:z', { band: 'urgent', age_ms: 5000 }),
      occurrence('comms:a', { band: 'urgent', age_ms: 5000 }),
      occurrence('comms:old', { band: 'background', age_ms: 60000 }),
      occurrence('comms:new', { band: 'attention', age_ms: 1000 }),
    ));
    const groups = [...document.querySelectorAll('.gm-attention-band-group')];
    expect(groups.map((group) => group.dataset.band)).toEqual(['urgent', 'attention', 'background']);
    expect(groups.map((group) => group.hidden)).toEqual([false, false, false]);
    // Same age: the id decides, so two peers and two repaints agree.
    expect([...groups[0].querySelectorAll('li')].map((row) => row.dataset.occurrenceId))
      .toEqual(['comms:a', 'comms:z']);
    // Ages come from the projection's real-time basis, not from arrival order.
    expect(document.querySelector('li[data-occurrence-id="comms:old"] .gm-attention-age').textContent)
      .toBe(t('server.gm.attention.age', { clock: '1:00' }));
    expect(formatAttentionAge(65000)).toBe('1:05');
    panel.dispose();
  });

  it('holds rows under focus, counts arrivals, and applies them on Return to live', () => {
    const { panel } = mount();
    panel.update(payload(occurrence('comms:1', { age_ms: 9000 }), occurrence('comms:2', { age_ms: 8000 })));
    expect(rowIds()).toEqual(['comms:1', 'comms:2']);
    document.querySelector('li[data-occurrence-id="comms:2"] button[data-action="snooze"]').focus();

    // An arrival that would sort ABOVE both rows, plus an escalation, while the
    // operator's finger is on a control.
    panel.update(payload(
      occurrence('comms:0', { age_ms: 20000 }),
      occurrence('comms:1', { age_ms: 9000, band: 'urgent' }),
      occurrence('comms:2', { age_ms: 8000 }),
    ));
    expect(panel.state().held).toBe(true);
    expect(rowIds()).toEqual(['comms:1', 'comms:2']);
    expect(document.activeElement.dataset.action).toBe('snooze');
    expect(document.activeElement.closest('li').dataset.occurrenceId).toBe('comms:2');
    expect(panel.state().newCount).toBe(1);
    expect(document.getElementById('gm-attention-status').textContent)
      .toBe(t('server.gm.attention.held', { count: 1 }));
    expect(document.getElementById('gm-attention-panel').dataset.freshness).toBe('held');
    expect(document.getElementById('gm-attention-live').hidden).toBe(false);
    // A held row may still tell the truth about its own band.
    expect(bandOf('comms:1')).toBe('urgent');

    document.getElementById('gm-attention-live').click();
    expect(panel.state().held).toBe(false);
    // Live again: comms:1 is now in the Urgent group above the two Attention
    // rows, which are themselves still oldest first.
    expect(rowIds()).toEqual(['comms:1', 'comms:0', 'comms:2']);
    expect(panel.state().newCount).toBe(0);
    expect(document.getElementById('gm-attention-panel').dataset.freshness).toBe('live');
    expect(document.getElementById('gm-attention-status').textContent)
      .toBe(t('server.gm.attention.live', { count: 3 }));
    panel.dispose();
  });

  it('gives Return to live somewhere to land when the hold was entered by focus alone', () => {
    const { panel } = mount();
    panel.update(payload(occurrence('comms:1', { age_ms: 9000 })));
    // Held by focus, with no row opened — so there is no selection to return
    // to, and the only keyboard way out is the button that is about to vanish.
    document.querySelector('li[data-occurrence-id="comms:1"] button[data-action="snooze"]').focus();
    panel.update(payload(
      occurrence('comms:0', { age_ms: 20000 }),
      occurrence('comms:1', { age_ms: 9000 }),
    ));
    expect(panel.state().held).toBe(true);
    expect(panel.state().selectedId).toBe(null);

    const liveButton = document.getElementById('gm-attention-live');
    liveButton.focus();
    expect(document.activeElement).toBe(liveButton);
    liveButton.click();
    expect(rowIds()).toEqual(['comms:0', 'comms:1']);
    expect(liveButton.hidden).toBe(true);
    // Catching up hides the control the operator was standing on; focus must
    // not be left on it, nor dropped to the document.
    expect(document.activeElement).not.toBe(liveButton);
    expect(document.getElementById('gm-attention-panel').contains(document.activeElement)).toBe(true);
    expect(document.activeElement.dataset.action).toBe('open');
    expect(document.activeElement.closest('li').dataset.occurrenceId).toBe('comms:0');
    panel.dispose();
  });

  it('opens a row through its existing conversation route without holding a command', () => {
    const onOpen = vi.fn();
    const { panel } = mount({ onOpen });
    panel.update(payload(occurrence('comms:1')));
    document.querySelector('li button[data-action="open"]').click();
    expect(onOpen).toHaveBeenCalledTimes(1);
    expect(onOpen.mock.calls[0][0].target).toMatchObject({ route: 'default-route', conversation: 'thread-comms:1' });
    // Reading holds the list, and an arrival cannot move the row being read.
    expect(panel.state().held).toBe(true);
    expect(panel.state().selectedId).toBe('comms:1');
    panel.update(payload(occurrence('comms:0', { age_ms: 60000 }), occurrence('comms:1')));
    expect(rowIds()).toEqual(['comms:1']);
    panel.dispose();
  });

  it('drops a resolved occurrence and shows a recurrence as a new row', () => {
    const { panel } = mount();
    panel.update(payload(occurrence('comms:m1')));
    expect(rowIds()).toEqual(['comms:m1']);
    panel.update(payload());
    expect(rowIds()).toEqual([]);
    expect(document.getElementById('gm-attention-empty').hidden).toBe(false);
    panel.update(payload(occurrence('comms:m2')));
    expect(rowIds()).toEqual(['comms:m2']);
    panel.dispose();
  });

  it('says so, in words, when a held row resolves under the operator', () => {
    const filters = createGmAttentionFilters();
    const { panel, onOpen } = mount({ filters });
    panel.update(payload(occurrence('comms:1', { age_ms: 252000 }), occurrence('comms:2', { age_ms: 8000 })));
    const waited = t('server.gm.attention.age', { clock: '4:12' });
    const row = () => document.querySelector('li[data-occurrence-id="comms:1"]');
    expect(row().querySelector('.gm-attention-age').textContent).toBe(waited);

    // The operator's finger is on the row's own control, so the list holds —
    // and then the crew answers that very conversation.
    row().querySelector('button[data-action="open"]').focus();
    panel.update(payload(occurrence('comms:2', { age_ms: 8000 })));
    expect(panel.state().held).toBe(true);
    expect(rowIds()).toEqual(['comms:1', 'comms:2']);

    // The wait it ended on, frozen — not a 0:00 that reads as a new arrival.
    expect(row().querySelector('.gm-attention-age').textContent).toBe(waited);
    vi.advanceTimersByTime(5000);
    expect(row().querySelector('.gm-attention-age').textContent).toBe(waited);
    // Status as a sentence, not a colour, and focus kept inside the list.
    expect(row().dataset.stale).toBe('true');
    expect(row().querySelector('.gm-attention-resolved').textContent)
      .toBe('server.gm.attention.resolved');
    expect(document.activeElement).toBe(row().querySelector('.gm-attention-resolved'));

    // Neither verb is left as a silent no-op.
    const open = row().querySelector('button[data-action="open"]');
    const snooze = row().querySelector('button[data-action="snooze"]');
    expect([open.disabled, snooze.disabled]).toEqual([true, true]);
    open.click();
    snooze.click();
    expect(onOpen).not.toHaveBeenCalled();
    expect(panel.state().selectedId).toBe(null);
    expect(filters.isSnoozed('comms:1')).toBe(false);

    // The still-waiting row went on ageing all along, and Return to live drops
    // the resolved one for good.
    expect(document.querySelector('li[data-occurrence-id="comms:2"] .gm-attention-age').textContent)
      .toBe(t('server.gm.attention.age', { clock: '0:13' }));
    document.getElementById('gm-attention-live').click();
    expect(rowIds()).toEqual(['comms:2']);
    panel.dispose();
  });

  it('ends the reading session on Return to live, so the queue keeps catching up', () => {
    const { panel } = mount();
    panel.update(payload(occurrence('comms:1', { age_ms: 5000 })));
    document.querySelector('li[data-occurrence-id="comms:1"] button[data-action="open"]').click();
    expect(panel.state().held).toBe(true);
    expect(panel.state().selectedId).toBe('comms:1');
    expect(document.querySelector('#gm-attention-list li[data-selected]')).not.toBe(null);

    document.getElementById('gm-attention-live').click();
    expect(panel.state().held).toBe(false);
    expect(panel.state().selectedId).toBe(null);
    expect(document.querySelector('#gm-attention-list li[data-selected]')).toBe(null);

    // The next projection arrives with nobody reading it, so it must land live
    // rather than freeze the list again on the strength of a finished Open.
    panel.update(payload(occurrence('comms:1', { age_ms: 8000 }), occurrence('comms:2', { age_ms: 1000 })));
    expect(panel.state().held).toBe(false);
    expect(rowIds()).toEqual(['comms:1', 'comms:2']);
    expect(panel.state().newCount).toBe(0);
    expect(document.getElementById('gm-attention-live').hidden).toBe(true);
    panel.dispose();
  });
});

describe('opening a row', () => {
  it('lands on the authored Comms route that already exists on the desk', () => {
    document.body.innerHTML = `${MARKUP}
      <section id="gm-comms-panel"><select id="gm-comms-route"></select>
        <select id="gm-comms-sender"></select><select id="gm-comms-recipients" multiple></select>
        <textarea id="gm-comms-text"></textarea><select id="gm-comms-hail"></select>
        <button id="gm-comms-send"></button><button id="gm-comms-start-hail"></button>
        <p id="gm-comms-feedback"></p><p id="gm-comms-count"></p><p id="gm-comms-empty"></p>
        <ol id="gm-comms-log"></ol></section>`;
    const submitTransmission = vi.fn(() => true);
    const comms = createGmCommsPanel({ doc: document, t, submitTransmission,
      getOperator: () => ({ id: 'gm-a' }), schedule: vi.fn(), cancelSchedule: vi.fn() });
    comms.update({
      routes: [
        { id: 'other-route', label: 'Other', visibility: 'fleet', senders: [], hails: [] },
        { id: 'default-route', label: 'Default', visibility: 'selected_ships',
          senders: [{ id: 'speaker', name: 'Axiom' }], hails: [] },
      ],
      recipients: [{ id: 'ship-a', name: 'Alpha' }], results: [], max_text_bytes: 64,
    });
    const selectShip = vi.fn();
    const panel = createGmAttentionPanel({
      doc: document, t, has: () => false, filters: createGmAttentionFilters(),
      onOpen: (occurrence) => {
        selectShip(occurrence.target.ship.entity_id);
        comms.focusRoute(occurrence.target.route);
      },
    });
    panel.update(payload(occurrence('comms:1')));
    document.querySelector('#gm-attention-list li button[data-action="open"]').click();
    expect(selectShip).toHaveBeenCalledWith('ship-a');
    expect(document.getElementById('gm-comms-route').value).toBe('default-route');
    expect(document.activeElement.id).toBe('gm-comms-route');
    // Navigation only. Nothing was transmitted, and no route the projection
    // does not hold could be selected.
    expect(submitTransmission).not.toHaveBeenCalled();
    expect(comms.focusRoute('invented-route')).toBe(false);
    expect(document.getElementById('gm-comms-route').value).toBe('default-route');
    panel.dispose();
  });
});

describe('personal snooze', () => {
  it('is exactly one real minute, keeps running while the simulation is paused, and needs no picker', () => {
    const { panel } = mount();
    panel.update(payload(occurrence('comms:1', { age_ms: 5000 }), occurrence('comms:2', { age_ms: 1000 })));
    expect(rowIds()).toEqual(['comms:1', 'comms:2']);
    // One click. No picker, no menu, no duration control anywhere on the row.
    const row = document.querySelector('li[data-occurrence-id="comms:1"]');
    expect([...row.querySelectorAll('button')].map((button) => button.dataset.action))
      .toEqual(['open', 'snooze']);
    row.querySelector('button[data-action="snooze"]').click();
    expect(rowIds()).toEqual(['comms:2']);

    // No projection arrives at all — a paused world publishes nothing new —
    // and the minute still runs down on the panel's own real clock.
    vi.advanceTimersByTime(GM_ATTENTION_SNOOZE_MS - 1000);
    expect(rowIds()).toEqual(['comms:2']);
    vi.advanceTimersByTime(1000);
    expect(rowIds()).toEqual(['comms:1', 'comms:2']);
    panel.dispose();
  });

  it('leaves the keyboard inside the panel when the row being stood on is snoozed', () => {
    const { panel } = mount();
    panel.update(payload(
      occurrence('comms:1', { age_ms: 9000 }),
      occurrence('comms:2', { age_ms: 8000 }),
    ));
    // Live, not held: the common case — a quiet moment with no projection in
    // flight and a Game Master working down the queue on the keyboard.
    expect(panel.state().held).toBe(false);
    const first = document.querySelector('li[data-occurrence-id="comms:1"] button[data-action="snooze"]');
    first.focus();
    first.click();
    expect(rowIds()).toEqual(['comms:2']);
    const panelEl = document.getElementById('gm-attention-panel');
    expect(panelEl.contains(document.activeElement)).toBe(true);
    expect(document.activeElement.dataset.action).toBe('snooze');
    expect(document.activeElement.closest('li').dataset.occurrenceId).toBe('comms:2');

    // The last row goes too: focus lands on the panel's own status sentence
    // rather than on the document, so the next Tab resumes from here.
    document.activeElement.click();
    expect(rowIds()).toEqual([]);
    expect(document.activeElement).toBe(document.getElementById('gm-attention-status'));
    panel.dispose();
  });

  it('keeps that rescue when the desk repaints this panel on every filter change', () => {
    // The wiring the GM workspace actually ships: the controller reports every
    // change back into this panel's repaint, and a snooze IS such a change — so
    // the repaint arrives in the middle of the click that made it, before the
    // handler has finished acting on the screen it was reading.
    let repaint = () => {};
    const filters = createGmAttentionFilters({ onChange: () => repaint() });
    const { panel } = mount({ filters });
    repaint = panel.repaint;
    panel.update(payload(
      occurrence('comms:1', { age_ms: 9000 }),
      occurrence('comms:2', { age_ms: 8000 }),
    ));
    expect(panel.state().held).toBe(false);
    const first = document.querySelector('li[data-occurrence-id="comms:1"] button[data-action="snooze"]');
    first.focus();
    first.click();
    expect(rowIds()).toEqual(['comms:2']);
    const panelEl = document.getElementById('gm-attention-panel');
    expect(panelEl.contains(document.activeElement)).toBe(true);
    expect(document.activeElement.dataset.action).toBe('snooze');
    expect(document.activeElement.closest('li').dataset.occurrenceId).toBe('comms:2');
    panel.dispose();
  });

  it('breaks on escalation to Urgent but not for a row that was already Urgent', () => {
    const filters = createGmAttentionFilters({ now: () => Date.now() });
    const { panel } = mount({ filters });
    panel.update(payload(
      occurrence('comms:calm'),
      occurrence('comms:loud', { band: 'urgent', age_ms: 1000 }),
    ));
    for (const id of ['comms:calm', 'comms:loud']) {
      document.querySelector(`li[data-occurrence-id="${id}"] button[data-action="snooze"]`).click();
    }
    expect(rowIds()).toEqual([]);

    vi.advanceTimersByTime(10000);
    // The calm row escalates: that is new information the snooze never covered.
    panel.update(payload(
      occurrence('comms:calm', { band: 'urgent' }),
      occurrence('comms:loud', { band: 'urgent', age_ms: 11000 }),
    ));
    expect(rowIds()).toEqual(['comms:calm']);
    // The already-Urgent row waits out its whole minute.
    expect(filters.snoozeRemainingMs('comms:loud')).toBe(GM_ATTENTION_SNOOZE_MS - 10000);
    vi.advanceTimersByTime(GM_ATTENTION_SNOOZE_MS - 10000);
    panel.refresh();
    // Both Urgent now, and `comms:loud` was already a second old when
    // `comms:calm` appeared, so it sits above it.
    expect(rowIds()).toEqual(['comms:loud', 'comms:calm']);
    panel.dispose();
  });
});

describe('private filters and their session scope', () => {
  it('narrows by band, kind and ship without ever hiding a technical banner', () => {
    const { panel } = mount();
    panel.update(payload(
      occurrence('comms:a', { band: 'urgent' }),
      occurrence('comms:b', {
        band: 'background',
        target: { route: 'r', ship: { entity_id: 'ship-b', name: 'Beta' }, sender: null, conversation: null },
      }),
    ));
    panel.banners([{ id: 'peer-lost', message_id: 'server.gm.shell.disconnected', params: {} }]);
    const band = document.getElementById('gm-attention-filter-band');
    expect([...band.options].map((option) => option.value))
      .toEqual(['all', 'urgent', 'attention', 'background']);
    band.value = 'urgent';
    band.dispatchEvent(new window.Event('change'));
    expect(rowIds()).toEqual(['comms:a']);

    const ship = document.getElementById('gm-attention-filter-ship');
    expect([...ship.options].map((option) => option.value)).toEqual(['all', 'ship-a', 'ship-b']);
    band.value = 'all'; band.dispatchEvent(new window.Event('change'));
    ship.value = 'ship-b'; ship.dispatchEvent(new window.Event('change'));
    expect(rowIds()).toEqual(['comms:b']);

    const kind = document.getElementById('gm-attention-filter-category');
    kind.value = 'pending_comms'; kind.dispatchEvent(new window.Event('change'));
    expect(rowIds()).toEqual(['comms:b']);

    // Nothing an operator can do to the queue reaches the banner region.
    expect([...document.querySelectorAll('#gm-attention-banners p')].map((row) => row.dataset.bannerId))
      .toEqual(['peer-lost']);
    expect(document.getElementById('gm-attention-banners').hidden).toBe(false);
    panel.dispose();
  });

  it('keeps two Game Masters isolated in one browser and resets for a new session', () => {
    const storage = memoryStorage();
    let session = 'server-code-1';
    const filtersFor = (operator) => createGmAttentionFilters({
      storage, getSessionId: () => session, getOperatorId: () => operator,
    });
    const alpha = filtersFor('gm-a');
    const beta = filtersFor('gm-b');
    alpha.restore(); beta.restore();
    alpha.setFilter('band', 'urgent');
    alpha.snooze('comms:1', 'attention');
    expect(alpha.visible(occurrence('comms:1'))).toBe(false);
    // The other Game Master's queue is untouched: no shared filter, no shared
    // snooze, and no way for one to hide a row from the other.
    expect(beta.visible(occurrence('comms:1'))).toBe(true);
    expect(beta.filters().band).toBe('all');

    // Same session, same operator, fresh controller = a reconnect. The filter
    // comes back and the snooze comes back with its REMAINING time.
    vi.advanceTimersByTime(20000);
    const reconnected = filtersFor('gm-a');
    reconnected.restore();
    expect(reconnected.filters().band).toBe('urgent');
    expect(reconnected.snoozeRemainingMs('comms:1')).toBe(GM_ATTENTION_SNOOZE_MS - 20000);

    // A new session resets both, and drops the old session's stored scopes.
    session = 'server-code-2';
    const fresh = filtersFor('gm-a');
    fresh.restore();
    expect(fresh.filters().band).toBe('all');
    expect(fresh.snoozeRemainingMs('comms:1')).toBe(0);
    fresh.setFilter('ship', 'ship-a');
    const scopes = JSON.parse(storage.getItem('phoenix.gm.attention.v1')).scopes;
    expect(Object.keys(scopes)).toEqual(['server-code-2|gm-a']);
    expect(scopes['server-code-2|gm-a'].session).toBe('server-code-2');
  });

  it('never persists anything before a session exists', () => {
    const storage = memoryStorage();
    const filters = createGmAttentionFilters({ storage, getSessionId: () => null, getOperatorId: () => 'gm-a' });
    filters.restore();
    filters.setFilter('band', 'urgent');
    filters.snooze('comms:1', 'attention');
    expect(storage.getItem('phoenix.gm.attention.v1')).toBe(null);
    // It is still live for this browser, it simply has nothing to survive with.
    expect(filters.filters().band).toBe('urgent');
    expect(filters.isSnoozed('comms:1')).toBe(true);
  });

  it('repaints a reconnected desk when the restored session brings its filters and snoozes back', () => {
    const storage = memoryStorage();
    const scope = { storage, getSessionId: () => 'server-code-1', getOperatorId: () => 'gm-a' };
    // What this operator left behind earlier in the SAME session.
    const earlier = createGmAttentionFilters(scope);
    earlier.restore();
    earlier.setFilter('band', 'urgent');
    earlier.snooze('comms:2', 'urgent');

    // The reconnect. Panel and controller are wired the way the workspace wires
    // them: the panel is built FROM the controller, so its repaint is resolved
    // lazily rather than named at construction time.
    let repaint = () => {};
    const filters = createGmAttentionFilters({ ...scope, onChange: () => repaint() });
    const { panel } = mount({ filters });
    repaint = panel.repaint;
    panel.update(payload(
      occurrence('comms:1', { band: 'urgent', age_ms: 5000 }),
      occurrence('comms:2', { band: 'urgent', age_ms: 4000 }),
      occurrence('comms:3', { age_ms: 3000 }),
    ));
    expect(rowIds()).toEqual(['comms:1', 'comms:2', 'comms:3']);
    expect(document.getElementById('gm-attention-filter-band').value).toBe('all');

    // The session identity resolves after the desk is already up, and no new
    // projection follows it — a quiet or paused world publishes nothing. The
    // queue must still be the one this operator narrowed: band back to Urgent,
    // and the row they snoozed still gone with its minute still running.
    filters.restore();
    expect(document.getElementById('gm-attention-filter-band').value).toBe('urgent');
    expect(rowIds()).toEqual(['comms:1']);
    expect(filters.snoozeRemainingMs('comms:2')).toBe(GM_ATTENTION_SNOOZE_MS);
    panel.dispose();
  });

  it('gives the GM workspace that same repaint, so a real reconnect is not left stale', () => {
    // gm-workspace.js is the page module (it wires every panel to `window`), so
    // the wiring itself is read rather than mounted; the behaviour it produces
    // is the test above.
    const workspace = readFileSync('gui/gm-workspace.js', 'utf8');
    expect(workspace).toContain('onChange: () => repaintGmAttention()');
    expect(workspace).toContain('repaintGmAttention = gmAttentionPanel.repaint;');
  });
});
