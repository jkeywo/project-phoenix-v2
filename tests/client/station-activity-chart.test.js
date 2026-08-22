// @vitest-environment jsdom
//
// Issue #1145 — the debug dock's station-activity chart.
//
// The renderer lives in gui/station-activity-chart.js precisely so it can be
// driven here without a browser or a WASM bundle: it is a pure function of the
// StationActivityPayload JSON the bridge publishes. These tests pin that it
// parses the payload defensively and draws the per-station, human-vs-AI split
// the chart exists to show.

import { describe, it, expect } from 'vitest';
import {
  parseStationActivity,
  buildStationActivityChart,
  renderStationActivityChart,
} from '../../gui/station-activity-chart.js';

/** A payload matching `codec::encode_station_activity`'s wire shape. */
function samplePayload() {
  return {
    schema_version: 1,
    bucket_ticks: 900,
    bucket_secs: 15,
    buckets: [
      {
        start_tick: 0,
        stations: [
          { station: 'helm', human: 3, ai: 1, offline: 0 },
          { station: 'weapons', human: 0, ai: 2, offline: 0 },
        ],
      },
      {
        start_tick: 900,
        stations: [{ station: 'helm', human: 1, ai: 0, offline: 0 }],
      },
    ],
  };
}

describe('parseStationActivity', () => {
  it('parses a well-formed payload', () => {
    const payload = parseStationActivity(JSON.stringify(samplePayload()));
    expect(payload).not.toBeNull();
    expect(payload.buckets).toHaveLength(2);
  });

  it('returns null for an empty string (before the first publish)', () => {
    expect(parseStationActivity('')).toBeNull();
  });

  it('returns null for malformed JSON rather than throwing', () => {
    expect(parseStationActivity('{not json')).toBeNull();
  });

  it('returns null when buckets is missing', () => {
    expect(parseStationActivity('{"schema_version":1}')).toBeNull();
  });
});

describe('buildStationActivityChart', () => {
  it('draws one lane per distinct station, sorted', () => {
    const chart = buildStationActivityChart(samplePayload(), { doc: document });
    const lanes = chart.querySelectorAll('.sa-station');
    expect(lanes).toHaveLength(2);
    expect(lanes[0].getAttribute('data-station')).toBe('helm');
    expect(lanes[1].getAttribute('data-station')).toBe('weapons');
  });

  it('splits each station-bucket into human and AI bars carrying their counts', () => {
    const chart = buildStationActivityChart(samplePayload(), { doc: document });
    const humanBar = chart.querySelector(
      '.sa-bar[data-station="helm"][data-bucket="0"][data-role="human"]',
    );
    const aiBar = chart.querySelector(
      '.sa-bar[data-station="helm"][data-bucket="0"][data-role="ai"]',
    );
    expect(humanBar).not.toBeNull();
    expect(humanBar.getAttribute('data-count')).toBe('3');
    expect(aiBar).not.toBeNull();
    expect(aiBar.getAttribute('data-count')).toBe('1');
  });

  it('omits a bar for a zero count', () => {
    const chart = buildStationActivityChart(samplePayload(), { doc: document });
    // weapons had zero human commands in bucket 0.
    const humanBar = chart.querySelector(
      '.sa-bar[data-station="weapons"][data-bucket="0"][data-role="human"]',
    );
    expect(humanBar).toBeNull();
    const aiBar = chart.querySelector(
      '.sa-bar[data-station="weapons"][data-bucket="0"][data-role="ai"]',
    );
    expect(aiBar.getAttribute('data-count')).toBe('2');
  });

  it('labels the human and AI legend', () => {
    const chart = buildStationActivityChart(samplePayload(), { doc: document });
    expect(chart.querySelector('.sa-legend-item[data-role="human"]')).not.toBeNull();
    expect(chart.querySelector('.sa-legend-item[data-role="ai"]')).not.toBeNull();
  });
});

describe('renderStationActivityChart', () => {
  it('renders the chart into a container from raw JSON', () => {
    const container = document.createElement('div');
    renderStationActivityChart(container, JSON.stringify(samplePayload()));
    expect(container.querySelector('.sa-chart')).not.toBeNull();
    expect(container.querySelectorAll('.sa-station')).toHaveLength(2);
  });

  it('shows the empty placeholder before any data arrives', () => {
    const container = document.createElement('div');
    renderStationActivityChart(container, '');
    expect(container.querySelector('.sa-empty')).not.toBeNull();
    expect(container.querySelector('.sa-chart')).toBeNull();
  });

  it('clears prior content on each render', () => {
    const container = document.createElement('div');
    renderStationActivityChart(container, JSON.stringify(samplePayload()));
    renderStationActivityChart(container, '');
    // The chart is gone, replaced by the placeholder — not appended alongside.
    expect(container.querySelectorAll('.sa-chart')).toHaveLength(0);
    expect(container.querySelectorAll('.sa-empty')).toHaveLength(1);
  });
});
