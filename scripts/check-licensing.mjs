#!/usr/bin/env node
/**
 * Enforces the client / console license boundary declared in licensing.json:
 *  1. every component manifest declares the component's license file;
 *  2. client and shared code never imports console code (TS/JS imports,
 *     Cargo path dependencies), and shared code never imports client code;
 *  3. the console links no client crate (it speaks the agent protocol).
 * Exit code 1 lists every violation.
 */
import fs from 'node:fs';
import path from 'node:path';

const root = process.env.LICENSING_ROOT ? path.resolve(process.env.LICENSING_ROOT) : path.resolve(path.dirname(new URL(import.meta.url).pathname), '..');
const cfg = JSON.parse(fs.readFileSync(path.join(root, 'licensing.json'), 'utf8'));
const problems = [];
const rel = (p) => path.relative(root, p).split(path.sep).join('/');
const owner = (file) => cfg.components.find((c) => c.paths.some((p) => file === p || file.startsWith(p + '/')));

for (const c of cfg.components) {
  const lic = cfg.licenses[c.license];
  if (!lic) problems.push(`${c.name}: unknown license "${c.license}"`);
  for (const m of c.manifests) {
    const file = path.join(root, m);
    if (!fs.existsSync(file)) {
      problems.push(`${c.name}: manifest ${m} is missing`);
      continue;
    }
    const text = fs.readFileSync(file, 'utf8');
    const licenseFile = rel(path.join(root, lic?.file ?? 'LICENSE'));
    const expectRel = path.relative(path.dirname(file), path.join(root, licenseFile)).split(path.sep).join('/');
    if (m.endsWith('.toml')) {
      const ok = lic?.spdx ? new RegExp(`^license\\s*=\\s*"${lic.spdx}"`, 'm').test(text) : new RegExp(`^license-file\\s*=\\s*"${expectRel.replace(/\./g, '\\.')}"`, 'm').test(text);
      if (!ok) problems.push(`${m}: expected ${lic?.spdx ? `license = "${lic.spdx}"` : `license-file = "${expectRel}"`}`);
      for (const dep of text.matchAll(/path\s*=\s*"([^"]+)"/g)) {
        const target = rel(path.resolve(path.dirname(file), dep[1]));
        const to = owner(target);
        if (to && to.kind === 'console' && c.kind !== 'console') problems.push(`${m}: depends on console code ${target}`);
        if (to && c.kind === 'console' && to.kind === 'client') problems.push(`${m}: the console links client crate ${target}`);
      }
    } else {
      const pkg = JSON.parse(text);
      const want = lic?.spdx ?? `SEE LICENSE IN ${expectRel}`;
      if (pkg.license !== want) problems.push(`${m}: expected "license": "${want}" (found ${JSON.stringify(pkg.license ?? null)})`);
    }
  }
}

const IMPORT = /(?:from\s+|import\s*\(\s*|require\s*\(\s*)['"]([^'"]+)['"]/g;
const SKIP = new Set(['node_modules', 'target', 'dist', '.next', '.claude', '.git']);
function walk(dir, out) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    if (SKIP.has(e.name)) continue;
    const p = path.join(dir, e.name);
    if (e.isDirectory()) walk(p, out);
    else if (/\.(m?[jt]sx?)$/.test(e.name)) out.push(p);
  }
  return out;
}
for (const c of cfg.components.filter((x) => x.kind !== 'console')) {
  for (const base of c.paths) {
    const dir = path.join(root, base);
    if (!fs.existsSync(dir) || !fs.statSync(dir).isDirectory()) continue;
    for (const file of walk(dir, [])) {
      const src = fs.readFileSync(file, 'utf8');
      for (const m of src.matchAll(IMPORT)) {
        const spec = m[1];
        if (!spec.startsWith('.')) continue;
        const target = owner(rel(path.resolve(path.dirname(file), spec)));
        if (!target || target === c) continue;
        if (target.kind === 'console') problems.push(`${rel(file)}: imports console code "${spec}"`);
        if (c.kind === 'shared' && target.kind === 'client') problems.push(`${rel(file)}: shared package imports client code "${spec}"`);
      }
    }
  }
}

if (problems.length) {
  console.error(`License boundary check failed (${problems.length}):\n- ${problems.join('\n- ')}`);
  process.exit(1);
}
console.log(`License boundary OK: ${cfg.components.length} components (${cfg.components.map((c) => `${c.name}=${c.kind}`).join(', ')})`);
