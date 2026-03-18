# Tech Stack Decision — V4.1 Revision

## Decision Summary

| Surface | Technology | Rationale |
|---------|-----------|-----------|
| Desktop Shell | Tauri 2.0 | Cross-platform, small binary, native webview, capability model |
| Core Engine | Rust (tokio) | File ops, transfer, sync, compat, encryption — all privileged logic |
| Desktop UI | React + TypeScript + Vite | Client-side only, fast HMR, Tauri recommended setup |
| File List | TanStack Table | Headless table: sort, resize, select, keyboard nav |
| Virtualization | TanStack Virtual | 10K+ files rendered as ~60 DOM elements |
| Server State | TanStack Query | Async IPC caching, background refresh, optimistic updates |
| Client State | Zustand | Lightweight UI state management |
| Styling | Tailwind CSS + shadcn/ui | Utility CSS + accessible components on Radix primitives |
| Accessibility | React Aria | Headless a11y hooks for file list, tree, DnD, focus mgmt |
| Terminal | xterm.js + Rust PTY | Embedded terminal for local and remote sessions |
| Database | SQLite (rusqlite) | Local state, history, mappings, config |
| Admin Console | Next.js (full) + TypeScript | SSR, API routes, middleware — genuine web app needs |
| Design System | Shared tokens + shadcn/ui | Consistent design across desktop and admin |
| Packaging | Tauri bundler | .dmg, .msi/.exe, .deb/.AppImage |

## Why Vite, Not Next.js Static Export (Desktop)

1. Tauri frontend is a webview rendering static assets — no server, no Node.js
2. Next.js static export disables SSR, ISR, API routes, middleware, server components
3. Vite dev server starts in <300ms vs Next.js 2-5s
4. Tauri officially recommends Vite as primary React setup
5. Next.js remains correct for the admin console where server features matter

## Key Rust Crates

tokio, reqwest, ssh2/async-ssh2-lite, suppaftp, rust-s3, opendal,
unicode-normalization, notify, ring/aes-gcm, rusqlite, serde/serde_json,
tauri-plugin-*, mdns-sd/zeroconf, clap, indicatif

## Key Frontend Libraries

React 19.x, TypeScript 5.x, Vite 6.x, TanStack Table 8.x,
TanStack Virtual 3.x, TanStack Query 5.x, Zustand 5.x,
Tailwind CSS 4.x, shadcn/ui, React Aria 3.x, React Router 7.x,
xterm.js 5.x, Lucide React

## Design System Packages

- @ufop/design-tokens — colors, spacing, typography, shared across both apps
- @ufop/ui-components — shadcn/ui based, used by both apps
- @ufop/file-components — FileListTable, TreeView, PaneLayout (desktop only)
- @ufop/admin-components — PolicyEditor, AuditExplorer, etc (admin only)

## Monorepo Structure

pnpm workspaces or Turborepo. Changes to design tokens propagate to both apps.
