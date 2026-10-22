# Licensing and the client / console split

UFOP has three kinds of code:

| Kind | Components | Paths |
|---|---|---|
| **client** | desktop app, `ufop-core` engine, `ufop` CLI | `src/`, `src-tauri/`, `crates/ufop-core/`, `cli/` |
| **shared** | design tokens, UI components | `packages/design-tokens/`, `packages/ui-components/` |
| **console** | admin / fleet console | `admin/` |

`licensing.json` at the repository root lists each component, its kind, its license and its manifests. Every manifest declares that license: `license-file` in Cargo and `"license": "SEE LICENSE IN …"` in `package.json`.

## Current license

All components are licensed under the **PolyForm Shield License 1.0.0** (`LICENSE`). Each component carries its own declaration, so the client can be relicensed separately from the console (for example under an OSI license, as backup clients often are, while the console stays proprietary). Doing that takes three steps:

1. Add the new license text, e.g. `LICENSES/<id>.txt`, and an entry under `licenses` in `licensing.json`. Set its `spdx` id so manifests use `license = "<spdx>"`.
2. Change `license` for the client and shared components in `licensing.json`.
3. Update their manifests; `pnpm check:licensing` lists exactly which lines.

Choosing and granting a license is a decision for the copyright holder. This repository does not make that decision.

## The boundary

`pnpm check:licensing` (`scripts/check-licensing.mjs`, covered by `src/__tests__/licensing-boundary.test.ts`) fails if:

- a component manifest does not declare the component's license;
- client or shared code imports console code (relative TS/JS imports, Cargo `path` dependencies);
- shared packages import client code;
- the console links a client crate.

The console and clients talk only through the documented agent protocol over HTTPS (`docs/admin-console.md`). Either side can therefore be built, shipped and licensed without the other. A client built from `src-tauri/`, `crates/` and `cli/` contains no console code. The console contains no client code and runs against any client that speaks the protocol.
