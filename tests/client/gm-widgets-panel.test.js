// @vitest-environment jsdom
/**
 * tests/client/gm-widgets-panel.test.js — typed world-authored GM widgets
 * (issue #1439, PRD #1419 story 13, presentation contract PRD #1418).
 *
 * The authored half of every case here is REAL: `PRESETS` is
 * `tests/fixtures/gm-widgets-presets.json`, which `tests/gm_widgets.rs` asserts
 * is exactly what `assets/worlds/probe_gm_widgets.toml` publishes through the
 * ordinary world loader and the ordinary presentation encoder. So a widget this
 * file draws is a widget a scenario author can actually write, and a shape they
 * cannot write is one this file never sees.
 *
 * The panel is driven against the exact markup `server.html` carries, over the
 * shipped `createGmAttentionFilters` — the ONE private filter/snooze controller
 * — rather than a stand-in, because "reuses A1's controller rather than
 * implementing another" is the acceptance criterion, not an implementation
 * detail.
 *
 * jsdom lays nothing out, so the measured half of the 200% claim is the CSS
 * contract at the bottom of this file plus `tests/smoke/gm-layout.spec.js`,
 * which drives the real desk at 1280x720 with `--a11y-text-scale: 2`.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { createGmWidgetsPanel } from '../../gui/gm-widgets-panel.js';
import {
  createGmAttentionFilters,
  GM_ATTENTION_SNOOZE_MS,
} from '../../gui/gm-attention-filters.js';
import {
  GM_ALL_ROLE_PRESET,
  createGmRolePresets,
  parseGmRolePresets,
} from '../../gui/gm-role-presets.js';

/** The authored worlds' own payload, shared with tests/gm_widgets.rs. */
const AUTHORED = JSON.parse(readFileSync('tests/fixtures/gm-widgets-presets.json', 'utf8'));

/** The exact ids server.html carries for the widget region and for the two GM
 * action buttons an `actions` widget may repeat. */
const MARKUP = `
  <select id="gm-role-preset-select"></select>
  <button id="gm-session-pause" type="button">Pause</button>
  <button id="gm-session-resume" type="button">Resume</button>
  <section id="gm-widgets" hidden>
    <h2 id="gm-widgets-heading"></h2>
    <ul id="gm-widgets-list"></ul>
  </section>`;

const t = (id, params = {}) => Object.entries(params)
  .reduce((text, [key, value]) => text.replaceAll(`{${key}}`, String(value)), id);

/** A String Table that resolves the ids the probe world authors, so a note can
 * be checked as the SENTENCE an operator reads rather than as an id. */
const COPY = {
  'world.probe_gm_widgets.note.brief': 'Hold the escort back until the relay answers.',
  'world.probe_gm_widgets.widget.brief': 'Facilitator brief',
  'world.probe_gm_widgets.widget.urgent_traffic': 'Urgent traffic',
  'world.probe_gm_widgets.widget.seats': 'Who is busy',
  'world.probe_gm_widgets.widget.session_levers': 'Session levers',
  'world.probe_gm_widgets.escort': 'Escort',
  'server.gm.widget.attention.narrowing': 'Starts on {narrowing}',
  'server.gm.attention.band.background': 'Background',
};
const copyT = (id, params = {}) => t(COPY[id] ?? id, params);
const copyHas = (id) => Object.prototype.hasOwnProperty.call(COPY, id);

function occurrence(extra = {}) {
  return {
    id: 'comms:m1',
    band: 'attention',
    category: 'pending_comms',
    first_seen_tick: 10,
    age_ms: 4000,
    reason: { id: 'server.gm.attention.reason.pending_comms', params: { sender: 'Axiom', ship: 'Alpha' } },
    target: { route: 'relay', ship: { entity_id: 'ship-a', name: 'world.probe_gm_widgets.escort' }, sender: null, conversation: null, event: null },
    ...extra,
  };
}

function station(extra = {}) {
  return {
    key: 'ship-a/comms',
    ship: { entity_id: 'ship-a', name: 'world.probe_gm_widgets.escort' },
    station_id: 'comms',
    station_name: 'Comms',
    level: 'engaged',
    count: 1,
    demands: [],
    ...extra,
  };
}

/** A storage the filter controller can actually persist into, so reconnect is
 * tested the way it happens rather than by poking internals. */
