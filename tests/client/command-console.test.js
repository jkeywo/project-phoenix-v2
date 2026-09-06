// @vitest-environment jsdom
/**
 * tests/client/command-console.test.js — the Command console cards (issue
 * #1381).
 *
 * `gui/command-console.html` imports `renderStation` from
 * `gui/stations/command-console.js`; this suite imports the SAME function
 * and drives it against a jsdom fixture, so the card-per-directable-station
 * contract — standard-only buttons, the synthesized Default row, and the
 * CREWED greying — is asserted without a browser. The payload shape (`{
 * red_alert, stations }`) is the one `buildCommandConsoleState` in
 * `gui/console-state.js` produces; `console-state.test.js` pins that builder,
 * this pins what the console draws from it.
 */
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { t } from '../../gui/strings.js';
import { renderStation } from '../../gui/stations/command-console.js';

function mount() {
  document.body.innerHTML = '<div id="stations-list"></div>';
}
const list = () => document.getElementById('stations-list');
const cards = () => [...document.querySelectorAll('.command-card')];
const stanceButtons = (card) => [...card.querySelectorAll('.stances > .stance-btn:not(.default-row)')];
const defaultButton = (card) => card.querySelector('.stance-btn.default-row');

function aiDirectedStation(over = {}) {
  return {
    command_system_id: 'command',
    directed_station: 'tactical',
    directed_station_name: 'Tactical',
    directed_station_ai: true,
    command_auto: false,
    selected_stance: 'tactical-normal',
    stances: [
      { id: 'tactical-weapons-free', label: 'entity.alliance_destroyer.station.tactical.stance.weapons_free', kind: 'standard', high_alert: true },
      { id: 'tactical-hold', label: 'entity.alliance_destroyer.station.tactical.stance.hold', kind: 'standard', high_alert: false },
    ],
    default_stance: { id: 'tactical-normal', label: 'entity.alliance_destroyer.station.tactical.stance.normal', kind: 'normal_alert_neutral', high_alert: false },
    default_selected: true,
    ...over,
  };
}

