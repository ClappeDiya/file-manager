import { describe, expect, it } from "vitest";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const script = path.resolve(__dirname, "../../scripts/check-licensing.mjs");
const run = (root?: string) => spawnSync(process.execPath, [script], { env: { ...process.env, ...(root ? { LICENSING_ROOT: root } : {}) }, encoding: "utf8" });

function fixture(files: Record<string, string>): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "ufop-lic-"));
  for (const [p, c] of Object.entries(files)) {
    fs.mkdirSync(path.dirname(path.join(dir, p)), { recursive: true });
    fs.writeFileSync(path.join(dir, p), c);
  }
  return dir;
}

const config = JSON.stringify({
  licenses: { "PolyForm-Shield-1.0.0": { file: "LICENSE", spdx: null } },
  components: [
    { name: "client", kind: "client", license: "PolyForm-Shield-1.0.0", paths: ["app"], manifests: ["app/package.json"] },
    { name: "console", kind: "console", license: "PolyForm-Shield-1.0.0", paths: ["admin"], manifests: ["admin/package.json"] },
  ],
});

describe("license boundary check", () => {
  it("passes on the repository", () => {
    const r = run();
    expect(r.stderr).toBe("");
    expect(r.status).toBe(0);
  });

  it("rejects client code importing the console and a missing license declaration", () => {
    const dir = fixture({
      "licensing.json": config,
      LICENSE: "x",
      "app/package.json": JSON.stringify({ name: "app", license: "SEE LICENSE IN ../LICENSE" }),
      "admin/package.json": JSON.stringify({ name: "admin" }),
            // Split so this test's own source is not read as a console import.
      "app/src/a.ts": "import { secret } from '" + "../../admin/src/lib/x';\n",
      "admin/src/lib/x.ts": "export const secret = 1;\n",
    });
    const r = run(dir);
    expect(r.status).toBe(1);
    expect(r.stderr).toContain("app/src/a.ts: imports console code");
    expect(r.stderr).toContain('admin/package.json: expected "license"');
    fs.rmSync(dir, { recursive: true, force: true });
  });
});
