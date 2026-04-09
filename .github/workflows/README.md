# GitHub Actions — Disabled

This project **does not use GitHub Actions**. CI/CD runs **locally** and
publishes container images directly to **GHCR** (GitHub Container Registry)
from the developer's machine.

The two workflow files that used to live in this directory have been
renamed with a `.disabled` suffix so GitHub's workflow scanner ignores
them (it only picks up files ending in `.yml` or `.yaml`):

- `ci.yml.disabled` — former lint + test matrix for Rust + frontend
- `release.yml.disabled` — former tag-triggered Tauri release build

Both files are preserved as a reference for porting their steps into
local scripts if needed. **Do not rename them back to `.yml`** — that
would re-enable the workflows and start consuming Actions minutes.

## Why local CI/CD?

- Full control over the build environment.
- No Actions minutes billing.
- Direct publishing to GHCR from the developer's machine.
- Avoids cross-account artifact permissions and workflow secret setup.

## Re-enabling

If this policy is ever reversed, `git mv <file>.yml.disabled <file>.yml`
and push. Nothing else is required — the workflow contents remain
untouched.
