// check-ipc-args.mjs — every tauriInvoke/tauriInvokeSafe call passes the
// argument NAMES its Rust #[tauri::command] declares.
//
// ipc-verify (dump-ipc-commands.sh) proves each command NAME exists; it can't
// see arguments. A frontend call sending `{ sourcePath }` to a command taking
// `path` compiles, type-checks and ships — and then fails every time with
// "missing required key path", usually swallowed by a `.catch`. This gate
// found ~15 such dead features at once (rename, SSH config lookup, S3
// versions, preflight, duplicate cleanup, history search, …).
//
// Rules (Tauri 2 defaults): Rust snake_case params become camelCase keys;
// State/AppHandle/Window/Webview params are injected, not sent; `Option<T>`
// params may be omitted; unknown keys are ignored by Tauri but are reported
// here because they always mean a typo or a stale call.
//
// Uses the TypeScript compiler API, so nested objects, template literals,
// comments and multi-line calls are read exactly. Calls whose argument is
// not an object literal (a variable, a spread) are skipped.

import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const ts = require("typescript");
const ROOT = path.resolve(path.dirname(new URL(import.meta.url).pathname), "..");

function walk(dir, exts, out = []) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) {
      if (e.name === "node_modules" || e.name === "target" || e.name === "__tests__") continue;
      walk(p, exts, out);
    } else if (exts.some((x) => e.name.endsWith(x)) && !/\.test\.tsx?$/.test(e.name)) {
      out.push(p);
    }
  }
  return out;
}

// ── Rust side ──
const INJECTED = /\b(State|AppHandle|Window|WebviewWindow|Webview)\b/;
const commands = new Map();
const camel = (s) => s.replace(/^_+/, "").replace(/_([a-z0-9])/g, (_, c) => c.toUpperCase());

function splitTopLevel(s) {
  const parts = [];
  let depth = 0;
  let cur = "";
  for (const ch of s) {
    if ("<([".includes(ch)) depth++;
    if (">)]".includes(ch)) depth--;
    if (ch === "," && depth === 0) {
      parts.push(cur);
      cur = "";
    } else cur += ch;
  }
  parts.push(cur);
  return parts.map((p) => p.trim()).filter(Boolean);
}

for (const file of walk(path.join(ROOT, "src-tauri/src"), [".rs"])) {
  const src = fs.readFileSync(file, "utf8");
  const re = /#\[tauri::command[^\]]*\]\s*(?:(?:#\[[^\]]*\]|\/\/\/[^\n]*)\s*)*pub\s+(?:async\s+)?fn\s+(\w+)\s*(?:<[^>]*>)?\s*\(/g;
  let m;
  while ((m = re.exec(src))) {
    // Balanced-paren scan for the parameter list.
    let i = re.lastIndex;
    let depth = 1;
    const start = i;
    while (i < src.length && depth > 0) {
      if (src[i] === "(") depth++;
      else if (src[i] === ")") depth--;
      i++;
    }
    const params = src.slice(start, i - 1).replace(/\/\/[^\n]*/g, "");
    const required = new Set();
    const all = new Set();
    for (const p of splitTopLevel(params)) {
      const colon = p.indexOf(":");
      if (colon < 0) continue;
      const name = p.slice(0, colon).replace(/^mut\s+/, "").trim();
      const type = p.slice(colon + 1).trim();
      if (INJECTED.test(type)) continue;
      const key = camel(name);
      all.add(key);
      if (!/^Option\s*</.test(type)) required.add(key);
    }
    commands.set(m[1], { required, all, file: path.relative(ROOT, file) });
  }
}

// ── Frontend side ──
const problems = [];
for (const file of walk(path.join(ROOT, "src"), [".ts", ".tsx"])) {
  const text = fs.readFileSync(file, "utf8");
  const sf = ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true, file.endsWith("x") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
  const visit = (node) => {
    if (
      ts.isCallExpression(node) &&
      ts.isIdentifier(node.expression) &&
      (node.expression.text === "tauriInvoke" || node.expression.text === "tauriInvokeSafe") &&
      node.arguments.length > 0 &&
      ts.isStringLiteralLike(node.arguments[0])
    ) {
      const name = node.arguments[0].text;
      const where = `${path.relative(ROOT, file)}:${sf.getLineAndCharacterOfPosition(node.getStart()).line + 1}`;
      const cmd = commands.get(name);
      if (!cmd) {
        problems.push(`${where}  ${name}: no #[tauri::command] with this name`);
      } else {
        const arg = node.arguments[1];
        let keys = null;
        if (!arg || (ts.isIdentifier(arg) && arg.text === "undefined")) keys = new Set();
        else if (ts.isObjectLiteralExpression(arg)) {
          keys = new Set();
          for (const prop of arg.properties) {
            if (ts.isSpreadAssignment(prop)) {
              keys = null;
              break;
            }
            const n = prop.name;
            if (n && (ts.isIdentifier(n) || ts.isStringLiteral(n))) keys.add(n.text);
          }
        }
        if (keys) {
          const missing = [...cmd.required].filter((k) => !keys.has(k));
          const extra = [...keys].filter((k) => !cmd.all.has(k));
          if (missing.length || extra.length) {
            const bits = [];
            if (missing.length) bits.push(`missing ${missing.join(", ")}`);
            if (extra.length) bits.push(`unknown ${extra.join(", ")}`);
            problems.push(`${where}  ${name}: ${bits.join("; ")}  (Rust: ${cmd.file}; expects ${[...cmd.all].join(", ") || "no arguments"})`);
          }
        }
      }
    }
    ts.forEachChild(node, visit);
  };
  visit(sf);
}

if (problems.length) {
  console.error(`FAIL: ${problems.length} tauriInvoke call(s) don't match their Rust command's arguments:`);
  for (const p of problems) console.error(`  ${p}`);
  process.exit(1);
}
console.log(`OK: every tauriInvoke call's arguments match its Rust command (${commands.size} commands).`);
