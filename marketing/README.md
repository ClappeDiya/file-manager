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
| `/docs`, `/docs/<slug>` | The published user docs (allow-list in `src/lib/docs.ts`, Markdown from repo-root `docs/`) with a search box (`/` focuses it) |
| `/docs/search-index.json` | Search index built at `next build` from the same Markdown (title, headings, text); no external service |
| `/changelog` | Built from repo-root `CHANGELOG.md` |
| `/contact` | Contact form + support email |
| `/legal/{privacy,terms,refund}` | Required legal pages |

### Docs and search

A doc is published by adding it to `PUBLISHED_DOCS` in `src/lib/docs.ts`; the page, its heading anchors and the
search index all come from that one list, so a published doc is searchable and a search hit always links to an
existing heading (`src/lib/docs-index.test.tsx` checks this against the real docs). The indexer and scorer are
in `src/lib/docs-search.ts`. Run the tests with `pnpm --filter marketing test`.

The docs link to each other with repo-relative paths (`backup-safety.md#fragment`); `DocMarkdown` rewrites them
through `resolveDocHref` in `src/lib/docs.ts`: a link to a published doc becomes `/docs/<slug>#fragment`, a link to a
doc that is not published goes to its page on GitHub. The same test file renders every published page and fails on a
link that ends in `.md`, names an unpublished slug or points at a heading that does not exist.

### Pricing copy

`src/lib/pricing.ts` holds the trial length, prices and unit (per enrolled machine, as the admin console sells seats) quoted on `/pricing`; the admin console's billing tests
(`admin/src/__tests__/billing.test.ts`) fail if they differ from what checkout does.
