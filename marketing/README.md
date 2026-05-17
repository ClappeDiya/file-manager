# Marketing site — filemanager.clappe.com

The public marketing site, pricing page, docs portal, and download hub for FileManager.

## Stack

- Next.js 15 App Router + TypeScript 5 + Tailwind v3
- Shared design tokens via `@ufop/design-tokens` (CSS variables — same file the desktop app uses)
- Shared UI primitives via `@ufop/ui-components`
- Output: `standalone` Next.js build, served by `node server.js` in a Docker container

## Develop

From repo root:

```bash
pnpm install
pnpm --filter marketing dev   # http://localhost:3002
```

## Build

```bash
pnpm --filter marketing build
pnpm --filter marketing start
```

## Deploy

Built and pushed to GHCR via the `ghcr-dokploy-pipeline` workflow, then deployed to Dokploy. See repo-root deploy docs.

## Routes

| Path | Purpose |
|---|---|
| `/` | Home: hero, value props, screenshots, comparison, pricing teaser |
| `/download` | OS-detected download CTAs + Homebrew/winget commands + checksums |
| `/pricing` | Free / Business / Enterprise; waitlist form for Business |
| `/security` | Privacy stance, local-AI explanation, security disclosures |
| `/changelog` | Built from repo-root `CHANGELOG.md` |
| `/contact` | Contact form + support email |
| `/legal/{privacy,terms,refund}` | Required legal pages |

To add `/docs/*` (Nextra) see the open Phase 1 task.
