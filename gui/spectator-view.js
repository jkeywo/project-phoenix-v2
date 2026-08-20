/**
 * gui/spectator-view.js — the crew-public Spectator summary surface (issue
 * #1105).
 *
 * A Spectator holds no Station, so the host only ever addresses it with
 * `Audience::All` broadcasts — exactly the crew-public set (mission
 * objectives, red-alert, ship-wide hull, per-Station health). None of the
 * per-station private feeds (`Audience::Holding*`) ever reach a seatless
 * client, so this surface CANNOT show private data: it only re-presents the
 * crew-public sim state the spectator already holds.
 *
 * Split like gui/hero-bar.js: `spectatorSummaryModel` is a pure, DOM-free model
 * builder (unit-tested), and `renderSpectatorDom` writes that model into a set
 * of host elements. The Hero Bar's health/importance classifiers are reused so
 * the two surfaces agree on what "damaged"/"critical" mean.
 */

import { heroBarHealthState, heroBarImportanceState } from './hero-bar.js';

/**
 * Build the read-only summary model from crew-public sim state.
 *
 * @param {{ redAlert?: boolean,
 *           hullAggregate?: number|null, hullDestroyed?: number|null,
 *           stations?: Array<{id:string,name?:string,human_seeking?:boolean}>,
 *           stationHealth?: Object<string, number|null>,
 *           stationImportance?: Object<string, {unread?:boolean,critical?:boolean}>,
 *           objectives?: Array }} [input]
 * @returns {object} summary model — see the return literal.
 */
export function spectatorSummaryModel(input = {}) {
  const {
    redAlert, hullAggregate, hullDestroyed,
    stations, stationHealth, stationImportance, objectives,
  } = input;

  // Crew stations only (the same set the Hero Bar tabs), so an auxiliary or
  // non-crew station never appears on the summary.
  const defs = (stations || []).filter(st => st && st.human_seeking);
  const stationRows = defs.map(st => {
    // Authoritative host figure (issue #1100), never inferred from
    // recipient-scoped damage rows a spectator does not even receive.
    const rawHealth = stationHealth ? stationHealth[st.id] : undefined;
    const health = typeof rawHealth === 'number' ? rawHealth : null;
    const importance = (stationImportance && stationImportance[st.id]) || null;
    return {
      id: st.id,
      name: st.name || st.id,
      health,
      healthState: heroBarHealthState(health),
      importance,
      importanceState: heroBarImportanceState(importance),
    };
  });

  const objectiveRows = (objectives || []).map(o => ({
    id: o.id || o.text || '',
    text: o.text || '',
    textParams: o.text_params || null,
    mandatory: o.mandatory !== false,
    done: o.done != null ? o.done : (o.status === 'Completed'),
  }));

  const aggregate = typeof hullAggregate === 'number' ? hullAggregate : null;
  const destroyed = typeof hullDestroyed === 'number' ? hullDestroyed : null;

  return {
    alertState: redAlert ? 'red' : 'normal',
    hull: {
      aggregate,
      destroyed,
      // Discrete, threshold-free state so the readout is legible without
      // relying on colour (same idea as heroBarHealthState).
      state: aggregate == null ? 'none' : (aggregate >= 1 ? 'full' : 'damaged'),
    },
    stations: stationRows,
    objectives: objectiveRows,
  };
}

/**
 * Write a summary model into the host elements. Every element is optional so a
 * partially-mounted shell (boot race) still paints what it can.
 *
 * @param {{ alertEl?: Element, hullEl?: Element,
 *           stationsEl?: Element, objectivesEl?: Element,
 *           emptyObjectivesId?: string,
 *           model: object,
 *           translate?: (id: string, params?: object) => string,
 *           objectiveText?: (o: object) => string }} args
 */
export function renderSpectatorDom(args) {
  const { alertEl, hullEl, stationsEl, objectivesEl, model } = args;
  const tr = args.translate || (id => id);
  const objectiveText = args.objectiveText || (o => o.text);

  // Red-alert banner: a data attribute plus a label, so it reads without
  // relying on colour.
  if (alertEl) {
    alertEl.dataset.alert = model.alertState;
    alertEl.textContent = tr('client.spectator.alert.' + model.alertState);
  }

  // Ship-wide hull readout from the authoritative aggregate.
  if (hullEl) {
    hullEl.dataset.hull = model.hull.state;
    hullEl.textContent = model.hull.aggregate == null
      ? tr('client.spectator.hull.none')
      : tr('client.spectator.hull.readout', {
          pct: Math.round(model.hull.aggregate * 100),
        });
  }

  // Per-Station health rows, reusing the Hero Bar's non-colour cue strings.
  if (stationsEl) {
    stationsEl.innerHTML = '';
    for (const st of model.stations) {
      const row = stationsEl.ownerDocument.createElement('div');
      row.className = 'spectator-station-row';
      row.dataset.station = st.id;
      row.dataset.health = st.healthState;
      row.dataset.importance = st.importanceState;
      const name = stationsEl.ownerDocument.createElement('span');
      name.className = 'spectator-station-name';
      name.textContent = st.name;
      const health = stationsEl.ownerDocument.createElement('span');
      health.className = 'spectator-station-health';
      health.textContent = tr('client.hero.health.cue.' + st.healthState);
      row.append(name, health);
      stationsEl.appendChild(row);
    }
  }

  // Mission objectives (crew-public — the same list the captain console shows).
  if (objectivesEl) {
    objectivesEl.innerHTML = '';
    if (model.objectives.length === 0) {
      const empty = objectivesEl.ownerDocument.createElement('div');
      empty.className = 'spectator-objectives-empty';
      empty.textContent = tr(args.emptyObjectivesId || 'component.objectives.empty');
      objectivesEl.appendChild(empty);
    } else {
      for (const o of model.objectives) {
        const row = objectivesEl.ownerDocument.createElement('div');
        row.className = 'spectator-objective-row' + (o.done ? ' done' : '');
        row.dataset.done = o.done ? 'true' : 'false';
        row.textContent = objectiveText(o);
        objectivesEl.appendChild(row);
      }
    }
  }
}

// Expose for the non-module inline script in client.html.
if (typeof window !== 'undefined') {
  window.spectatorSummaryModel = spectatorSummaryModel;
  window.renderSpectatorDom = renderSpectatorDom;
}
