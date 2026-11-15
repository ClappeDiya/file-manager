import js from '@eslint/js';
import tseslint from 'typescript-eslint';
export default tseslint.config(
  { ignores: ['.next', 'node_modules', 'out'] },
  { files: ['src/**/*.{ts,tsx}'], extends: [js.configs.recommended, ...tseslint.configs.recommended], rules: {
    '@typescript-eslint/no-unused-vars': ['error', {argsIgnorePattern: '^_', varsIgnorePattern: '^_', ignoreRestSiblings: true}],
  } },
);
