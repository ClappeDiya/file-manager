import { defineConfig } from 'vitest/config';
import path from 'path';

export default defineConfig({
  resolve: { alias: { '@': path.resolve(__dirname, 'src') } },
  test: {
    environment: 'node',
    include: ['src/**/*.test.ts'],
    setupFiles: ['./src/__tests__/admin-setup.ts'],
    // SQLite is a process-global singleton per worker; files run in isolation.
    pool: 'forks',
    testTimeout: 20_000,
  },
});
