# Contributing & Development

## Project layout

```
FileManager/
├── src/                              ← Tauri frontend (React + TypeScript + Vite)
├── src-tauri/                        ← Rust backend (Tauri 2 + engines + ~30 command modules)
├── admin/                            ← Next.js admin console + customer portal
├── marketing/                        ← Next.js marketing site (filemanager.clappe.com)
├── cli/                              ← Rust CLI binary
├── packages/                         ← Shared workspace packages
│   ├── design-tokens/                ← Design tokens consumed by all UI apps
│   └── ui-components/                ← Shared React component library
├── docs/                             ← Product + technical docs (rendered via Nextra)
│   ├── PRD-V5-Definitive.md          ← Master PRD
│   └── AI-Agent-Governance-Addendum.md
├── ai-agents/                        ← AI agent configurations (development tooling)
│   ├── global-policy.yaml            ← Shared governance rules
│   ├── agent-index.yaml              ← Agent registry
│   ├── core/                         ← 11 core agents
│   ├── support/                      ← 8 support agents
│   └── workflows/                    ← Task bundle templates
└── planning/                         ← 42-sprint delivery plan
```

## Tech stack

- **Desktop:** Tauri 2 + Rust (engines, command handlers, SQLite + WAL) + React 19 + TypeScript 5 + Vite 6
- **State:** Zustand stores in `src/stores/` with `persist()` middleware
- **UI:** shadcn/ui + React Aria + Tailwind v4 + lucide-react icons
- **Data:** TanStack Query, TanStack Table, TanStack Virtual
- **Admin/Marketing:** Next.js 15 App Router + TypeScript + Tailwind v4
- **Package manager:** pnpm 10.27.0 (workspaces)

## Common commands

### Desktop app
```bash
pnpm dev              # Vite dev server (port 1420) — frontend only
pnpm tauri:dev        # Full Tauri desktop in dev mode (Rust + React)
pnpm tauri:build      # Native installer build
pnpm lint             # ESLint on src/
pnpm format           # Prettier on src/
pnpm test             # Vitest
```

### Rust backend
```bash
cd src-tauri
cargo check           # Fast compilation check
cargo test --lib      # ~733 library tests
cargo clippy          # Lint
```

### Admin console
```bash
pnpm admin:dev        # Port 3001
pnpm admin:build
```

### Marketing site
```bash
pnpm --filter marketing dev    # Port 3000
pnpm --filter marketing build
```

## Architecture quick reference

Full architectural notes (IPC boundary, AppState pattern, command module wiring, error handling, traits, migrations, connector protocol pattern, three-layer transfer engine) live in **[CLAUDE.md](CLAUDE.md)**. That file is the authoritative source for adding new features end-to-end.

When adding a new feature, follow the **end-to-end checklist** in CLAUDE.md — it covers:
1. Rust engine module
2. Tauri command module + `invoke_handler` registration
3. `AppState` wiring (both init paths!)
4. SQLite migration + test assertions
5. Zustand store
6. React panel component
7. Toolbar entry
8. Command palette entry

## AI-agent workflow (for contributors using Claude Code / Cursor / similar)

This repo is set up to be driven by AI coding agents. The configuration lives in `ai-agents/`:

1. Read `docs/PRD-V5-Definitive.md` for full product spec
2. Read `ai-agents/README.md` for agent loading instructions
3. Load `ai-agents/global-policy.yaml` + a specific agent for any task

Agent governance rules are in `docs/AI-Agent-Governance-Addendum.md`.

## Versioning

- PRD: V5 (Definitive)
- Agent Governance: v1.0
- Agent Pack: v1.0
- Sprint Plan: 42 sprints, 8 phases, 21 months

## License & contribution policy

This project is source-available under [PolyForm Shield 1.0.0](LICENSE). By submitting a contribution you agree it is licensed under the same terms. The Shield license prohibits using the software to build a competing product but otherwise permits commercial use, modification, and redistribution.
