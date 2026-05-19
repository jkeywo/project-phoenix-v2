import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    include: ['editor/tests/**/*.test.js'],
    environment: 'node',
  },
});
