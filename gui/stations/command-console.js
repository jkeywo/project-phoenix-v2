/**
 * gui/stations/command-console.js — the Command renderer (issue #1381).
 *
 * Command is not a per-hull composite: every hull that authors a Command
 * station points its `[[station]] console` at the SAME shared
 * `gui/command-console.html`, so — unlike the `gui/stations/*.js` renderers
 * that take a per-hull `variant` and get re-exported through a
 * `gui/<class>/command.console.js` sibling — this module exports
 * `renderStation(s, doc)` directly, and `gui/command-console.html` imports it
 * straight off this path. That is also the natural home
 * `tests/client/command-console.test.js` imports from.
 *
 * `s` is the `CommandConsolePayload` documented on `buildCommandConsoleState`
 * in `gui/console-state.js`: `{ red_alert, stations: CommandStationCard[] }`.
 * One card renders per entry in `stations` — exactly one today, since exactly
 * one Station on a shipped hull ever authors `command_target` — each showing
 * that Station's STANDARD stances plus one synthesized Default row. The
 * Default row's target stance (`station.default_stance`) and whether it is
 * currently in force (`station.default_selected`) both already arrive
 * resolved from the builder; this module only draws them and sends the click.
 */
import { t } from '../strings.js';

// A high-alert posture reads as "▲ armed"; a stood-down one as "▬ cold". A
// glyph, not a colour, so the posture survives greyscale (issue #1107).
function postureGlyph(high) {
  return high ? '▲' : '▬';
}

/** Send `set_station_stance` the same way every console action leaves the
 * page (issue #1236's PhElement accessor reads the exact same global) —
 * `window.sendAction` is published by `initConsole` at boot. */
function sendStanceAction(station, stanceId) {
  const send = (typeof window !== 'undefined') ? window.sendAction : null;
  if (typeof send === 'function') send('set_station_stance', { station, stance: stanceId });
}

/**
 * One stance button: the mark/posture/name row shared by every ordinary
 * stance and the synthesized Default row alike.
 *
 * @param {Document} doc
 * @param {{id?: string, label?: string, high_alert?: boolean}} stance
 * @param {{pressed: boolean, disabled: boolean, extraClass?: string}} state
 * @param {function(): void} onClick
 */
function buildStanceButton(doc, stance, { pressed, disabled, extraClass }, onClick) {
  const btn = doc.createElement('button');
  btn.type = 'button';
  btn.className = extraClass ? 'stance-btn ' + extraClass : 'stance-btn';
  btn.setAttribute('aria-pressed', pressed ? 'true' : 'false');
  btn.disabled = disabled;

  // Selected: a persistent glyph, not a colour — the mark survives greyscale.
  const mark = doc.createElement('span');
  mark.className = 'mark';
  mark.textContent = pressed ? '●' : '';
  btn.appendChild(mark);

  const posture = doc.createElement('span');
  posture.className = 'posture';
  posture.textContent = postureGlyph(!!(stance && stance.high_alert));
  btn.appendChild(posture);

  const name = doc.createElement('span');
  name.className = 'name';
  name.textContent = stance && stance.label ? t(stance.label) : (stance && stance.id) || '';
  btn.appendChild(name);

  btn.addEventListener('click', function () {
    if (btn.disabled) return;
    onClick();
  });
  return btn;
}

/**
 * One directable-station card: the "Directing" row with its cues, the
 * "Stance" heading with the stance in force, and the button list — the
 * station's standard stances plus one Default row (issue #1381).
 *
 * @param {Document} doc
 * @param {import('../console-state.js').CommandStationCard} station
 */
