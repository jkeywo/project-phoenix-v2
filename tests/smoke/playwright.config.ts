import { defineConfig, devices } from '@playwright/test';
import path from 'path';

const distDir = path.resolve(__dirname, '../../dist');

export default defineConfig({
  testDir: '.',
  testMatch: '*.spec.ts',
  // Tests run sequentially — WASM is heavy; parallel runs OOM on CI
  workers: 1,
  timeout: 60_000,
  expect: { timeout: 30_000 },

  use: {
    baseURL: 'http://localhost:3000',
    // Capture traces on first retry to aid debugging
    trace: 'on-first-retry',
  },

  projects: [
    {
      name: 'chromium',
      use: {
        ...devices['Desktop Chrome'],
        launchOptions: {
          args: [
            '--autoplay-policy=no-user-gesture-required',
            // cpal's WASM audio backend stalls the Bevy update loop in headless
            // Chromium once InProgress audio entities are spawned.  Disabling the
            // Web Audio API makes cpal fail gracefully (BuildStreamError, no panic)
            // so the simulation keeps ticking.
            '--disable-web-audio',
          ],
        },
      },
    },
  ],

  webServer: {
    command: `npx serve "${distDir}" -p 3000 --no-clipboard`,
    url: 'http://localhost:3000',
    reuseExistingServer: !process.env.CI,
    timeout: 15_000,
  },

  reporter: process.env.CI
    ? [['github'], ['list'], ['json', { outputFile: '../../target/playwright-results.json' }]]
    : [['list'], ['html', { outputFolder: '../../target/playwright-report', open: 'never' }]],
});
