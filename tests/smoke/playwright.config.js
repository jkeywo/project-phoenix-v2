import { defineConfig, devices } from '@playwright/test';
import path from 'path';

const distDir = path.resolve(__dirname, '../../dist');

// Which port the built dist/ is served on.
//
// PHOENIX_SMOKE_PORT exists because of a specific incident, not for tidiness.
// `reuseExistingServer` below means a stale `npx serve` left running on 3000 —
// pointed at a dist/ built hours earlier, or at another worktree's — is silently
// adopted, and the whole suite then times out against a bundle that has nothing
// to do with the change under test. Two of three 0/17 runs during #1113 were
// exactly that. Setting this variable moves the port AND switches reuse off, so
// a run that sets it cannot inherit somebody else's server.
const port = Number(process.env.PHOENIX_SMOKE_PORT || 3000);
const ownPort = !!process.env.PHOENIX_SMOKE_PORT;

export default defineConfig({
  testDir: '.',
  testMatch: '*.spec.js',
  // Tests run sequentially — WASM is heavy; parallel runs OOM on CI
  workers: 1,
  timeout: 60_000,
  expect: { timeout: 30_000 },

  use: {
    baseURL: `http://localhost:${port}`,
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
    command: `npx serve "${distDir}" -p ${port} --no-clipboard`,
    url: `http://localhost:${port}`,
    reuseExistingServer: !process.env.CI && !ownPort,
    timeout: 15_000,
  },

  reporter: process.env.CI
    ? [['github'], ['list'], ['json', { outputFile: '../../target/playwright-results.json' }]]
    : [['list'], ['html', { outputFolder: '../../target/playwright-report', open: 'never' }]],
});
