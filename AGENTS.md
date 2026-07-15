# AGENTS.md

This file is the **Codex adapter** for the UFOP repository. It tells Codex how
to find its way around without duplicating what already lives in `CLAUDE.md`.

- **Canonical project knowledge** → `CLAUDE.md` (architecture, conventions,
  end-to-end checklist for adding a feature). Read it first.
- **Skills** Codex can load → `.agents/skills/` (Claude-format skills, kept in
  sync from `.claude/skills/` so both runtimes see the same playbooks).
- **Codex-only configuration** → `.codex/` (this directory holds Codex's
  `config.toml` and `agents/`, and is the only Codex-specific surface).

## Project map

```
.
├── AGENTS.md              ← this file (Codex adapter)
├── CLAUDE.md              ← canonical project guidance — read first
├── README.md, CHANGELOG.md, CONTRIBUTING.md, LICENSE
├── PRD-V5-Definitive.md   ← product spec
├── .codex/
│   ├── config.toml        ← Codex CLI configuration (tracked)
│   └── agents/            ← Codex agent definitions (.toml)
├── .agents/
│   └── skills/            ← Claude-format skills usable by Codex
├── .claude/               ← Claude Code config (gitignored)
├── src/                   ← React + Vite desktop UI
│   ├── file-manager.tsx   ← root orchestrator
│   ├── components/, stores/, hooks/, lib/
├── src-tauri/             ← Rust core engine (Tauri 2.0)
│   ├── src/commands/      ← #[tauri::command] handlers (~30 modules)
│   ├── src/storage/       ← SQLite pool + migrations + repository
│   ├── src/connectors/    ← 17 protocol connectors
│   └── src/core/          ← AppError, traits
├── admin/                 ← Next.js admin console (port 3001 dev)
├── marketing/             ← Marketing site
├── cli/                   ← UFOP CLI
├── packages/
│   ├── design-tokens/     ← @ufop/design-tokens
│   └── ui-components/     ← @ufop/ui-components
├── docs/, planning/, scripts/, public/
└── docker-compose.ghcr.yml, releases-proxy/, ai-agents/
```

## Where Codex differs from Claude

- **Skills directory.** Codex loads skills from `.agents/skills/` (not
  `.codex/skills/`). The files are the same Markdown-with-frontmatter format
  Claude uses, kept in sync from `.claude/skills/` — when you update a skill,
  edit `.claude/skills/<name>/SKILL.md` and re-copy it into `.agents/skills/`.
- **Agent definitions.** Claude agents are `.md` files in `.claude/agents/`;
  Codex agents are `.toml` files in `.codex/agents/`. There are no agents
  defined yet — add one by dropping a `<name>.toml` into `.codex/agents/` with
  a `developer_instructions = """..."""` block.
- **Configuration.** Codex reads `.codex/config.toml`. Treat it as
  team-shared; per-developer overrides go in `*.local.toml` (gitignored).
- **Permissions and secrets.** Codex's `.codex/auth.json` and any local
  override files are gitignored. Never commit them.

## Conventions and runtime context

All conventions — IPC boundary, AppState pattern, error model, serde tag
rules, migration version-count assertions, the "adding a new feature"
checklist, and the connector / transfer-engine architecture — are documented
in `CLAUDE.md` and apply identically when Codex is driving. Defer to that
file rather than restating it here.

## Common commands

See `CLAUDE.md` § Commands for the full list. Short reference:

- Desktop UI: `pnpm dev`, `pnpm tauri:dev`, `pnpm lint`, `pnpm test`, `pnpm build`
- Rust core: `cd src-tauri && cargo check && cargo test --lib`
- Admin console: `pnpm admin:dev`
- Package manager: pnpm 10.27.0 (workspaces: root, `packages/*`, `admin/`)
