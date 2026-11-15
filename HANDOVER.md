# Handover note — Claude session `claude/nice-ramanujan-bqq9cd`

Date: 2026-10-25
Session: https://claude.ai/code/session_017KionXLLW9aoxAPCE177Ym

## What this session did

No code changes. This session only recorded the repository's state for whoever picks up the work next.

## Repository state

| Item | State |
|---|---|
| Working branch | `claude/nice-ramanujan-bqq9cd` |
| Branch vs `master` | Same commit (`1487ac6`, the merge of ClappeDiya/file-manager#20). No commits ahead, nothing uncommitted, no stashes |
| Open PRs | None |
| Last merged PRs | [#20 Kopia parity](https://github.com/ClappeDiya/file-manager/pull/20) (2026-10-23), [#19 Gap closure](https://github.com/ClappeDiya/file-manager/pull/19) (2026-10-23), [#18 Versioned backup engine](https://github.com/ClappeDiya/file-manager/pull/18) (2026-10-22), [#17 Pane-to-pane copies via transfer engine](https://github.com/ClappeDiya/file-manager/pull/17), [#16 Wire dead features end-to-end](https://github.com/ClappeDiya/file-manager/pull/16) |

## Recent work (from #20, `feat/kopia-parity`)

- Repository format v2: envelope key, zstd, FastCDC chunking, ChaCha20-Poly1305, sharded layout, lock leases, clock-skew checks, checkpoints, read-only connections.
- `ufop agent` runs managed jobs on schedules, runs maintenance and applies organization policy templates.
- Optional desktop Prometheus exporter (K108). It is off by default and listens on 127.0.0.1 only.
- Simple and Advanced backup screens (K132), plus `SECURITY.md` (K130).
- Clippy 1.97 clean-up in `ufop-core`, `src-tauri` (including the Windows GNU target) and the CLI.
- Fixed a broken-pipe flake on gateway uploads, with a regression test.
- Portable mode on macOS now gives the window its own WKWebView data store.

## Still open

`planning/kopia-parity-checklist.md` has 95 items ticked and **1 open**:

- **Multi-arch `ufop-server` image build and smoke test** (`scripts/docker-build.sh --platforms linux/amd64,linux/arm64`).
  - So far only the dry-run plan has been tested and the Dockerfile reviewed.
  - The cloud container can't do the real build: it has no running Docker daemon, no QEMU/binfmt for arm64, and only about 3 GB of free disk.
  - **Next step:** run the build on a machine with Docker and buildx (plus arm64 emulation or a native arm64 builder), smoke-test both images, then tick the box.

Other planning docs to check before starting new work: `planning/gap-closure-plan.md`, `planning/build-sequence.md`, `docs/v1.5-roadmap.md`, `docs/release-checklist.md`, `docs/qa-checklist.md`.

## Picking up

```bash
git fetch origin master && git checkout -B <new-branch> origin/master
pnpm install
cd src-tauri && cargo test --lib        # desktop backend
cd crates/ufop-core && cargo test       # shared engine
pnpm test                               # frontend (Vitest)
```

Follow `CLAUDE.md`:
- Branch off `master` and open a PR.
- Don't commit screenshots, reports or other artifacts.
- Before calling a deploy done, run the deploy-verify-heal pipeline and check every route.
