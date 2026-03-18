import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "path";
const host = process.env.TAURI_DEV_HOST;
export default defineConfig(async () => ({
    plugins: [react(), tailwindcss()],
    resolve: {
        alias: {
            "@": path.resolve(__dirname, "./src"),
            "@ufop/design-tokens": path.resolve(__dirname, "./packages/design-tokens/src/index.ts"),
            "@ufop/ui-components": path.resolve(__dirname, "./packages/ui-components/src/index.ts"),
        },
    },
    clearScreen: false,
    server: {
        port: 1420,
        strictPort: true,
        host: host || false,
        hmr: host
            ? {
                protocol: "ws",
                host,
                port: 1421,
            }
            : undefined,
        watch: {
            ignored: ["**/src-tauri/**"],
        },
    },
}));
