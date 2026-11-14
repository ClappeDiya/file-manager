import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import path from "node:path";
export default defineConfig({
  root: path.resolve(__dirname, ".."),
  plugins: [react()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "src"),
      "@ufop/ui-components": path.resolve(
        __dirname,
        "../packages/ui-components/src/index.ts",
      ),
    },
  },
  test: {
    environment: "jsdom",
    setupFiles: [path.resolve(__dirname, "../src/__tests__/setup.ts")],
    include: ["admin/src/**/*.test.{ts,tsx}"],
    maxWorkers: 1,
  },
});