function memoryStorage(seed = {}) {
  const map = new Map(Object.entries(seed));
  return {
    getItem: (key) => (map.has(key) ? map.get(key) : null),
    setItem: (key, value) => { map.set(key, String(value)); },
    removeItem: (key) => { map.delete(key); },
    dump: () => Object.fromEntries(map),
  };
}

/**
 * One Game Master's whole private surface: their own filter controller, their
 * own widget region, their own storage scope.
 */
function desk({
  doc = document,
  storage = memoryStorage(),
  session = 'session-1',
  operator = 'gm-1',
  occurrences = [occurrence()],
  stations = [station()],
  now = () => 1_000_000,
  copy = false,
} = {}) {
  doc.body.innerHTML = MARKUP;
  const repaintAttention = vi.fn();
  const filters = createGmAttentionFilters({
    storage,
    getSessionId: () => session,
    getOperatorId: () => operator,
    now,
  });
  filters.restore();
  const panel = createGmWidgetsPanel({
    doc,
    t: copy ? copyT : t,
    has: copy ? copyHas : ((id) => typeof id === 'string' && id.startsWith('world.')),
    filters,
    readAttention: () => ({ occurrences }),
    readWorkload: () => stations,
    repaintAttention,
  });
  return { panel, filters, storage, repaintAttention };
}

const cards = (doc = document) => [...doc.querySelectorAll('#gm-widgets-list > li')];
const cardFor = (id, doc = document) => doc.querySelector(`#gm-widgets-list > li[data-widget-id="${id}"]`);
const tactical = () => parseGmRolePresets(AUTHORED)[0];
const narrative = () => parseGmRolePresets(AUTHORED)[1];

afterEach(() => { document.body.innerHTML = ''; vi.restoreAllMocks(); });

describe('the four typed widgets an author may compose', () => {
  it('renders all four from a real authored world, keyed by their authored ids', () => {
    const { panel } = desk();
    panel.setPreset(tactical(), { source: 'select' });
    expect(cards().map((card) => card.dataset.widgetId))
      .toEqual(['urgent-traffic', 'seats', 'session-levers', 'brief']);
    expect(cards().map((card) => card.dataset.widgetType))
      .toEqual(['attention', 'workload', 'actions', 'note']);
    // Every heading is String Table copy, and every card is a labelled group.
    for (const card of cards()) {
      const heading = card.querySelector('.gm-widget-heading');
      expect(heading.textContent.startsWith('world.probe_gm_widgets.')).toBe(true);
      expect(card.getAttribute('aria-labelledby')).toBe(heading.id);
    }
    expect(document.getElementById('gm-widgets').hidden).toBe(false);
  });

  it('drops an unknown widget type and a widget with no label rather than drawing an empty card', () => {
    // Rust refuses both at world load naming the section and index
    // (src/world/config_tests.rs). This is the same refusal one level on, for
    // a payload that never came from a world file.
    const poked = parseGmRolePresets([{
      id: 'tactical',
      widget: [
        { id: 'scores', type: 'scoreboard', label: 'world.x.a' },
        { id: 'nameless', type: 'note', text: 'world.x.b' },
        { id: 'real', type: 'note', label: 'world.x.c', text: 'world.x.d' },
      ],
    }]);
    const { panel } = desk();
    panel.setPreset(poked[0], { source: 'select' });
    expect(panel.state().widgets.map((widget) => widget.id)).toEqual(['real']);
  });

  it('shows an authored note as text, never as markup a page could run', () => {
    const { panel } = desk({ copy: true });
    panel.setPreset(tactical(), { source: 'select' });
    const note = cardFor('brief').querySelector('.gm-widget-note');
    expect(note.textContent).toBe('Hold the escort back until the relay answers.');
    // The whole contract in one assertion: even if a String Table row were
    // poked full of markup, the note is text and the card grows no elements.
    COPY['world.probe_gm_widgets.note.brief'] = '<img src=x onerror="alert(1)">';
    panel.repaint();
    const poked = cardFor('brief').querySelector('.gm-widget-note');
    expect(poked.textContent).toBe('<img src=x onerror="alert(1)">');
    expect(poked.children.length).toBe(0);
    expect(document.querySelector('#gm-widgets-list img')).toBeNull();
    COPY['world.probe_gm_widgets.note.brief'] = 'Hold the escort back until the relay answers.';
  });

  it('summarises the Station workload from the rows the advisory already parsed', () => {
    const { panel } = desk({
      stations: [station(), station({ key: 'ship-b/helm', ship: { entity_id: 'ship-b', name: 'Bravo' }, station_id: 'helm', station_name: 'Helm', level: 'overloaded', count: 3 })],
    });
    panel.setPreset(tactical(), { source: 'select' });
    const rows = [...cardFor('seats').querySelectorAll('li')];
    expect(rows.map((row) => row.dataset.stationKey)).toEqual(['ship-a/comms', 'ship-b/helm']);
    // The level is a WORD on the row, so forced colours lose nothing.
    expect(rows[1].querySelector('.gm-widget-level').textContent)
      .toBe('server.gm.workload.state.overloaded');
    expect(cardFor('seats').querySelector('.gm-widget-summary').textContent)
      .toBe('server.gm.widget.workload.summary');
  });
});

