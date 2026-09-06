// @vitest-environment jsdom
/**
 * tests/client/comms-console.test.js — the two Comms consoles on one
 * renderer (issue #1235, T4.C3 chunk 3 — final chunk of the console-seam
 * programme).
 *
 * Only the battleship and cruiser mount a dedicated Comms Station (the
 * destroyer and courier have none). Each hull's `.html` imports its
 * `renderStation` from `gui/<class>/comms.console.js`; this suite imports
 * the SAME functions and drives them against a jsdom fixture, so the
 * contact-list/hail-list/current-message contract is
 * asserted per hull without a browser.
 *
 * The custom-element modules are deliberately NOT imported: an un-upgraded
 * `<ph-*>` element is a plain `HTMLUnknownElement`, so assigning `.state`
 * stores a readable property and we assert the exact object the console
 * pushed, with no shadow-DOM machinery in the way.
 */
import { describe, it, expect, beforeEach } from 'vitest';
import { t } from '../../gui/strings.js';
import { renderStation as battleshipRender } from '../../gui/battleship/comms.console.js';
import { renderStation as rawCruiserRender } from '../../gui/cruiser/comms.console.js';
import { sortedThreadsFrom } from '../../gui/comms-state.js';
import { withConsoleFamilyProjection } from './console-family-fixture.js';

const cruiserRender = (payload, doc) => rawCruiserRender(withConsoleFamilyProjection(payload), doc);

function mount(markup) {
  document.body.innerHTML = markup;
}
const el = (id) => document.getElementById(id);

const FIXTURES = {
  battleship:
    '<ph-comms-contact-list id="comms-contact-list"></ph-comms-contact-list>' +
    '<ph-comms-hail-list id="comms-hail-list"></ph-comms-hail-list>' +
    '<div class="overlay-panel" id="comms-thread-panel">' +
    '<ph-comms-current-message id="comms-current-message"></ph-comms-current-message>' +
    '</div>' +
    '<span id="comms-hails-unread" hidden></span>' +
    '<span id="footer-target"></span>' +
    '<span id="comms-auto-badge" hidden></span>',
  cruiser:
    '<div id="nav-view" hidden></div>' +
    '<div id="comms-view" hidden></div>' +
    '<ph-comms-contact-list id="comms-contact-list"></ph-comms-contact-list>' +
    '<ph-comms-hail-list id="comms-hail-list"></ph-comms-hail-list>' +
    '<div class="overlay-panel" id="comms-thread-panel">' +
    '<ph-comms-current-message id="comms-current-message"></ph-comms-current-message>' +
    '</div>' +
    '<span id="comms-hails-unread" hidden></span>' +
    '<ph-navigation-map id="navigation-map"></ph-navigation-map>' +
    '<ph-civilian-traffic id="civilian-traffic"></ph-civilian-traffic>' +
    '<ph-objective-list id="objective-list"></ph-objective-list>' +
    '<span id="nav-contact-count"></span>' +
    '<span id="waypoint-name"></span>' +
    '<button id="btn-on-screen"></button>' +
    '<span id="navigation-auto-badge" hidden></span>' +
    '<span id="footer-target"></span>' +
    '<span id="footer-right"></span>' +
    '<span id="comms-auto-badge" hidden></span>',
};

