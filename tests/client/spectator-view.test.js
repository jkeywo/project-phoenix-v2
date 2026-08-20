import { JSDOM } from 'jsdom';
import { describe, expect, it } from 'vitest';
import { spectatorSummaryModel, renderSpectatorDom } from '../../gui/spectator-view.js';

const stations = [
  { id: 'helm', name: 'Helm', human_seeking: true },
  { id: 'tactical', name: 'Tactical', human_seeking: true },
  // A non-crew / auxiliary station must never appear on the summary.
  { id: 'viewscreen', name: 'Viewscreen', human_seeking: false },
];

describe('spectatorSummaryModel', () => {
  it('classifies red-alert, ship hull and per-crew-station health', () => {
    const model = spectatorSummaryModel({
      redAlert: true,
      hullAggregate: 0.62,
      hullDestroyed: 0.1,
      stations,
      stationHealth: { helm: 1, tactical: 0.4 },
      stationImportance: { tactical: { critical: true } },
      objectives: [],
    });
    expect(model.alertState).toBe('red');
    expect(model.hull).toMatchObject({ state: 'damaged' });
    expect(Math.round(model.hull.aggregate * 100)).toBe(62);
    // Only the two human-seeking stations, in order; viewscreen excluded.
    expect(model.stations.map(s => s.id)).toEqual(['helm', 'tactical']);
    expect(model.stations[0]).toMatchObject({ healthState: 'healthy' });
    expect(model.stations[1]).toMatchObject({ healthState: 'damaged', importanceState: 'critical' });
  });

  it('treats a missing hull aggregate as the neutral no-damage-model state', () => {
    const model = spectatorSummaryModel({ redAlert: false, stations, stationHealth: {} });
    expect(model.alertState).toBe('normal');
    expect(model.hull.aggregate).toBeNull();
    expect(model.hull.state).toBe('none');
    // A station with no host health figure is neutral, never inferred.
    expect(model.stations[0].healthState).toBe('none');
  });

  it('carries objective status through (crew-public, done from wire status)', () => {
    const model = spectatorSummaryModel({
      objectives: [
        { id: 'a', text: 'objective.reach_beacon', mandatory: true, status: 'Active' },
        { id: 'b', text: 'objective.scan_derelict', mandatory: false, status: 'Completed' },
      ],
    });
    expect(model.objectives).toHaveLength(2);
    expect(model.objectives[0]).toMatchObject({ id: 'a', done: false, mandatory: true });
    expect(model.objectives[1]).toMatchObject({ id: 'b', done: true, mandatory: false });
  });
});

describe('renderSpectatorDom', () => {
  function dom() {
    const { window } = new JSDOM('<!doctype html><body>'
      + '<div id="alert"></div><div id="hull"></div>'
      + '<div id="stations"></div><div id="objectives"></div></body>');
    const d = window.document;
    return {
      d,
      alertEl: d.getElementById('alert'),
      hullEl: d.getElementById('hull'),
      stationsEl: d.getElementById('stations'),
      objectivesEl: d.getElementById('objectives'),
    };
  }
  // Identity translate so assertions read the string ids, not resolved copy.
  const translate = (id, params = {}) => id + (Object.keys(params).length
    ? '(' + Object.entries(params).map(([k, v]) => `${k}=${v}`).join(',') + ')'
    : '');

  it('paints the alert banner and hull readout with data attributes', () => {
    const els = dom();
    const model = spectatorSummaryModel({
      redAlert: true, hullAggregate: 0.5,
      stations, stationHealth: { helm: 1, tactical: 0.4 },
    });
    renderSpectatorDom({ ...els, model, translate });
    expect(els.alertEl.dataset.alert).toBe('red');
    expect(els.alertEl.textContent).toBe('client.spectator.alert.red');
    expect(els.hullEl.dataset.hull).toBe('damaged');
    expect(els.hullEl.textContent).toBe('client.spectator.hull.readout(pct=50)');
  });

  it('renders one row per crew station with a non-colour health cue', () => {
    const els = dom();
    const model = spectatorSummaryModel({
      stations, stationHealth: { helm: 1, tactical: 0.4 },
    });
    renderSpectatorDom({ ...els, model, translate });
    const rows = els.stationsEl.querySelectorAll('.spectator-station-row');
    expect(rows).toHaveLength(2);
    expect(rows[0].dataset.station).toBe('helm');
    expect(rows[0].dataset.health).toBe('healthy');
    expect(rows[1].dataset.health).toBe('damaged');
  });

  it('lists objectives and strikes through completed ones', () => {
    const els = dom();
    const model = spectatorSummaryModel({
      objectives: [
        { id: 'a', text: 'obj.a', status: 'Active' },
        { id: 'b', text: 'obj.b', status: 'Completed' },
      ],
    });
    renderSpectatorDom({ ...els, model, translate, objectiveText: o => o.text });
    const rows = els.objectivesEl.querySelectorAll('.spectator-objective-row');
    expect(rows).toHaveLength(2);
    expect(rows[0].dataset.done).toBe('false');
    expect(rows[1].dataset.done).toBe('true');
    expect(rows[1].className).toContain('done');
  });

  it('shows the empty label when there are no objectives', () => {
    const els = dom();
    const model = spectatorSummaryModel({ objectives: [] });
    renderSpectatorDom({ ...els, model, translate });
    expect(els.objectivesEl.querySelector('.spectator-objectives-empty').textContent)
      .toBe('component.objectives.empty');
  });
});
