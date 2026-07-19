import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    include: ['editor/tests/**/*.test.js', 'tests/client/**/*.test.js'],
    environment: 'node',
    // Loads assets/strings/strings.csv into gui/strings.js so t() resolves
    // real text in Node, where the browser boot module is a no-op.
    setupFiles: ['tests/client/setup-strings.js'],
  },
});
