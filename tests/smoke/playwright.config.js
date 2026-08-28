import { defineConfig, devices } from '@playwright/test';
import path from 'path';

const distDir = path.resolve(__dirname, '../../dist');

// Which port THIS checkout's `dist/` is served on. CI and an ordinary local run
// want 3000 and get it. The override exists because `reuseExistingServer` below
// will happily adopt a server somebody else started — and when two worktrees of
// this repo are open at once, which is the normal way work happens here, that
// server is publishing the OTHER checkout's bundle. The failure is silent and
// genuinely baffling: every spec runs against a build you did not make, and the
// evidence (a page missing code you can see on disk) points nowhere near the
// port. Set PHOENIX_SMOKE_PORT to give a second worktree its own.
const PORT = Number(process.env.PHOENIX_SMOKE_PORT || 3000);

export default defineConfig({
  testDir: '.',
  testMatch: '*.spec.js',
  // Tests run sequentially — WASM is heavy; parallel runs OOM on CI
  workers: 1,
  timeout: 60_000,
  expect: { timeout: 30_000 },

  use: {
    baseURL: `http://localhost:${PORT}`,
    // Capture traces on first retry to aid debugging
    trace: 'on-first-retry',
  },

  projects: [
    {
      name: 'chromium',
      // The message/DOM suite. `*.render.spec.js` is excluded because it needs
      // a GL backend this project deliberately does without — see below.
      testIgnore: /\.render\.spec\.js$/,
      use: {
        ...devices['Desktop Chrome'],
        launchOptions: {
          args: [
            '--autoplay-policy=no-user-gesture-required',
          ],
        },
      },
    },
    {
      // Does the viewscreen actually draw?
      //
      // The suite above asserts on wire messages and DOM, so it never needed a
      // canvas — and `src/server/bridge.rs` skips `RenderPlugin` entirely under
      // `navigator.webdriver` because Bevy's wgpu init panics on a GPU-less
      // runner. Correct for 135 message tests, and precisely why a render-graph
      // break could ship: nothing in CI ever drew a frame.
      //
      // This project supplies the software WebGL2 context those specs hide from
      // (the same SwiftShader args `playwright.capture.config.js` uses); the
      // specs themselves hide the `navigator.webdriver` flag so bridge.rs takes
      // the real render path. Kept as a separate project rather than flags on
      // the one above so the message suite's browser is unchanged.
      name: 'render',
      testMatch: /\.render\.spec\.js$/,
      use: {
        ...devices['Desktop Chrome'],
        launchOptions: {
          args: [
            '--autoplay-policy=no-user-gesture-required',
            '--use-gl=angle',
            '--use-angle=swiftshader',
            '--enable-unsafe-swiftshader',
            '--ignore-gpu-blocklist',
          ],
        },
      },
    },
  ],

  webServer: {
    command: `npx serve "${distDir}" -p ${PORT} --no-clipboard`,
    url: `http://localhost:${PORT}`,
    reuseExistingServer: !process.env.CI,
    timeout: 15_000,
  },

  reporter: process.env.CI
    ? [['github'], ['list'], ['json', { outputFile: '../../target/playwright-results.json' }]]
    : [['list'], ['html', { outputFolder: '../../target/playwright-report', open: 'never' }]],
});
