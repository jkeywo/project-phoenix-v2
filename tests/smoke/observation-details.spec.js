import { test, expect } from '@playwright/test';
import { ts } from './strings';
import path from 'node:path';

// Real shipped custom elements and StringTable in a browser. The fixture supplies
// already-projected records; server-side permission and restore behavior have
// separate owner tests. No mocked element renderer or wall-clock age updates.
const PAGE = `<!doctype html><html lang="en"><meta charset="utf-8">
<style>body { background:#111827; color:#e5e7eb; font:16px sans-serif; max-width:760px; margin:24px; } ph-scan-readout,ph-repair-teams { display:block; margin:24px 0; }</style>
<ph-scan-readout id="scan"></ph-scan-readout><ph-repair-teams id="repair"></ph-repair-teams>
<script type="module">
import '/client/gui/components/ph-scan-readout.js';
import '/client/gui/components/ph-repair-teams.js';
window.observationProbeReady = true;
</script></html>`;
const observation = () => ({ subject: { kind: 'entity', uuid: 'fixture-target' },
  provenance: { kind: 'scan', system: 'fixture-scanner' }, observed_tick: 10,
  received_tick: 90, uncertainty: { kind: 'quantized', field: 'hidden_strength', step: 98765 } });

async function scan(page, now, received = 90) {
  const metadata = observation(); metadata.received_tick = received;
  await page.evaluate(({ now, metadata }) => {
    document.getElementById('scan').state = { scan: { capable: true, observation_tick: now,
      reading: { subject_uuid: 'fixture-target', subject_name: 'observation.source.scan',
        condition_fraction: .5, condition_step: .25, mass: 3, observation: metadata } } };
  }, { now, metadata });
}

test('recorded observation details retain sample age and repair contributor privacy', { tag: '@core' }, async ({ page }) => {
  const errors = []; page.on('pageerror', error => errors.push(error.message));
  await page.route('**/client/observation-details-probe', route => route.fulfill({ contentType: 'text/html', body: PAGE }));
  await page.goto('/client/observation-details-probe');
  await page.waitForFunction(() => window.observationProbeReady === true);
  await scan(page, 100);
  const panel = page.locator('#scan'), details = panel.locator('details');
  await details.locator('summary').click();
  await expect(details).toHaveAttribute('open', '');
  await expect(details.locator('[data-observation="sample"]')).toHaveText(ts('observation.tick', { tick: 10 }));
  await expect(details.locator('[data-observation="receipt"]')).toHaveText(ts('observation.tick', { tick: 90 }));
  await expect(details.locator('[data-observation="age"]')).toHaveText(ts('observation.ticks', { ticks: 90 }));
  await expect(details.locator('[data-observation="precision"]')).toHaveText(ts('observation.unknown'));
  await expect(panel).not.toContainText('hidden_strength'); await expect(panel).not.toContainText('98765');

  // A later receipt is not a fresh sample. Only the supplied simulation tick
  // advances age; no sleep or browser-time interpolation participates here.
  await scan(page, 100, 99);
  await expect(details.locator('[data-observation="age"]')).toHaveText(ts('observation.ticks', { ticks: 90 }));
  await scan(page, 120, 99);
  await expect(details).toHaveAttribute('open', '');
  await expect(details.locator('[data-observation="age"]')).toHaveText(ts('observation.ticks', { ticks: 110 }));
  await expect(panel.locator('.row .value').first()).toContainText('50%');

  await page.evaluate(metadata => {
    document.getElementById('repair').state = { teams: [], observation_tick: 120, reports: [{
      group: { type: 'Console', data: 'flight' }, station_label: 'observation.source.system_report',
      deficit: 7654321, observations: [metadata, { provenance: { kind: 'repair_team', system: 'team-system' },
        observed_tick: null, received_tick: 115, uncertainty: { kind: 'unspecified' } }],
    }] };
  }, observation());
  const reports = page.locator('#repair').locator('details');
  await expect(reports).toHaveCount(2);
  await reports.nth(1).locator('summary').click();
  await expect(reports.nth(1).locator('[data-observation="source"]')).toContainText(ts('observation.source.repair_team'));
  await expect(reports.nth(1).locator('[data-observation="age"]')).toHaveText(ts('observation.unknown'));
  await expect(page.locator('#repair')).not.toContainText('7654321');
  await expect(reports.locator('button')).toHaveCount(0);
  await page.screenshot({ path: path.join(__dirname, '../../.phoenix/increment-4-observation-details.png'), fullPage: true });
  await page.evaluate(() => {
    document.getElementById('repair').state = { teams: [], reports: [] };
    document.getElementById('scan').state = { scan: { capable: true, reading: null } };
  });
  await expect(reports).toHaveCount(0); await expect(details).toHaveCount(0);
  expect(errors).toEqual([]);
});
