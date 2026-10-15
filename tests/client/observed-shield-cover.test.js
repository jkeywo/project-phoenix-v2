/** @vitest-environment jsdom */
import { afterEach, expect, it } from 'vitest';
import { projectSensorTracks } from '../../gui/sensor-tracks.js';
import '../../gui/components/ph-sensor-panel.js';
import '../../gui/components/ph-target-lock-card.js';
afterEach(() => document.body.replaceChildren());
const track = cover => ({ contact_id: 'held', selectable: true, predicted: false, stage: 'assessed', observed_tick: 10,
  assessment_tick: 12, position: [10, 0], position_error: 2, assessment: { protection: cover } });
it('shows only the held assessed ray with original sample time on Sensors and Tactical', () => {
  const view = projectSensorTracks([track({ incoming_sector: 91, blocked: true })], { range: 100, selected: 'held', tick: 100 });
  expect(view.target_protection).toEqual({ incoming_sector: 91, blocked: true, sample_tick: 12 });
  for (const kind of ['ph-sensor-panel', 'ph-target-lock-card']) {
    const element = document.createElement(kind); document.body.append(element); element.state = view;
    expect(element.shadowRoot.textContent).toContain('Blocked at incoming bearing 91°');
    expect(element.shadowRoot.textContent).toContain('Sample tick 12');
    element.state = { ...view, target_protection: { incoming_sector: 360, blocked: false } };
    expect(element.shadowRoot.textContent).not.toContain('Observed shield cover');
  }
});
it('never infers missing protection from snapshots or presents a malformed field as open', () => {
  for (const value of [null, {}, { incoming_sector: 0 }, { incoming_sector: -1, blocked: false }, { incoming_sector: 1.2, blocked: false }, { incoming_sector: 1, blocked: 0 }]) {
    expect(projectSensorTracks([track(value)], { range: 100, selected: 'held' }).target_protection).toBeNull();
  }
  const open = projectSensorTracks([track({ incoming_sector: 0, blocked: false })], { range: 100, selected: 'held' });
  expect(open.target_protection.blocked).toBe(false);
  expect(open.target_shield_freq).toBeNull(); expect(open.target_shields).toBeNull();
});