describe('authored default filters over the one shared controller', () => {
  it('seeds the operator\'s own band/category filters when they select the preset', () => {
    const { panel, filters, repaintAttention } = desk();
    expect(filters.filters()).toEqual({ band: 'all', category: 'all', ship: 'all' });
    panel.setPreset(tactical(), { source: 'select' });
    expect(filters.filters()).toEqual({ band: 'urgent', category: 'pending_comms', ship: 'all' });
    // The queue beside the widget repaints from the SAME controller, so the
    // two surfaces cannot disagree about what this operator is looking at.
    expect(repaintAttention).toHaveBeenCalled();
  });

  it('resolves an authored ship name to the entity id the shared filter speaks', () => {
    // An author cannot know a runtime entity id, so they name the hull the way
    // [[gm_role_preset]].contacts does and the browser resolves it live.
    const { panel, filters } = desk();
    panel.setPreset(narrative(), { source: 'select' });
    expect(filters.filters()).toEqual({ band: 'background', category: 'all', ship: 'ship-a' });
  });

  it('leaves the ship facet unnarrowed when this session has never seen that hull', () => {
    // Narrowing to an id that exists nowhere would empty every surface and
    // read as "nothing is happening", which is the opposite of the truth.
    const { panel, filters } = desk({ occurrences: [], stations: [] });
    panel.setPreset(narrative(), { source: 'select' });
    expect(filters.filters().ship).toBe('all');
  });

  it('clears a previous role\'s narrowing when the operator switches to a broader one', () => {
    const { panel, filters } = desk();
    panel.setPreset(tactical(), { source: 'select' });
    expect(filters.filters().category).toBe('pending_comms');
    panel.setPreset(narrative(), { source: 'select' });
    expect(filters.filters()).toEqual({ band: 'background', category: 'all', ship: 'ship-a' });
  });

  it('narrows nothing of its own: the rows it lists are exactly the controller\'s visible set', () => {
    const rows = [
      occurrence({ id: 'comms:m1', band: 'urgent' }),
      occurrence({ id: 'comms:m2', band: 'background' }),
      occurrence({ id: 'beat:one', band: 'urgent', category: 'eligible_beat' }),
    ];
    const { panel, filters } = desk({ occurrences: rows });
    panel.setPreset(tactical(), { source: 'select' });
    // band=urgent + category=pending_comms leaves one row, and it is the one
    // `filters.visible` leaves.
    const listed = [...cardFor('urgent-traffic').querySelectorAll('li')]
      .map((row) => row.dataset.occurrenceId);
    expect(listed).toEqual(rows.filter((row) => filters.visible(row)).map((row) => row.id));
    expect(listed).toEqual(['comms:m1']);
    expect(cardFor('urgent-traffic').querySelector('.gm-widget-summary').textContent)
      .toBe('server.gm.widget.attention.summary');
  });

  it('hides a row this operator snoozed, through the same snooze the queue uses', () => {
    let clock = 1_000_000;
    const { panel, filters } = desk({ occurrences: [occurrence({ band: 'urgent' })], now: () => clock });
    panel.setPreset(tactical(), { source: 'select' });
    expect(cardFor('urgent-traffic').querySelectorAll('li')).toHaveLength(1);
    filters.snooze('comms:m1', 'urgent');
    panel.repaint();
    expect(cardFor('urgent-traffic').querySelectorAll('li')).toHaveLength(0);
    expect(cardFor('urgent-traffic').querySelector('.gm-widget-empty').textContent)
      .toBe('server.gm.widget.attention.empty');
    // And it comes back when the one real minute is up — the widget has no
    // clock of its own to disagree with.
    clock += GM_ATTENTION_SNOOZE_MS + 1;
    panel.repaint();
    expect(cardFor('urgent-traffic').querySelectorAll('li')).toHaveLength(1);
  });

  it('says what it is narrowed to, in words rather than by colour alone', () => {
    const { panel } = desk({ copy: true });
    panel.setPreset(narrative(), { source: 'select' });
    expect(cardFor('quiet-watch').querySelector('.gm-widget-narrowing').textContent)
      .toBe('Starts on Background · Escort');
  });
});

