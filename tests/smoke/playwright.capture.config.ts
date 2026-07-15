// Config for manual *.capture.ts verification aids (see dust-pfx.capture.ts).
//
// The main config restricts testMatch to '*.spec.ts' so captures never run in
// CI; this one opts them in explicitly.
//
// The smoke suite only asserts on messages and DOM, so it never needed the
// canvas to actually draw. Captures do: without a GL backend headless Chromium
// hands Bevy no WebGL2 context and the viewscreen renders pure black.
// SwiftShader provides a software one.
import base from './playwright.config';
import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
  ...base,
  testMatch: '*.capture.ts',
  timeout: 300_000,
  projects: [
    {
      name: 'chromium',
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
});
