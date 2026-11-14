import { defineConfig } from 'vitest/config';
import path from 'path';

export default defineConfig({
  esbuild: { jsx: "automatic" },
  resolve: { alias: { '@': path.resolve(__dirname, 'src'), '@ufop/ui-components': path.resolve(__dirname, '../packages/ui-components/src/index.ts') } },
  test: {
    environment: 'node',
    include: ['src/**/*.test.{ts,tsx}'],
    setupFiles: ['./src/__tests__/admin-setup.ts'],
    // SQLite is a process-global singleton per worker; files run in isolation.
    pool: 'forks',
    maxWorkers: 2,
    testTimeout: 20_000,
  },
});
