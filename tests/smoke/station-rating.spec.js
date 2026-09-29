// #1059/#1083: real cruiser content and depth projection; only transport is stubbed.
import { test, expect, createServerPage, readHostPeerId, createTestClient } from './fixtures';

test('authored stars change live depth without giving the holder AI actuator authority', { tag: '@core' }, async ({ context }) => {
  const host = await createServerPage(context);
  const crew = await createTestClient(context, await readHostPeerId(host), { name: 'Engineer' });
  await crew.send('SelectStation', { station: 'Engineering' });
  await crew.waitForMessage('StationAssigned');
  await crew.send('SetStationRating', { rating_name: 'Guided' });
  const rating = await crew.waitForMessage('RatingChanged');
  expect(rating.data.rating_name).toBe('Guided');
  await crew.send('SetReady', { ready: true });
  await crew.waitForMessage('GameStarted', 15_000);
  await crew.page.waitForFunction(() => window.__messages.some(message =>
    message.type === 'SimState'
      && message.data.snapshot.system_depths?.repair === 'Simplified'
      && message.data.snapshot.control_sources?.repair === 'Simplified'), undefined, { timeout: 15_000 });
  const label = await crew.page.evaluate(async () => {
    const { stationRatingLabel } = await import('/gui/station-rating.js');
    return stationRatingLabel(['Std', 'Guided', 'Simplified', 'Maintenance'], 'Guided');
  });
  expect(label).toBe('★★★ Guided');
  const before = await crew.page.evaluate(() => window.__messages.length);
  await crew.send('SetStationRating', { rating_name: 'Std' });
  await crew.page.waitForFunction(from => window.__messages.slice(from).some(message =>
    message.type === 'SimState'
      && message.data.snapshot.system_depths?.repair === 'Detailed'
      && message.data.snapshot.control_sources?.repair === 'Human'), before, { timeout: 15_000 });
  await crew.close();
});