describe('command console renderStation', () => {
  let sent;
  beforeEach(() => {
    mount();
    sent = [];
    window.sendAction = (action, payload) => sent.push({ action, ...payload });
  });
  afterEach(() => {
    delete window.sendAction;
  });

  it('renders no cards before any station has arrived', () => {
    renderStation({ red_alert: false, stations: [] }, document);
    expect(cards()).toHaveLength(0);
  });

  it('renders one card per directable station (criterion 1) — one for the singular command_target', () => {
    renderStation({ red_alert: false, stations: [aiDirectedStation()] }, document);
    expect(cards()).toHaveLength(1);
    expect(cards()[0].querySelector('.directed-name').textContent).toBe(t('station.tactical.name'));
  });

  it('lists only the standard stances as ordinary buttons (criterion 2)', () => {
    renderStation({ red_alert: false, stations: [aiDirectedStation()] }, document);
    const card = cards()[0];
    // The fixture's two standard stances, plus exactly one Default row.
    expect(stanceButtons(card)).toHaveLength(2);
    expect(defaultButton(card)).not.toBeNull();
    // Neither neutral kind leaked into the ordinary button list.
    const names = stanceButtons(card).map((b) => b.querySelector('.name').textContent);
    expect(names).toEqual([
      t('entity.alliance_destroyer.station.tactical.stance.weapons_free'),
      t('entity.alliance_destroyer.station.tactical.stance.hold'),
    ]);
  });

  it('labels the Default row from the console\'s own string, not any authored stance label', () => {
    renderStation({ red_alert: false, stations: [aiDirectedStation()] }, document);
    const btn = defaultButton(cards()[0]);
    expect(btn.querySelector('.name').textContent).toBe(t('console.command.stance.default'));
  });

  it('marks the Default row pressed when the current alert-neutral is in force, and sends its resolved id on click', () => {
    renderStation({ red_alert: false, stations: [aiDirectedStation()] }, document);
    const card = cards()[0];
    const btn = defaultButton(card);
    expect(btn.getAttribute('aria-pressed')).toBe('true');
    // No ordinary button reads as pressed — the neutral fallback is not a
    // standard stance and does not double up on both rows.
    expect(stanceButtons(card).some((b) => b.getAttribute('aria-pressed') === 'true')).toBe(false);

    btn.click();
    expect(sent).toEqual([{ action: 'set_station_stance', station: 'tactical', stance: 'tactical-normal' }]);
  });

  it('keeps the Default row pressed for EITHER neutral kind, tracking across an alert change with no new command', () => {
    // red_alert has flipped to true, but the resolved default_stance now
    // targets the HIGH-alert neutral while selected_stance is still (briefly)
    // the pre-flip normal-alert neutral — the server's own reconciliation
    // (`command_stance::selection_after_alert_change`) is what moves the
    // selection on; this only pins that the row still reads as in force
    // through that gap.
    const station = aiDirectedStation({
      selected_stance: 'tactical-normal',
      default_stance: { id: 'tactical-high', label: 'entity.alliance_destroyer.station.tactical.stance.high', kind: 'high_alert_neutral', high_alert: true },
      default_selected: true,
    });
    renderStation({ red_alert: true, stations: [station] }, document);
    expect(defaultButton(cards()[0]).getAttribute('aria-pressed')).toBe('true');
  });

  it('sends set_station_stance for an ordinary standard stance on click', () => {
    renderStation({ red_alert: false, stations: [aiDirectedStation()] }, document);
    stanceButtons(cards()[0])[1].click();
    expect(sent).toEqual([{ action: 'set_station_stance', station: 'tactical', stance: 'tactical-hold' }]);
  });

  it('greys a crewed station\'s card and disables its buttons, with the CREWED cue', () => {
    const station = aiDirectedStation({ directed_station_ai: false, default_selected: false });
    renderStation({ red_alert: false, stations: [station] }, document);
    const card = cards()[0];
    expect(card.classList.contains('crewed')).toBe(true);
    expect(card.querySelector('.station-cue-text').textContent).toBe(t('console.command.human_held'));
    for (const btn of stanceButtons(card)) expect(btn.disabled).toBe(true);
    expect(defaultButton(card).disabled).toBe(true);

    // Disabled buttons refuse the click.
    stanceButtons(card)[0].click();
    defaultButton(card).click();
    expect(sent).toHaveLength(0);
  });

  it('shows the AI-directable cue and enabled buttons for an AI-controlled station', () => {
    renderStation({ red_alert: false, stations: [aiDirectedStation()] }, document);
    const card = cards()[0];
    expect(card.classList.contains('crewed')).toBe(false);
    expect(card.querySelector('.station-cue-text').textContent).toBe(t('console.command.ai_directed'));
    for (const btn of stanceButtons(card)) expect(btn.disabled).toBe(false);
    expect(defaultButton(card).disabled).toBe(false);
  });

  it('shows the command-auto cue only while Command itself is AI-run', () => {
    renderStation({ red_alert: false, stations: [aiDirectedStation({ command_auto: true })] }, document);
    expect(cards()[0].querySelector('.command-cue-text').textContent).toBe(t('console.command.command_auto'));

    renderStation({ red_alert: false, stations: [aiDirectedStation({ command_auto: false })] }, document);
    expect(cards()[0].querySelector('.command-cue')).toBeNull();
  });

  it('disables the Default row and never throws when the catalogue has no neutral stance to resolve (e.g. no blackboard yet)', () => {
    const station = aiDirectedStation({ stances: [], default_stance: null, default_selected: false, selected_stance: '' });
    renderStation({ red_alert: false, stations: [station] }, document);
    const card = cards()[0];
    const btn = defaultButton(card);
    expect(btn.disabled).toBe(true);
    btn.click();
    expect(sent).toHaveLength(0);
  });

  it('rebuilds cards on re-render rather than accumulating them', () => {
    renderStation({ red_alert: false, stations: [aiDirectedStation()] }, document);
    renderStation({ red_alert: false, stations: [aiDirectedStation()] }, document);
    expect(cards()).toHaveLength(1);
  });

  it('does nothing when the mount point is absent (no throw)', () => {
    document.body.innerHTML = '';
    expect(() => renderStation({ red_alert: false, stations: [aiDirectedStation()] }, document)).not.toThrow();
  });
});