describe('two Game Masters, one session', () => {
  it('lets two operators hold different presets and different filters with no crosstalk', () => {
    // One browser profile, two operator identities: the scope key is session +
    // operator, so neither inherits the other's narrowing or their snoozes.
    const storage = memoryStorage();
    const first = desk({ storage, operator: 'gm-1' });
    first.panel.setPreset(tactical(), { source: 'select' });
    first.filters.snooze('comms:m1', 'urgent');
    const second = desk({ storage, operator: 'gm-2' });
    second.panel.setPreset(narrative(), { source: 'select' });

    expect(first.filters.state().snoozes.map((row) => row.id)).toEqual(['comms:m1']);
    expect(second.filters.isSnoozed('comms:m1')).toBe(false);
    expect(second.filters.filters()).toEqual({ band: 'background', category: 'all', ship: 'ship-a' });
    // Re-reading the first operator's own scope still finds what they left.
    const again = desk({ storage, operator: 'gm-1' });
    expect(again.filters.filters().band).toBe('urgent');
  });

  it('lets two operators hold the SAME preset without one of them changing the other', () => {
    const storage = memoryStorage();
    const first = desk({ storage, operator: 'gm-1' });
    const second = desk({ storage, operator: 'gm-2' });
    first.panel.setPreset(tactical(), { source: 'select' });
    second.panel.setPreset(tactical(), { source: 'select' });
    first.filters.setFilter('band', 'background');
    expect(first.filters.filters().band).toBe('background');
    expect(second.filters.filters().band).toBe('urgent');
  });

  it('submits nothing: a widget region is presentation, and presses no action of its own', () => {
    // The structural claim behind "without changing authority". The only
    // control this region draws is a forwarding button, and it forwards.
    const { panel } = desk();
    const pause = document.getElementById('gm-session-pause');
    const pressed = vi.fn();
    pause.addEventListener('click', pressed);
    panel.setPreset(tactical(), { source: 'select' });
    expect(pressed).not.toHaveBeenCalled();
    panel.repaint();
    panel.setPreset(narrative(), { source: 'select' });
    expect(pressed).not.toHaveBeenCalled();
  });
});

