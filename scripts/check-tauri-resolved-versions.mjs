/** Match the packaging CLI's major/minor requirement using installed JS and locked Rust. */
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

export function mismatches(rust, javascript) {
  return Object.entries(javascript).flatMap(([name, version]) => {
    const crate = name === "@tauri-apps/api" || name === "@tauri-apps/cli" ? "tauri" : name.replace("@tauri-apps/plugin-", "tauri-plugin-");
    const locked = rust[crate];
    return !locked || locked.split(".").slice(0, 2).join(".") !== version.split(".").slice(0, 2).join(".")
      ? [`${name} ${version} does not match ${crate} ${locked ?? "missing"}; align major/minor versions before packaging`] : [];
  });
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const rust = Object.fromEntries([...readFileSync("src-tauri/Cargo.lock", "utf8").matchAll(/\[\[package\]\]\s+name = "([^"]+)"\s+version = "([^"]+)"/g)].map(m => [m[1], m[2]]));
  const manifest = JSON.parse(readFileSync("package.json", "utf8"));
  const javascript = Object.fromEntries(Object.keys({...manifest.dependencies, ...manifest.devDependencies}).filter(n => n.startsWith("@tauri-apps/")).map(name => [name, JSON.parse(readFileSync(`node_modules/${name}/package.json`, "utf8")).version]));
  const errors = mismatches(rust, javascript);
  for (const error of errors) console.error(`FAIL: ${error}`);
  if (errors.length) process.exitCode = 1;
  else console.log("OK: installed Tauri JavaScript and locked Rust major/minor versions match");
}
