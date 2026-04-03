import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    testTimeout: 300_000,
    hookTimeout: 120_000,
    include: ['test/**/*.test.ts'],
  },
});