describe('reconnect, a new session, and a removed preset', () => {
  it('restores a same-session operator\'s own filters and remaining snooze instead of re-imposing the authored defaults', () => {
    let clock = 1_000_000;
    const storage = memoryStorage();
    const before = desk({ storage, now: () => clock });
    before.panel.setPreset(tactical(), { source: 'select' });
    // The operator then decides otherwise. That is their state, not the
    // author's suggestion.
    before.filters.setFilter('band', 'background');
    before.filters.snooze('comms:m1', 'background');

    clock += 30_000;
    const after = desk({ storage, now: () => clock });
    after.panel.setPreset(tactical(), { source: 'restore' });
    expect(after.filters.filters().band).toBe('background');
    // Half the minute is left, not a fresh one.
    expect(after.filters.snoozeRemainingMs('comms:m1')).toBe(GM_ATTENTION_SNOOZE_MS - 30_000);
    // And the widgets themselves are back, because the preset is.
    expect(cards().map((card) => card.dataset.widgetId))
      .toEqual(['urgent-traffic', 'seats', 'session-levers', 'brief']);
  });

  it('starts a new session on All with no stale snoozes', () => {
    let clock = 1_000_000;
    const storage = memoryStorage();
    const before = desk({ storage, session: 'session-1', now: () => clock });
    before.panel.setPreset(tactical(), { source: 'select' });
    before.filters.snooze('comms:m1', 'urgent');

    // A DIFFERENT session on the same browser: the stored scope is not this
    // session's, so it is dropped rather than resurrected.
    const fresh = desk({ storage, session: 'session-2', now: () => clock + 1000 });
    expect(fresh.filters.filters()).toEqual({ band: 'all', category: 'all', ship: 'all' });
    expect(fresh.filters.isSnoozed('comms:m1')).toBe(false);
    // And with no stored preset choice, the desk is the built-in All: no
    // authored widget region at all.
    fresh.panel.setPreset(GM_ALL_ROLE_PRESET, { source: 'restore' });
    expect(cards()).toHaveLength(0);
    expect(document.getElementById('gm-widgets').hidden).toBe(true);
  });

  it('falls back to All — and therefore to no widget region — when the world stops declaring the preset', () => {
    const { panel } = desk();
    const controller = createGmRolePresets({
      doc: document,
      onEffective: (preset, context) => panel.setPreset(preset, context),
    });
    controller.setAvailablePresets(AUTHORED);
    controller.select('tactical');
    expect(cards().map((card) => card.dataset.widgetId)).toContain('session-levers');

    // A world reload (or a mod-pack switch) that no longer declares it. The
    // operator's CHOICE is untouched, so the preset returning restores the
    // region without them re-selecting it.
    controller.setAvailablePresets([AUTHORED[1]]);
    expect(controller.state().effectivePresetId).toBe('all');
    expect(cards()).toHaveLength(0);
    expect(document.getElementById('gm-widgets').hidden).toBe(true);
    controller.setAvailablePresets(AUTHORED);
    expect(cards().map((card) => card.dataset.widgetId)).toContain('session-levers');
  });

  it('applies the authored defaults on a live switch and never on a restore', () => {
    const { panel, filters } = desk();
    const controller = createGmRolePresets({
      doc: document,
      onEffective: (preset, context) => panel.setPreset(preset, context),
    });
    controller.setAvailablePresets(AUTHORED);
    filters.setFilter('band', 'background');
    // A restore of the same preset leaves the operator's own choice alone…
    controller.restore('tactical');
    expect(filters.filters().band).toBe('background');
    // …and their own live switch is them asking for the author's view.
    controller.select('tactical');
    expect(filters.filters().band).toBe('urgent');
  });
});

describe('existing permitted GM action buttons', () => {
  it('activates the shipped control rather than issuing an action of its own', () => {
    const { panel } = desk();
    const pause = document.getElementById('gm-session-pause');
    const pressed = vi.fn();
    pause.addEventListener('click', pressed);
    panel.setPreset(tactical(), { source: 'select' });
    const button = cardFor('session-levers').querySelector('button[data-widget-action="gm-session-pause"]');
    // It wears the shipped control's own name, because it IS that control.
    expect(button.textContent).toBe('Pause');
    button.click();
    expect(pressed).toHaveBeenCalledTimes(1);
    // One press, one activation: the confirmation and feedback that button
    // already carries are the only ones in play.
    expect(pressed.mock.calls[0][0].target).toBe(pause);
  });

  it('mirrors a control the desk is not offering as a disabled button and says so', () => {
    const { panel } = desk();
    document.getElementById('gm-session-resume').disabled = true;
    panel.setPreset(tactical(), { source: 'select' });
    const card = cardFor('session-levers');
    expect(card.querySelector('button[data-widget-action="gm-session-pause"]').disabled).toBe(false);
    const resume = card.querySelector('button[data-widget-action="gm-session-resume"]');
    expect(resume.disabled).toBe(true);
    expect(card.querySelector('.gm-widget-unavailable').textContent)
      .toBe('server.gm.widget.actions.unavailable');
    // A disabled mirror presses nothing.
    const pressed = vi.fn();
    document.getElementById('gm-session-resume').addEventListener('click', pressed);
    resume.click();
    expect(pressed).not.toHaveBeenCalled();
  });

  it('draws no button for an action id outside the registry, however the payload arrives', () => {
    const poked = parseGmRolePresets([{
      id: 'tactical',
      widget: [
        { id: 'sneaky', type: 'actions', label: 'world.x.a', actions: ['gm-despawn-confirm'] },
        { id: 'real', type: 'actions', label: 'world.x.b', actions: ['gm-session-pause', 'gm-despawn-confirm'] },
      ],
    }]);
    const { panel } = desk();
    panel.setPreset(poked[0], { source: 'select' });
    // The whole widget goes when nothing in it is permitted; the mixed one
    // keeps only the id the registry holds.
    expect(panel.state().widgets.map((widget) => widget.id)).toEqual(['real']);
    expect([...cardFor('real').querySelectorAll('button')].map((b) => b.dataset.widgetAction))
      .toEqual(['gm-session-pause']);
  });
});