// ── Battleship: the reference hull, flat `comms` family ───────────────────────
describe('battleship comms renderStation', () => {
  beforeEach(() => {
    battleshipRender.resetSelection();
    mount(FIXTURES.battleship);
  });

  const base = {
    contacts: [{ id: 'c1' }],
    messages: [{ id: 'm1', is_read: true, sender_name: 'Old' }, { id: 'm2', is_read: false, sender_name: 'Ops' }],
    rejection: null,
    own_hull: { pct: 0.9 },
    comms_auto: true,
  };

  it('drives the contact list, hail list and current message from the flat payload', () => {
    battleshipRender(base, document);
    expect(el('comms-contact-list').state).toEqual({ contacts: [{ id: 'c1' }] });
    expect(el('comms-hail-list').state).toEqual({
      ...base,
      threads: sortedThreadsFrom(base.messages, base.contacts),
      selected_thread_id: 'm2',
    });
    // The thread panel gets the CONVERSATION, not the whole inbox: these two
    // messages are two one-message threads, so the open one carries just its own.
    expect(el('comms-current-message').state).toEqual({
      thread: { id: 'm2', is_read: false, sender_name: 'Ops' },
      messages: [base.messages[1]],
      sender_name: 'Ops',
      rejection: null,
    });
  });

  // Issue #1380 AC1: a five-message conversation is one row showing the latest.
  it('folds a five-message conversation into one row and opens its whole history', () => {
    const conversation = ['a', 'b', 'c', 'd', 'e'].map((suffix, i) => ({
      id: `m-${suffix}`, thread_id: 'relay', sender_name: 'Relay Seven',
      body: `Line ${i + 1}`, is_read: i < 4,
    }));
    battleshipRender({ ...base, messages: conversation }, document);
    const threads = el('comms-hail-list').state.threads;
    expect(threads).toHaveLength(1);
    expect(threads[0]).toMatchObject({
      thread_id: 'relay', message_count: 5, subject: 'Line 5', any_unread: true,
    });
    expect(el('comms-current-message').state.messages.map((m) => m.id))
      .toEqual(['m-a', 'm-b', 'm-c', 'm-d', 'm-e']);
    expect(el('footer-target').textContent).toBe('Relay Seven');
  });

  it('counts the threads still carrying unread traffic on the HAILS tab', () => {
    battleshipRender(base, document);
    expect(el('comms-hails-unread').hidden).toBe(false);
    expect(el('comms-hails-unread').textContent).toBe('1');
    battleshipRender({
      ...base,
      messages: base.messages.map((m) => ({ ...m, is_read: true })),
    }, document);
    expect(el('comms-hails-unread').hidden).toBe(true);
    expect(el('comms-hails-unread').textContent).toBe('');
  });

  it('owns local thread selection and repaints list, thread and readout together', () => {
    battleshipRender(base, document);
    expect(battleshipRender.selectThread(base, 'm1', document)).toBe(true);
    expect(el('comms-hail-list').state.selected_thread_id).toBe('m1');
    expect(el('comms-current-message').state.thread.id).toBe('m1');
    expect(el('footer-target').textContent).toBe('Old');
    // No `data-tab-code`, so the shell's Station Bar never advertises it.
    expect(document.getElementById('comms-thread-panel').hasAttribute('data-tab-code'))
      .toBe(false);
    expect(battleshipRender.selectThread(base, 'missing', document)).toBe(false);
    expect(el('comms-current-message').state.thread.id).toBe('m1');
  });

  // `.overlay-panel.open` is the SHELL-FACING fact, not a local style hook:
  // console-core reads it back out of the DOM and posts it to the Station Bar
  // as this console's open overlay, and the bar then gives the seat's own tab
  // its "come back to the console" meaning instead of opening the per-system
  // damage popup (issue #1374). The panel only earns that while it is really
  // covering the console — which the stylesheet decides (absolute in portrait,
  // an ordinary column otherwise) and which the renderer reads back as "taken
  // out of flow". jsdom computes `static` for the plain fixture node, i.e. the
  // landscape/desktop column.
  it('marks the thread panel open only where the stylesheet makes it an overlay', () => {
    const panel = document.getElementById('comms-thread-panel');
    battleshipRender(base, document);
    expect(panel.classList.contains('open')).toBe(false);

    // A column member: selected and rendered, but NOT reported as an overlay.
    expect(battleshipRender.selectThread(base, 'm1', document)).toBe(true);
    expect(el('comms-current-message').state.thread.id).toBe('m1');
    expect(panel.classList.contains('open')).toBe(false);
    expect(battleshipRender.selectMessage(base, 'm2', document)).toBe(true);
    expect(panel.classList.contains('open')).toBe(false);

    // Phone portrait: the same node, positioned over the list, IS an overlay.
    panel.style.position = 'absolute';
    expect(battleshipRender.selectThread(base, 'm1', document)).toBe(true);
    expect(panel.classList.contains('open')).toBe(true);

    // Back to a column — a rotation — and the next selection clears the stale
    // mark, so the bar is never left holding an overlay that covers nothing.
    panel.style.position = '';
    expect(battleshipRender.selectMessage(base, 'm2', document)).toBe(true);
    expect(panel.classList.contains('open')).toBe(false);
  });

  // A thread-grain pick must stay thread-grain. A scripted follow-up arrives as
  // a NEW message on the SAME thread_id (`CommsMessage::injected`), and it does
  // not raise a new inbox row — it folds into the row already highlighted. So
  // if tapping the row pinned the message that was active at tap time, the
  // operator would be left with the answered message's disabled responses and
  // no cue that the reply they are owed had become unreachable.
  it('follows a same-thread follow-up after the tapped message is answered', () => {
    const opening = {
      id: 'm1', thread_id: 'relay', sender_name: 'Relay Seven', body: 'Do you copy?',
      is_read: true, selected_response: null,
      responses: [{ text: 'Acknowledge', available: true }],
    };
    const before = { ...base, messages: [opening] };
    battleshipRender(before, document);
    expect(battleshipRender.selectThread(before, 'relay', document)).toBe(true);
    expect(el('comms-current-message').state.thread.id).toBe('m1');

    // The operator answers m1; the script injects m2 into the same thread.
    const followUp = {
      id: 'm2', thread_id: 'relay', sender_name: 'Relay Seven', body: 'Then hold station.',
      is_read: false, selected_response: null,
      responses: [{ text: 'Holding', available: true }],
    };
    const after = {
      ...base,
      messages: [{ ...opening, selected_response: 0 }, followUp],
    };
    battleshipRender(after, document);
    expect(el('comms-hail-list').state.selected_thread_id).toBe('relay');
    expect(el('comms-current-message').state.thread.id).toBe('m2');
    expect(battleshipRender.currentMessage(after).id).toBe('m2');

    // An explicit MESSAGE-grain pick still wins and still pins.
    expect(battleshipRender.selectMessage(after, 'm1', document)).toBe(true);
    expect(battleshipRender.currentMessage(after).id).toBe('m1');
  });

  it('owns local message selection and moves the open thread with it', () => {
    battleshipRender(base, document);
    expect(battleshipRender.selectMessage(base, 'm1', document)).toBe(true);
    expect(el('comms-hail-list').state.selected_thread_id).toBe('m1');
    expect(el('comms-current-message').state.thread.id).toBe('m1');
    expect(el('footer-target').textContent).toBe('Old');
    expect(battleshipRender.currentMessage(base).id).toBe('m1');
    expect(battleshipRender.selectMessage(base, 'missing', document)).toBe(false);
    expect(el('comms-current-message').state.thread.id).toBe('m1');
  });

  it('shows the active hail sender name, or the localized fallback for an unnamed hail', () => {
    battleshipRender(base, document);
    expect(el('footer-target').textContent).toBe('Ops');
    battleshipRender({ ...base, messages: [{ id: 'm1', is_read: false }] }, document);
    expect(el('footer-target').textContent).toBe(t('console.common.active_hail'));
  });

  it('selects a live Critical hail without making it modal', () => {
    const messages = [
      { id: 'm1', thread_id: 'routine', is_read: false, priority: 'Routine' },
      { id: 'm2', thread_id: 'lark', is_read: true, priority: 'Critical', selected_response: null },
    ];
    battleshipRender({ ...base, messages }, document);
    expect(el('comms-current-message').state).toEqual({
      thread: messages[1], messages: [messages[1]], sender_name: undefined, rejection: null,
    });
  });

  it('shows the no-active-hail fallback with no messages', () => {
    battleshipRender({ ...base, messages: [] }, document);
    expect(el('footer-target').textContent).toBe(t('console.common.no_active_hail'));
  });

  it('reflects the AUTO badge from comms_auto alone', () => {
    battleshipRender(base, document);
    expect(el('comms-auto-badge').hidden).toBe(false);
    battleshipRender({ ...base, comms_auto: false }, document);
    expect(el('comms-auto-badge').hidden).toBe(true);
  });

  it('carries no navigation map or footer-right message count', () => {
    expect(el('navigation-map')).toBeNull();
    expect(el('footer-right')).toBeNull();
  });
});

