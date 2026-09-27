import { expect } from '@playwright/test';

/** Open a one-GM fleet and wait for its authoritative start policy. */
export async function openReadyGmOnlyFleet(page) {
  expect(await page.evaluate(() => window.__hostFleetOpen())).toBe(true);
  await page.waitForFunction(() => {
    const state = window.__hostGmStartState?.();
    const policy = state?.policy;
    return state?.admitted === true
      && state.presentationReady === true
      && state.localValidation === true
      && policy?.connected_gms === 1
      && policy.connected_total === 1
      && policy.validation_passed === true
      && policy.started === false;
  }, undefined, { timeout: 30_000 });
}