describe('the #1418 usability contract', () => {
  it('keeps the keyboard on the same button across a repaint', () => {
    const { panel } = desk();
    panel.setPreset(tactical(), { source: 'select' });
    const button = cardFor('session-levers').querySelector('button[data-widget-action="gm-session-pause"]');
    button.focus();
    expect(document.activeElement).toBe(button);
    // A projection arrives while the operator is standing on the control.
    panel.repaint();
    expect(document.activeElement.dataset.widgetAction).toBe('gm-session-pause');
  });

  it('keeps every sentence and every target at 200% text', () => {
    document.documentElement.style.setProperty('--a11y-text-scale', '2');
    const { panel } = desk({ copy: true });
    panel.setPreset(tactical(), { source: 'select' });
    // Nothing is shortened for the larger size: the note is the whole sentence
    // and the headings are the whole headings.
    expect(cardFor('brief').querySelector('.gm-widget-note').textContent)
      .toBe('Hold the escort back until the relay answers.');
    expect(cardFor('seats').querySelector('.gm-widget-heading').textContent).toBe('Who is busy');
    // Every control is a real button, in the tab ring, reachable by keyboard.
    for (const button of cardFor('session-levers').querySelectorAll('button')) {
      expect(button.tagName).toBe('BUTTON');
      expect(button.type).toBe('button');
      button.focus();
      expect(document.activeElement).toBe(button);
    }
    document.documentElement.style.removeProperty('--a11y-text-scale');
  });

  it('opens no pop-up and switches no panel for routine attention', () => {
    // PRD #1418 story 27 in one assertion: a busy queue arriving under a
    // widget adds rows and nothing else.
    const { panel } = desk({
      occurrences: [occurrence({ band: 'urgent' }), occurrence({ id: 'comms:m2', band: 'urgent' })],
    });
    panel.setPreset(tactical(), { source: 'select' });
    expect(document.querySelectorAll('dialog')).toHaveLength(0);
    expect(cardFor('urgent-traffic').querySelectorAll('li')).toHaveLength(2);
    expect(document.activeElement).toBe(document.body);
  });

  it('has widget styles that grow with the text rather than clipping it', () => {
    const css = readFileSync('gui/gm-workspace.css', 'utf8');
    const card = css.slice(css.indexOf('#gm-console.gm-desk #gm-widgets-list > li {'));
    const rule = card.slice(0, card.indexOf('}'));
    expect(rule).toContain('overflow-wrap: anywhere');
    expect(rule).toContain('min-width: 0');
    // No fixed height to clip a wrapped note at 200%.
    expect(rule).not.toMatch(/(^|[^-])height:\s*\d/);
    // The cards wrap into as many columns as fit and never force the desk to
    // scroll sideways, which is the contract the smoke spec measures.
    const list = css.slice(css.indexOf('#gm-console.gm-desk #gm-widgets-list {'));
    const listRule = list.slice(0, list.indexOf('}'));
    expect(listRule).toContain('auto-fit');
    expect(listRule).toContain('min-width: 0');
    // A widget button is a second press of a shipped control, not a smaller
    // one: it keeps the shared hit floor and wraps rather than squeezing.
    const actions = css.slice(css.indexOf('#gm-console.gm-desk .gm-widget-actions {'));
    expect(actions.slice(0, actions.indexOf('}'))).toContain('flex-wrap: wrap');
    const button = css.slice(css.indexOf('#gm-console.gm-desk .gm-widget-actions button {'));
    expect(button.slice(0, button.indexOf('}'))).toContain('min-height: var(--control-hit-min)');
  });

  it('is the region server.html actually ships, with its hidden state owned here', () => {
    const html = readFileSync('server.html', 'utf8');
    expect(html).toContain('<section id="gm-widgets" role="region" aria-labelledby="gm-widgets-heading" hidden>');
    expect(html).toContain('<ul id="gm-widgets-list" role="list"></ul>');
    // The region is deliberately absent from the role preset's own panel list:
    // gm-widgets-panel.js owns `hidden`, and a second writer would race it.
    const presets = readFileSync('gui/gm-role-presets.js', 'utf8');
    const panelIds = presets.slice(presets.indexOf('GM_ROLE_PRESET_PANEL_IDS'));
    expect(panelIds.slice(0, panelIds.indexOf(']);'))).not.toContain('gm-widgets');
  });
});
