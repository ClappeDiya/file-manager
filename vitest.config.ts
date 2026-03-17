import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import path from "path";

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
      "@ufop/design-tokens": path.resolve(
        __dirname,
        "./packages/design-tokens/src/index.ts",
      ),
      "@ufop/ui-components": path.resolve(
        __dirname,
        "./packages/ui-components/src/index.ts",
      ),
    },
  },
  test: {
    globals: true,
    environment: "jsdom",
    setupFiles: ["./src/__tests__/setup.ts"],
    css: false,
    include: ["src/**/*.test.{ts,tsx}", "packages/**/*.test.{ts,tsx}"],
  },
});