// ── Cruiser: keyed payload, Navigation absorbed into the same Station ────────
describe('cruiser comms renderStation', () => {
  beforeEach(() => {
    rawCruiserRender.resetSelection();
    mount(FIXTURES.cruiser);
  });

  const comms = {
    contacts: [{ id: 'c1' }],
    messages: [{ id: 'm1', is_read: false }],
    rejection: 'console.common.no_target',
    comms_auto: true,
  };
  const nav = {
    blips: [{ uuid: 'n1' }], regions: [{ id: 'r1' }], radar_range: 4000,
    ship_x: 1, ship_z: 2, ship_heading: 90, waypoint: { name: 'Gate' },
    navigation_auto: true,
  };
  const payload = { systems: { comms, navigation: nav }, own_hull: { pct: 0.5 } };

  it('reads the comms view via projected Console Family for the shared core', () => {
    cruiserRender(payload, document);
    expect(el('comms-contact-list').state).toEqual({ contacts: [{ id: 'c1' }] });
    expect(el('comms-hail-list').state).toEqual({
      ...comms,
      threads: sortedThreadsFrom(comms.messages, comms.contacts),
      selected_thread_id: 'm1',
    });
    expect(el('comms-current-message').state).toEqual({
      thread: { id: 'm1', is_read: false },
      messages: comms.messages,
      sender_name: undefined,
      rejection: 'console.common.no_target',
    });
  });

  it('drives the navigation map from the absorbed navigation system, through the shared Navigation renderer', () => {
    cruiserRender(payload, document);
    expect(el('navigation-map').state).toEqual({
      blips: [{ uuid: 'n1' }], regions: [{ id: 'r1' }], range: 4000,
      ship_pos: { x: 1, z: 2 }, ship_heading: 90, waypoint: { name: 'Gate' }, auto: true,
    });
    expect(el('waypoint-name').textContent).toBe('Gate');
    expect(el('civilian-traffic').state).toEqual({ civilians: [], auto: true });
  });

  it('shows the Navigation full-panel view and hides Comms when the navigation family resolves non-empty (issue #1379)', () => {
    cruiserRender(payload, document);
    expect(el('nav-view').hidden).toBe(false);
    expect(el('comms-view').hidden).toBe(true);
  });

  it('shows the Comms full-panel view and hides Navigation when this load carries no navigation family', () => {
    const commsOnly = { systems: { comms }, own_hull: { pct: 0.5 } };
    cruiserRender(commsOnly, document);
    expect(el('comms-view').hidden).toBe(false);
    expect(el('nav-view').hidden).toBe(true);
  });

  // Issue #1379 review finding 1: the auxiliary "navigation" Station owns
  // exactly one System, so `buildConsoleStateInner` takes the single-family
  // FLAT branch and this document's load carries `system_ids`/
  // `system_families` with NO `.systems` key at all — never the keyed
  // `{ systems: {...} }` shape every other case in this suite constructs.
  // `familyView` alone resolves `{}` for that shape; the renderer must fall
  // back to the payload itself once `system_families` says it IS navigation.
  it('shows the Navigation full-panel view with real map/waypoint data from a genuinely flat single-family payload', () => {
    const flatNav = {
      ...nav,
      own_hull: { pct: 0.5 },
      system_ids: ['navigation'],
      system_families: { navigation: 'navigation' },
    };
    cruiserRender(flatNav, document);
    expect(el('nav-view').hidden).toBe(false);
    expect(el('comms-view').hidden).toBe(true);
    expect(el('navigation-map').state).toEqual({
      blips: [{ uuid: 'n1' }], regions: [{ id: 'r1' }], range: 4000,
      ship_pos: { x: 1, z: 2 }, ship_heading: 90, waypoint: { name: 'Gate' }, auto: true,
    });
    expect(el('waypoint-name').textContent).toBe('Gate');
  });

  // Issue #1380: the Comms view's readout names the open thread's channel.
  // It used to carry the WAYPOINT, from the era when one seat was Comms AND
  // Navigation; since #1379 split them into two full-panel views that readout
  // sat inside the Comms view and could only ever say NO WAYPOINT, because a
  // Comms load resolves no navigation family at all. The waypoint metric lives
  // in the Navigation view, which is the tab that shows it.
  it('names the open thread in the Comms readout, not a waypoint', () => {
    const named = {
      ...payload,
      systems: {
        ...payload.systems,
        comms: { ...comms, messages: [{ id: 'm1', is_read: false, sender_name: 'Relay Seven' }] },
      },
    };
    cruiserRender(named, document);
    expect(el('footer-target').textContent).toBe('Relay Seven');
    // The fixture's own unnamed hail falls back to the shared label.
    cruiserRender(payload, document);
    expect(el('footer-target').textContent).toBe(t('console.common.active_hail'));
    const empty = { ...payload, systems: { ...payload.systems, comms: { ...comms, messages: [] } } };
    cruiserRender(empty, document);
    expect(el('footer-target').textContent).toBe(t('console.common.no_active_hail'));
    // The Navigation view keeps the waypoint metric it owns.
    expect(el('waypoint-name').textContent).toBe('Gate');
  });

  it('shows a pluralized message count in footer-right', () => {
    cruiserRender(payload, document);
    expect(el('footer-right').textContent).toBe(t('console.comms.messages.one', { n: 1 }));
    const twoMsgs = { ...payload, systems: { ...payload.systems, comms: { ...comms, messages: [{ id: 'm1' }, { id: 'm2' }] } } };
    cruiserRender(twoMsgs, document);
    expect(el('footer-right').textContent).toBe(t('console.comms.messages.other', { n: 2 }));
  });

  it('conjuncts comms_auto and navigation_auto for the AUTO badge', () => {
    cruiserRender(payload, document);
    expect(el('comms-auto-badge').hidden).toBe(false);
    const navNotAuto = { ...payload, systems: { ...payload.systems, navigation: { ...nav, navigation_auto: false } } };
    cruiserRender(navNotAuto, document);
    expect(el('comms-auto-badge').hidden).toBe(true);
  });
});