function buildCard(doc, station) {
  const directable = !!station.directed_station_ai;

  const card = doc.createElement('div');
  card.className = 'command-card';
  // A crewed station's card is greyed as a whole (criterion 3): the CREWED
  // cue inside it still reads at full contrast against its own bordered
  // badge, so dimming the card never hides the reason its buttons refuse.
  if (!directable) card.classList.add('crewed');
  if (station.directed_station) card.dataset.station = station.directed_station;

  // ── "Directing" row: name plus the two persistent non-colour cues ──────
  const directed = doc.createElement('div');
  directed.className = 'directed';

  const label = doc.createElement('span');
  label.className = 'label';
  label.textContent = t('console.command.directing');
  directed.appendChild(label);

  // A directed Station's name is localised from `station.<id>.name` (the
  // project convention — station ids stay English in TOML). With no station
  // directed, the blackboard's authored name is shown, or an em-dash when
  // even that is absent.
  const name = doc.createElement('span');
  name.className = 'name directed-name';
  name.textContent = station.directed_station
    ? t('station.' + station.directed_station + '.name')
    : (station.directed_station_name || '—');
  directed.appendChild(name);

  // Command's own automation cue (it may itself be run by the ship AI).
  if (station.command_auto) {
    const cmdCue = doc.createElement('span');
    cmdCue.className = 'cue command-cue';
    const glyph = doc.createElement('span');
    glyph.className = 'glyph command-cue-glyph';
    glyph.textContent = '◇';
    cmdCue.appendChild(glyph);
    const text = doc.createElement('span');
    text.className = 'command-cue-text';
    text.textContent = t('console.command.command_auto');
    cmdCue.appendChild(text);
    directed.appendChild(cmdCue);
  }

  // The directed Station's persistent NON-COLOUR cue: directable only while
  // it is AI-controlled.
  const stationCue = doc.createElement('span');
  stationCue.className = 'cue station-cue';
  const stationGlyph = doc.createElement('span');
  stationGlyph.className = 'glyph station-cue-glyph';
  const stationText = doc.createElement('span');
  stationText.className = 'station-cue-text';
  if (directable) {
    stationGlyph.textContent = '◆'; // ◆ filled — under automation, directable
    stationText.textContent = t('console.command.ai_directed');
  } else {
    stationGlyph.textContent = '◇'; // ◇ hollow — crewed, off the board
    stationText.textContent = t('console.command.human_held');
  }
  stationCue.appendChild(stationGlyph);
  stationCue.appendChild(stationText);
  directed.appendChild(stationCue);

  card.appendChild(directed);

  // ── "Stance" heading: the stance in force, resolved to its label ───────
  const head = doc.createElement('div');
  head.className = 'stance-head';
  const heading = doc.createElement('span');
  heading.className = 'stance-heading';
  heading.textContent = t('console.command.stance_heading');
  head.appendChild(heading);
  const spacer = doc.createElement('span');
  spacer.className = 'spacer';
  head.appendChild(spacer);

  const stances = station.stances || [];
  let current = stances.find(function (x) { return x.id === station.selected_stance; });
  // The stance in force may be one of the two neutral kinds this card folds
  // into the Default row rather than listing — look there next.
  if (!current && station.default_selected) current = station.default_stance;
  const currentLabel = current
    ? (current.label ? t(current.label) : current.id)
    : (station.selected_stance || '—');
  const currentEl = doc.createElement('span');
  currentEl.className = 'stance-heading stance-current';
  currentEl.textContent = t('console.command.current_stance') + ' ' + currentLabel;
  head.appendChild(currentEl);
  card.appendChild(head);

  // ── Stance buttons: the standard catalogue, plus one Default row ───────
  const list = doc.createElement('div');
  list.className = 'stances';
  for (const stance of stances) {
    const pressed = stance.id === station.selected_stance;
    const btn = buildStanceButton(doc, stance, { pressed, disabled: !directable }, function () {
      sendStanceAction(station.directed_station, stance.id);
    });
    list.appendChild(btn);
  }

  // The synthesized Default row (issue #1381): resolved server-catalogue-side
  // to whichever neutral stance matches the client's own `red_alert`, but its
  // NAME is this console's own string — a client synthesis over the two
  // authored neutral entries, never itself an authored stance label. Marked
  // pressed whenever the stance in force is EITHER neutral kind, so it keeps
  // visibly tracking the reconciled selection across an alert change with no
  // new command from here.
  const dflt = station.default_stance;
  const dfltBtn = buildStanceButton(
    doc,
    dflt || {},
    { pressed: !!station.default_selected, disabled: !directable || !dflt, extraClass: 'default-row' },
    function () {
      if (dflt) sendStanceAction(station.directed_station, dflt.id);
    },
  );
  const dfltName = dfltBtn.querySelector('.name');
  if (dfltName) dfltName.textContent = t('console.command.stance.default');
  list.appendChild(dfltBtn);

  card.appendChild(list);
  return card;
}

/**
 * Render every directable station's card (issue #1381). `s` is the parsed
 * `CommandConsolePayload`; `doc` defaults to the global `document`.
 *
 * @param {import('../console-state.js').CommandConsolePayload} s
 * @param {Document} [doc]
 */
export function renderStation(s, doc) {
  doc = doc || (typeof document !== 'undefined' ? document : null);
  if (!doc) return;
  const list = doc.getElementById('stations-list');
  if (!list) return;

  list.textContent = '';
  const stations = (s && s.stations) || [];
  for (const station of stations) {
    list.appendChild(buildCard(doc, station));
  }
}
