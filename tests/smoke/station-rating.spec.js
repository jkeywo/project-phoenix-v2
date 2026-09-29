// #1059: real host admission and depth projection; only transport is stubbed.
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { test, expect, createServerPage, readHostPeerId, createTestClient } from './fixtures';

test('authored stars change live depth without giving the holder AI actuator authority', { tag: '@core' }, async ({ context }) => {
  const source = readFileSync(path.resolve(__dirname, '../../assets/entities/alliance_cruiser.toml'), 'utf8').replaceAll('\r\n', '\n');
  const boundary = '[[station]]\nid = "comms"';
  expect(source).toContain(boundary);
  const hull = source.replace(boundary, `[[station.rating]]
name = "SummarySmoke"
automated_systems = []
detailed_systems = ["power-reactor", "power-battery", "tractor", "umbilical"]

${boundary}`);
  await context.route('**/assets/entities/alliance_cruiser.toml', route => route.fulfill({ contentType: 'text/plain', body: hull }));
  const host = await createServerPage(context);
  const crew = await createTestClient(context, await readHostPeerId(host), { name: 'Engineer' });
  await crew.send('SelectStation', { station: 'Engineering' });
  await crew.waitForMessage('StationAssigned');
  await crew.send('SetStationRating', { rating_name: 'SummarySmoke' });
  const rating = await crew.waitForMessage('RatingChanged');
  expect(rating.data.rating_name).toBe('SummarySmoke');
  await crew.send('SetReady', { ready: true });
  await crew.waitForMessage('GameStarted', 15_000);
  await crew.page.waitForFunction(() => window.__messages.some(message =>
    message.type === 'SimState'
      && message.data.snapshot.system_depths?.repair === 'Simplified'
      && message.data.snapshot.control_sources?.repair === 'Simplified'), undefined, { timeout: 15_000 });
  const label = await crew.page.evaluate(async () => {
    const { stationRatingLabel } = await import('/gui/station-rating.js');
    return stationRatingLabel(['Std', 'SummarySmoke'], 'SummarySmoke');
  });
  expect(label).toBe('★ SummarySmoke');
  const before = await crew.page.evaluate(() => window.__messages.length);
  await crew.send('SetStationRating', { rating_name: 'Std' });
  await crew.page.waitForFunction(from => window.__messages.slice(from).some(message =>
    message.type === 'SimState'
      && message.data.snapshot.system_depths?.repair === 'Detailed'
      && message.data.snapshot.control_sources?.repair === 'Human'), before, { timeout: 15_000 });
  await crew.close();
});
