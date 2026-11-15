import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { runInNewContext } from 'node:vm';
import { describe, expect, it } from 'vitest';

async function policy(environment: string) {
  const configPath = fileURLToPath(new URL('../../next.config.js', import.meta.url));
  const module = { exports: {} as { headers(): Promise<{ headers: { key: string; value: string }[] }[]> } };
  runInNewContext(readFileSync(configPath, 'utf8'), { module, require: createRequire(import.meta.url), __dirname: dirname(configPath), process: { env: { NODE_ENV: environment } } });
  const rules = await module.exports.headers();
  return rules[0].headers.find(h => h.key === 'Content-Security-Policy')!.value;
}

describe('admin CSP', () => {
  it('allows the debugging evaluator required for development hydration', async () => {
    expect(await policy('development')).toContain("'unsafe-eval'");
  });
  it('keeps dynamic evaluation blocked for production and unspecified environments', async () => {
    for (const environment of ['production', 'test', '']) {
      const value = await policy(environment);
      expect(value).not.toContain("'unsafe-eval'");
      expect(value).toContain("frame-ancestors 'none'");
      expect(value).toContain("form-action 'self'");
    }
  });
});
