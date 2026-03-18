import js from "@eslint/js";
import tseslint from "typescript-eslint";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";
import globals from "globals";

export default tseslint.config(
  { ignores: ["dist", "node_modules", "admin", "cli", "packages", "src-tauri", "*.config.*"] },
  {
    extends: [js.configs.recommended, ...tseslint.configs.recommended],
    files: ["src/**/*.{ts,tsx}"],
    languageOptions: {
      ecmaVersion: 2022,
      globals: globals.browser,
    },
    plugins: {
      "react-hooks": reactHooks,
      "react-refresh": reactRefresh,
    },
    rules: {
      ...reactHooks.configs.recommended.rules,
      "react-refresh/only-export-components": ["warn", { allowConstantExport: true }],
      "@typescript-eslint/no-unused-vars": ["error", { argsIgnorePattern: "^_", varsIgnorePattern: "^_", caughtErrorsIgnorePattern: "^_" }],
      "@typescript-eslint/no-explicit-any": "warn",
      "no-console": ["warn", { allow: ["warn", "error", "log"] }],
      // react-hooks v7 new rules - too strict for existing codebase patterns
      "react-hooks/set-state-in-effect": "off",
      "react-hooks/no-components-during-render": "off",
      "react-hooks/static-components": "off",
      "react-hooks/no-access-state-before-declaration": "off",
      "react-hooks/incompatible-library": "off",
      // Assignment patterns used for keyboard navigation index clamping
      "no-useless-assignment": "off",
      "no-empty": ["error", { allowEmptyCatch: true }],
    },
  },
);
