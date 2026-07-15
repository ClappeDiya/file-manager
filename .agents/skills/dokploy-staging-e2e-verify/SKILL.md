---
name: dokploy-staging-e2e-verify
description: Use when performing staging deployment verification with visible browser E2E testing on Dokploy. Triggers on "verify staging", "staging e2e", "run staging tests", "browser verification staging", "staging checklist", "e2e staging verification", "test staging deploy", "staging browser test", or any request for evidence-based staging verification with browser-first E2E discipline. Three-layer architecture — Core Policy (never changes), Project Manifest (per-project), Execution Runbook (ordered procedure). Covers local candidate gates, Dokploy deploy, visible browser E2E, cleanup, reporting, and rollback. Platform is Dokploy (NOT Coolify). Tech stack context — UFOP project with Tauri 2.0 (Rust + React/Vite), Next.js admin console, pnpm workspaces, SQLite/rusqlite, Zustand, shadcn/ui, Tailwind CSS; also supports Django/DRF + Next.js, PostgreSQL, Redis, Celery stacks.
---

# Staging Deployment Verification + Visible Browser E2E Runbook — v6.3

## Universal Edition · DOKPLOY-ONLY · Structured, Evidence-First, Rollback-Ready

> **Purpose**
> Execute a guarded **staging deployment verification** and a full **visible, human-followable, browser-first** end-to-end validation of any Django / DRF + Next.js application deployed to **staging on Dokploy**.
>
> Designed for **Claude Code** or any similarly capable autonomous coding/testing agent.
>
> **How to use this for a new project:**
> Fill in Layer 2 (the manifest) with your project-specific values.
> Layer 1 (Core Policy) and Layer 3 (Execution) never change between projects.
>
> This document is intentionally split into three layers:
>
> 1. **Core Policy** — non-negotiable rules, severity/status model, evidence standard, rollback logic
> 2. **Dokploy Project Manifest** — project-specific hooks and Dokploy deployment values resolved per-project
> 3. **Execution Runbook** — ordered procedure for local gates, Dokploy deploy, visible browser E2E, cleanup, reporting, and rollback
>
> This structure is deliberate:
>
> * policy remains stable across all projects
> * project-specific values are isolated and swappable
> * Dokploy behavior is explicit and never guessed
> * execution is easy for the agent to follow
>
> ---
>
> ## IMPORTANT SCOPE STATEMENT
>
> This is **not** a code-editing or staging-hotfix runbook.
>
> **It does allow:**
>
> * approved local validation
> * image build and push
> * deploy/redeploy/restart through Dokploy
> * migrations through the approved Dokploy execution path
> * read-only verification commands
> * isolated staging test-data creation and cleanup
> * rollback through Dokploy
>
> **It does not allow:**
>
> * ad hoc code edits on staging
> * manual server repair via SSH
> * direct container manipulation outside Dokploy
> * undocumented manual environment/config changes on staging
> * direct schema manipulation
> * any undocumented workaround that bypasses Dokploy or the evidence rules

**UFOP Project Stack Context:**
- **Desktop App:** Tauri 2.0 shell + Rust core engine (tokio async) + React/TypeScript/Vite frontend
- **Admin Console:** Next.js + TypeScript (port 3001 dev)
- **State Management:** Zustand (with `persist()` middleware)
- **UI:** shadcn/ui + React Aria + Tailwind CSS + lucide-react icons
- **Data:** SQLite via rusqlite (`Arc<Mutex<Connection>>` pool, WAL mode)
- **Shared Packages:** `@ufop/design-tokens`, `@ufop/ui-components`
- **Package Manager:** pnpm 10.27.0 workspaces
- **IPC Boundary:** Tauri IPC commands (`#[tauri::command]` in Rust, `tauriInvoke<T>()` on frontend)
- **Deployment Platform:** Dokploy (exclusive — never Coolify)
- **Browser Tools:** Playwright MCP or Chrome DevTools MCP for visible browser verification

> **UFOP Adaptation Notes:** For UFOP, the "backend" may be a Rust service (not Django). The "frontend" is the Next.js admin console. The Tauri desktop app itself is not deployed to staging — only the admin console and any containerized backend services are. For UFOP-only deployments, set Django-specific hooks to empty/N/A and adapt the local validation hooks for `cargo check`/`cargo test` + `pnpm lint`/`pnpm test` instead. SQLite migrations run automatically via `all_migrations()` in `src-tauri/src/storage/migrations.rs`.

---

# LAYER 1 — CORE POLICY

---

## 0. BINDING EXECUTION DIRECTIVE

You are an autonomous **staging deployment verification and visible-browser E2E testing agent for applications deployed through Dokploy**.

This document is an **instruction set**, not a discussion document.

Immediately upon receiving this runbook:

* do **not** ask whether to proceed
* do **not** ask what to test
* do **not** offer scope options
* do **not** reduce scope
* do **not** silently skip items
* do **not** replace browser proof with backend-only proof
* do **not** mark items passed because they passed locally
* start execution from **Layer 2 / Manifest Resolution**

The checklist defines the feature scope.
The manifest defines the project-specific and Dokploy-specific wiring.
This runbook defines the operating discipline.

If context becomes constrained:

* checkpoint cleanly
* emit the exact resume point
* resume from the **first unfinished checklist item**

Running out of context is **never** permission to skip remaining work.

---

## 1. OPERATING PRINCIPLES

These principles override convenience at all times:

* **visible over hidden**
* **user-like over shortcut**
* **evidence over optimism**
* **screenshots over unsupported assertions**
* **Dokploy discipline over improvisation**
* **complete execution over partial sampling**
* **checkpoint over abandonment**
* **rollback readiness over wishful thinking**
* **real browser proof over backend proof**

---

## 2. STAGING MODE DEFINITION

This runbook applies to a **deployed staging environment managed by Dokploy**.

### Allowed

* local candidate validation
* image build and push
* deploy/redeploy through Dokploy
* migrations through Dokploy
* visible browser E2E testing in staging
* read-only validation commands explicitly permitted in this runbook
* isolated test data creation and cleanup using the run-specific prefix
* rollback through Dokploy

### Forbidden

* editing application code on staging
* SSHing into the server to make fixes
* modifying deployment files directly on the server
* manual direct container restarts outside Dokploy
* direct schema edits
* interfering with other applications on shared infrastructure
* bypassing login through token injection, direct cookie planting, or API-only auth
* exposing secrets in logs, reports, screenshots, or console output

---

## 3. RELATIONSHIP TO LOCAL VALIDATION

LOCAL validation is a **prerequisite**, not proof of staging correctness.

* LOCAL proves the revision behaves in controlled local validation.
* STAGING proves the deployed environment behaves correctly with staging-specific:

  * domains
  * networking
  * secrets
  * callbacks
  * CORS / CSRF
  * tenancy
  * external integrations
  * build/runtime deployment configuration
  * Dokploy-managed release behavior

A staging item may **never** be marked passed merely because the local equivalent passed.

---

## 4. STATUS MODEL

Only these statuses are allowed:

* `✅ PASSED`
* `❌ FAILED`
* `⚠️ PARTIAL`
* `⏭️ BLOCKED`

`SKIPPED` is never valid.

Every checklist item must receive one of the four valid statuses.

---

## 5. SEVERITY MODEL

| Severity   | Meaning                                                                                                                         | Typical Action                                          |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------- |
| Critical   | App crash, 500 on core routes, login impossible, health failure, security failure, major data corruption, tenant/domain failure | Rollback recommended immediately after evidence capture |
| High       | Core journey broken, major regression, serious data integrity issue, key integration failure                                    | Rollback recommended or redeploy after fix cycle        |
| Medium     | Secondary flow broken, significant non-fatal defect, material UX problem                                                        | Log for next local fix cycle                            |
| Low        | Minor non-blocking defect, cosmetic issue                                                                                       | Backlog                                                 |
| Regression | Passed locally but failed in staging                                                                                            | Flag separately in addition to severity                 |

Severity is mandatory for every failed or partial item.

---

## 6. ROLLBACK DECISION MATRIX

| Condition                               | Action                                                  |
| --------------------------------------- | ------------------------------------------------------- |
| Any Critical issue                      | Rollback recommended immediately after evidence capture |
| 2+ High issues                          | Rollback recommended                                    |
| 1 High + multiple Medium issues         | Rollback strongly recommended                           |
| Health endpoint failure after deploy    | Rollback recommended immediately                        |
| Multi-tenancy/domain mapping failure    | Usually Critical; rollback recommended                  |
| Only Medium / Low issues                | Do not auto-rollback; log for next fix cycle            |
| No Critical/High and core journeys pass | Approve staging as production candidate                 |

---

## 7. EVIDENCE STANDARD

A feature is not verified unless the evidence is sufficient for a human reviewer to follow what happened.

Minimum evidence for a tested checklist item:

1. visible browser navigation or interaction
2. screenshot of relevant page state
3. screenshot after the key result
4. concise written note describing what the screenshot shows
5. console and/or network evidence when relevant
6. explicit status assignment

For failed or blocked items, evidence must show the visible UI state, the visible symptom, the defect context, and enough detail for a human to understand the failure without guessing.

---

## 8. VISUAL BROWSER MANDATE

This is the highest-priority testing rule.

All staging verification must happen through a **visible, headed browser session** that a human could watch in real time.

If a human could not have watched the browser do it, it does **not** count as valid E2E verification.

### Required

* headed browser
* visible window
* human-followable pacing
* screenshot capture before/after meaningful state changes
* navigation through the real UI for first-time page visits when possible

### Forbidden substitutes

* headless-only execution
* backend-only proof for UI behavior
* curl/httpie as primary evidence for UI correctness
* API-only login instead of visible form login
* token/cookie injection to bypass login
* direct deep-link opening for first visit when visible UI navigation exists
* batch-marking similar items as passed
* inferring untested items as passed

### Preferred Playwright settings

```javascript
use: {
  headless: false,
  slowMo: 300,
  video: 'on',
  trace: 'on'
}
```

### UFOP Browser Tool Notes

- Use Playwright MCP (`mcp__playwright__*`) or Chrome DevTools MCP (`mcp__chrome-devtools__*`) for visible browser verification
- For the UFOP admin console (Next.js), verify shadcn/ui component rendering, Tailwind CSS class application, and lucide-react icon loading
- Watch for Zustand persist rehydration flashes and React/Next.js hydration mismatch warnings
- TanStack Table/Virtual/Query components need special attention for data loading states

---

## 9. FIRST-VISIT NAVIGATION RULE

For the **first visit** to any page reachable through the visible product UI, navigate through the visible UI. Do not open the route directly unless one of these exceptions applies:

1. the page is login/logout/startup related
2. the page is intentionally deep-link-only
3. route-access testing explicitly requires direct access
4. a blocker prevents normal UI navigation and direct access is needed for evidence capture
5. a crash/reload/recovery step requires it

---

## 10. VBEP SELF-CHECK

Before assigning a final status to any checklist item, perform this self-check:

```text
VBEP SELF-CHECK
Q1: Did I reach or open this page in the visible browser?
Q2: Did I capture a screenshot of the relevant page state?
Q3: Did I perform the UI interaction in the visible browser?
Q4: Did I capture the result state?
Q5: Did I describe what the screenshot(s) show?

ALL YES = status may be updated
ANY NO = complete proper browser validation first
```

---

## 11. SECRET HANDLING POLICY

This policy is mandatory. Never relax it.

**Never:**

* print secrets in terminal logs, reports, or screenshots
* echo secret env vars directly
* write tokens/passwords into markdown artifacts
* commit temp credential files
* store secrets inside publicly readable artifact directories
* paste raw headers containing credentials into reports

**Always:**

* redact tokens, cookies, API keys, passwords, signed URLs, and auth headers
* store temporary test credentials only in restricted files
* use minimal-retention temporary files for sensitive values
* remove transient secret-bearing files after their operational need ends
* ensure state directories are permission-restricted

Required file permission posture for credential/state files:

```bash
chmod 700 "$STATE_DIR"  # state directory
find "$STATE_DIR" -type f -name "*.creds" -exec chmod 600 {} \; 2>/dev/null || true
```

Reports must contain **references** to secret locations only when strictly necessary, never the secret contents themselves.

### Test account model

This runbook supports **two valid test account models**. The project manifest must declare which model is in use:

1. **Pre-provisioned account model**

   * `E2E_TEST_EMAIL` and `E2E_TEST_PASSWORD` are provided from environment variables or secret storage.
   * No user creation step is required unless the project explicitly mandates per-run provisioning.

2. **Per-run generated account model**

   * the runbook creates an isolated E2E test user during execution using `STAGING_TEST_USER_CREATE_SNIPPET`
   * credentials are generated or assigned during the run
   * the generated credentials must be stored only in restricted files under `STATE_DIR`
   * cleanup is mandatory at the end of the run

**Selection rule:**

* If the manifest sets `TEST_ACCOUNT_MODE=preprovisioned`, use the environment-supplied credentials and do not create a new account unless the project requires supplementary setup.
* If the manifest sets `TEST_ACCOUNT_MODE=per_run`, create and clean up the account during this run.

**Pre-provisioned account requirements:**

```bash
export E2E_TEST_EMAIL="${E2E_TEST_EMAIL:?E2E_TEST_EMAIL is required}"
export E2E_TEST_PASSWORD="${E2E_TEST_PASSWORD:?E2E_TEST_PASSWORD is required}"
```

Never hardcode test credentials in this runbook, in test scripts, or in any committed file. Store them only in `.env.test.local` (gitignored) or CI/CD secret storage.

---

## 12. BLOCKER AND PATTERN-FAILURE POLICY

A blocker is not permission to skip downstream work.

If a blocker occurs:

1. capture evidence quickly
2. register the blocker
3. continue attempting dependent items
4. mark them appropriately based on evidence

Escalate to a **pattern failure** when:

* 3 consecutive items fail from the same root cause
* 5 items across sections show the same systemic failure
* the backend or frontend becomes broadly unreachable

Investigate briefly, classify, record, and continue with unaffected scope.

---

## 13. TOKEN BUDGET AND BATCH MANAGEMENT

For checklists exceeding 100 items, batching is mandatory.

**Default batch size:** 25 checklist items per execution segment.

After completing each batch of 25:

1. save tracking-table checkpoint
2. save `run-state.json` with `last_completed_cl` and `next_cl_to_execute`
3. emit a segment summary before proceeding

If context is approaching its limit:

1. save the tracking table to `${CHECKLIST_DIR}/tracking-table-checkpoint-[N].md`
2. save `run-state.json` with exact resume pointer
3. emit a partial report with the resume point
4. stop cleanly

Running out of context is **never** permission to skip remaining items.

Batch handoff format:

```
--- BATCH [N] COMPLETE ---
Items completed this batch: CL-[start] to CL-[end]
Total completed: [N]
Checkpoint saved: ${CHECKLIST_DIR}/tracking-table-checkpoint-[N].md
Resume from: CL-[next]
--- END BATCH ---
```

---

## 14. FINAL ANTI-SKIP GATE

Before issuing the final verdict, all of the following must be true:

1. checklist item count matches tracking-table row count
2. every row has a valid status
3. every tested item has screenshot evidence
4. every significant blocker/critical event has supporting evidence
5. no item was passed using backend-only proof for UI behavior
6. final report exists at the canonical path

If any condition fails, execution is incomplete.

---

# LAYER 2 — DOKPLOY PROJECT MANIFEST

---

## 15. MANIFEST RESOLUTION RULE

Before any irreversible step, resolve the manifest completely enough to execute safely.

If a required manifest value cannot be derived confidently:

* log the gap in `{REPORT_DIR}/report/manifest-gaps.md`
* stop before the irreversible step that depends on it
* do **not** guess
* surface the gap explicitly to the operator within 10 minutes of attempting derivation

Required manifest gap format:

```
--- MANIFEST GAP ---
Hook: [HOOK_NAME]
Time spent on derivation: [N] minutes
Steps attempted: [list]
Action required: Operator must provide [HOOK_NAME] before execution can continue.
Runbook execution is PAUSED.
--- END MANIFEST GAP ---
```

---

## 16. CANONICAL RUN VARIABLES

Define and persist these before deployment/testing work begins.

```bash
STAGING_RUN_ID="$(date +%Y%m%d-%H%M%S)"
PROJECT_NAME="${PROJECT_NAME:-my-project}"

LOCAL_RUN_ID="${LOCAL_RUN_ID:?LOCAL_RUN_ID is required}"
CHECKLIST_FILE="${CHECKLIST_FILE:?CHECKLIST_FILE is required}"

DEPLOY_COMMIT="${DEPLOY_COMMIT:-$(git rev-parse --short HEAD 2>/dev/null || true)}"
DEPLOY_COMMIT_FULL="${DEPLOY_COMMIT_FULL:-$(git rev-parse HEAD 2>/dev/null || true)}"
DEPLOY_IMAGE_TAG="${DEPLOY_IMAGE_TAG:-staging-${DEPLOY_COMMIT}-${STAGING_RUN_ID}}"

REPORT_ROOT="${PROJECT_NAME}/docs/playwright-devtools"
REPORT_DIR="${REPORT_ROOT}/${STAGING_RUN_ID}"

STATE_DIR="${REPORT_DIR}/artifacts/state"
LOG_DIR="${REPORT_DIR}/artifacts/logs"
CONSOLE_DIR="${REPORT_DIR}/artifacts/console"
DB_DIR="${REPORT_DIR}/artifacts/db-checks"
SCREENSHOT_SMOKE_DIR="${REPORT_DIR}/screenshots/smoke"
SCREENSHOT_DEEP_DIR="${REPORT_DIR}/screenshots/deep"
SCREENSHOT_RESPONSIVE_DIR="${REPORT_DIR}/screenshots/responsive"
SCREENSHOT_REGRESSION_DIR="${REPORT_DIR}/screenshots/regression"
SCREENSHOT_BLOCKERS_DIR="${REPORT_DIR}/screenshots/blockers"
REGRESSION_DIR="${REPORT_DIR}/artifacts/regression"
CHECKLIST_DIR="${REPORT_DIR}/artifacts/checklists"
CI_DIR="${REPORT_DIR}/artifacts/ci-gates"
TIMING_DIR="${REPORT_DIR}/artifacts/timing"
REPORT_SUBDIR="${REPORT_DIR}/report"

TEST_PREFIX="e2e_staging_${STAGING_RUN_ID}"
E2E_ADMIN_EMAIL="${TEST_PREFIX}_admin@test.local"
TEST_ACCOUNT_MODE="${TEST_ACCOUNT_MODE:?TEST_ACCOUNT_MODE must be 'preprovisioned' or 'per_run'}"

STAGING_BACKEND_URL="${STAGING_BACKEND_URL:?STAGING_BACKEND_URL is required}"
STAGING_FRONTEND_URL="${STAGING_FRONTEND_URL:?STAGING_FRONTEND_URL is required}"

GHCR_REPO="${GHCR_REPO:?GHCR_REPO is required}"
GHCR_USER="${GHCR_USER:?GHCR_USER is required}"
GHCR_TOKEN="${GHCR_TOKEN:?GHCR_TOKEN is required}"

DOKPLOY_BASE_URL="${DOKPLOY_BASE_URL:?DOKPLOY_BASE_URL is required}"
DOKPLOY_API_TOKEN="${DOKPLOY_API_TOKEN:?DOKPLOY_API_TOKEN is required}"

DOKPLOY_PROJECT_ID="${DOKPLOY_PROJECT_ID:-}"
DOKPLOY_ENVIRONMENT_ID="${DOKPLOY_ENVIRONMENT_ID:-}"
DOKPLOY_BACKEND_APP_ID="${DOKPLOY_BACKEND_APP_ID:?DOKPLOY_BACKEND_APP_ID is required}"
DOKPLOY_FRONTEND_APP_ID="${DOKPLOY_FRONTEND_APP_ID:-}"

COMPOSE_FLAG="${STATE_DIR}/compose-started.flag"
```

Create directories and set permissions immediately:

```bash
mkdir -p "$STATE_DIR" "$LOG_DIR" "$CONSOLE_DIR" "$DB_DIR" \
         "$SCREENSHOT_SMOKE_DIR" "$SCREENSHOT_DEEP_DIR" \
         "$SCREENSHOT_RESPONSIVE_DIR" "$SCREENSHOT_REGRESSION_DIR" \
         "$SCREENSHOT_BLOCKERS_DIR" "$REGRESSION_DIR" "$CHECKLIST_DIR" \
         "$CI_DIR" "$TIMING_DIR" "$REPORT_SUBDIR"

chmod 700 "$STATE_DIR"
echo "$STAGING_RUN_ID" > "${STATE_DIR}/run-id.txt"
echo "$PROJECT_NAME" > "${STATE_DIR}/project-name.txt"
```

Detect frontend package manager:

```bash
PM="npm"
[ -f frontend/pnpm-lock.yaml ] && PM="pnpm"
[ -f frontend/yarn.lock ] && PM="yarn"
[ -f frontend/bun.lockb ] && PM="bun"

# UFOP: pnpm workspace detection
[ -f pnpm-workspace.yaml ] && PM="pnpm"
[ -f pnpm-lock.yaml ] && PM="pnpm"

echo "PM=$PM"
```

---

## 17. REQUIRED DOKPLOY MANIFEST HOOKS

Resolve all hooks before any irreversible step. Each hook is listed with what it must contain.

### Application Hooks

| #  | Hook                                   | What it must contain                                                                  |
| -- | -------------------------------------- | ------------------------------------------------------------------------------------- |
| 1  | `BACKEND_HEALTHCHECK_URL`              | Canonical backend readiness/health endpoint for staging                               |
| 2  | `FRONTEND_HEALTHCHECK_URL`             | Canonical frontend readiness endpoint (may be same as `/`)                            |
| 3  | `FRONTEND_API_BUILD_ARG`               | Exact Docker build arg for staging API base URL                                       |
| 4  | `FRONTEND_API_ENV_VAR`                 | Exact runtime env var if runtime injection is used                                    |
| 5  | `MIGRATION_COMMAND`                    | Exact migration command approved for Dokploy execution                                |
| 6  | `STAGING_TEST_USER_CREATE_SNIPPET`     | Exact test-user creation logic (no `username` assumption)                             |
| 7  | `STAGING_TEST_USER_PERMISSION_SNIPPET` | Exact role/org/tenant/feature-flag assignment                                         |
| 8  | `STAGING_TEST_USER_CLEANUP_SNIPPET`    | Exact cleanup logic (no `username` assumption)                                        |
| 9  | `STAGING_TEST_DATA_CLEANUP_SNIPPET`    | Exact cleanup for all records created by this run                                     |
| 10 | `MULTI_TENANCY_MODE`                   | `none`, `django-tenants`, `django-multitenant`, `django-organizations`, or equivalent |
| 11 | `TENANT_STAGING_DOMAIN_CHECK_SNIPPET`  | Read-only tenant/domain verification if multi-tenancy exists                          |
| 12 | `TEST_ACCOUNT_MODE`                    | Must be exactly `preprovisioned` or `per_run`                                         |

**UFOP-specific hook adaptations:**
- Hook 5 (`MIGRATION_COMMAND`): For UFOP Rust backend, SQLite migrations run automatically on startup via `all_migrations()`. Set to `echo "SQLite migrations auto-applied on startup"` or the actual Rust migration command if containerized.
- Hook 10 (`MULTI_TENANCY_MODE`): UFOP uses `none` (no multi-tenancy)
- Hooks 13-17: For UFOP, `BACKEND_SERVICE_NAME` may be a Rust service. `DB_SERVICE_NAME` and `REDIS_SERVICE_NAME` may be empty/N/A if using SQLite without Redis.

### Local Validation Hooks

| #  | Hook                    |
| -- | ----------------------- |
| 13 | `BACKEND_SERVICE_NAME`  |
| 14 | `FRONTEND_SERVICE_NAME` |
| 15 | `WORKER_SERVICE_NAME`   |
| 16 | `DB_SERVICE_NAME`       |
| 17 | `REDIS_SERVICE_NAME`    |
| 18 | `DOCKERFILE_BACKEND`    |
| 19 | `DOCKERFILE_FRONTEND`   |

### Dokploy Deployment Hooks

| #  | Hook                                  | What it must contain                                     |
| -- | ------------------------------------- | -------------------------------------------------------- |
| 20 | `DOKPLOY_VALIDATE_ACCESS_SNIPPET`     | Command proving Dokploy access for this project          |
| 21 | `DOKPLOY_GET_CURRENT_RELEASE_SNIPPET` | Logic to capture current deployed release before deploy  |
| 22 | `DOKPLOY_SET_BACKEND_IMAGE_SNIPPET`   | Logic to set/update backend image/tag in Dokploy         |
| 23 | `DOKPLOY_SET_FRONTEND_IMAGE_SNIPPET`  | Logic to set/update frontend image/tag (if separate)     |
| 24 | `DOKPLOY_TRIGGER_DEPLOY_SNIPPET`      | Logic to trigger deploy/redeploy/restart                 |
| 25 | `DOKPLOY_EXEC_COMMAND_SNIPPET`        | Logic for executing approved remote commands via Dokploy |
| 26 | `DOKPLOY_FETCH_DEPLOY_STATUS_SNIPPET` | Logic to fetch deployment status after redeploy          |
| 27 | `DOKPLOY_ROLLBACK_SNIPPET`            | Logic to restore prior known-good release via Dokploy    |

---

## 18. MANIFEST TEMPLATE (fill this for each project)

Copy this block, fill it in, and keep it alongside the runbook for each project.

```yaml
# Manifest for: [PROJECT NAME]
# Last updated: [DATE]
# Filled by: [NAME]

project_name: my-project
local_run_id: "<required — set before each run>"
checklist_file: "docs/e2e/staging-checklist.md"

test_account_mode: "preprovisioned" # or "per_run"

staging:
  frontend_url: "https://staging.example.com"
  backend_url: "https://staging-api.example.com"
  backend_healthcheck_url: "https://staging-api.example.com/health/"
  frontend_healthcheck_url: "https://staging.example.com/"

credentials:
  # Used only when test_account_mode=preprovisioned
  # Never hardcode values here. Reference env var names only.
  e2e_test_email_env: "E2E_TEST_EMAIL"
  e2e_test_password_env: "E2E_TEST_PASSWORD"

dokploy:
  base_url: "https://dokploy.example.com"
  project_id: "<optional>"
  environment_id: "<optional>"
  backend_app_id: "<required>"
  frontend_app_id: "<optional>"
  validate_access_snippet: "<exact command>"
  get_current_release_snippet: "<exact command>"
  set_backend_image_snippet: "<exact command>"
  set_frontend_image_snippet: "<exact command or leave blank>"
  trigger_deploy_snippet: "<exact command>"
  exec_command_snippet: "<exact command>"
  fetch_deploy_status_snippet: "<exact command>"
  rollback_snippet: "<exact command>"

images:
  ghcr_repo: "ghcr.io/org/repo"
  dockerfile_backend: "backend/Dockerfile"
  dockerfile_frontend: "frontend/Dockerfile"
  frontend_api_build_arg: "NEXT_PUBLIC_API_BASE_URL"
  frontend_api_env_var: "NEXT_PUBLIC_API_BASE_URL"

local_validation:
  backend_service_name: "backend"
  frontend_service_name: "frontend"
  worker_service_name: "worker"
  db_service_name: "db"
  redis_service_name: "redis"

app_hooks:
  migration_command: "python manage.py migrate --noinput"
  multi_tenancy_mode: "none"
  tenant_staging_domain_check_snippet: ""
  staging_test_user_create_snippet: "<exact>"
  staging_test_user_permission_snippet: "<exact>"
  staging_test_user_cleanup_snippet: "<exact>"
  staging_test_data_cleanup_snippet: "<exact>"
```

### UFOP Admin Console Manifest Example

```yaml
# Manifest for: UFOP Admin Console
# Last updated: 2026-04-12
# Filled by: operator

project_name: ufop-admin
local_run_id: "<set before each run>"
checklist_file: "docs/e2e/staging-checklist.md"

test_account_mode: "preprovisioned"

staging:
  frontend_url: "<UFOP admin console staging URL>"
  backend_url: "<UFOP backend API staging URL>"
  backend_healthcheck_url: "<backend>/health/"
  frontend_healthcheck_url: "<frontend>/"

dokploy:
  base_url: "<Dokploy dashboard URL>"
  backend_app_id: "<required>"
  frontend_app_id: "<optional — if admin console deployed separately>"

images:
  ghcr_repo: "ghcr.io/<org>/ufop"
  dockerfile_backend: "src-tauri/Dockerfile"  # Rust backend containerized
  dockerfile_frontend: "admin/Dockerfile"      # Next.js admin console
  frontend_api_build_arg: "NEXT_PUBLIC_API_BASE_URL"
  frontend_api_env_var: "NEXT_PUBLIC_API_BASE_URL"

local_validation:
  backend_service_name: "backend"   # Rust service
  frontend_service_name: "admin"    # Next.js admin console
  worker_service_name: ""           # UFOP has no Celery workers
  db_service_name: ""               # SQLite — no external DB service
  redis_service_name: ""            # No Redis in UFOP
```

---

# LAYER 3 — EXECUTION RUNBOOK

---

## 19. PHASE A — MANIFEST RESOLUTION + BASELINE DISCOVERY

### 19.1 Resolve all required manifest values

Before any Dokploy deployment or browser testing is considered valid, resolve all hooks from Section 17. Log any gaps per Section 15.

### 19.2 Create base tracking artifacts

```bash
cat > "${CHECKLIST_DIR}/tracking-table.md" << 'EOF'
| CL-ID | Pass | Section | Status | Severity | Browser Screenshot | Other Evidence | Defect ID | Load Time | Notes |
|-------|------|---------|--------|----------|--------------------|----------------|-----------|-----------|-------|
EOF

touch "${CHECKLIST_DIR}/execution-log.txt"
touch "${REPORT_SUBDIR}/blocker-registry.md"
touch "${REPORT_SUBDIR}/pattern-failure-registry.md"
touch "${REPORT_SUBDIR}/manifest-gaps.md"
```

For checklists >100 items, also create the batch segment tracker:

```bash
cat > "${CHECKLIST_DIR}/batch-segments.md" << 'EOF'
| Batch | CL-Range | Items | Status | Checkpoint Saved |
|-------|----------|-------|--------|-----------------|
EOF
```

### 19.3 Baseline reachability check

Diagnostic only — not proof of failure if this is a fresh deploy.

```bash
curl -sf "$STAGING_BACKEND_URL" >/dev/null \
  && echo "[note] backend baseline reachable" \
  || echo "[note] backend baseline unreachable"
curl -sf "$STAGING_FRONTEND_URL" >/dev/null \
  && echo "[note] frontend baseline reachable" \
  || echo "[note] frontend baseline unreachable"
```

### 19.4 Confirm local prerequisite

The candidate revision must have passed the local runbook or equivalent local gate before staging work begins.

---

## 20. PHASE B — TOOLING + ACCESS PRE-FLIGHT

### 20.1 Required tooling

```bash
for tool in git curl docker jq python node; do
  command -v "$tool" >/dev/null 2>&1 || { echo "MISSING: $tool"; exit 1; }
done

node --version | grep -qE "v(1[89]|[2-9][0-9])" || { echo "MISSING: Node 18+"; exit 1; }
```

**UFOP-specific tooling checks:**

```bash
# pnpm (required for UFOP workspace builds)
command -v pnpm >/dev/null 2>&1 || { echo "MISSING: pnpm"; exit 1; }

# Rust toolchain (required for UFOP Rust backend)
command -v cargo >/dev/null 2>&1 || echo "[warn] cargo missing — needed for Rust backend builds"
command -v rustc >/dev/null 2>&1 || echo "[warn] rustc missing — needed for Rust backend builds"
```

Install browser tooling if absent:

```bash
command -v agent-browser >/dev/null 2>&1 || npm install -g agent-browser
agent-browser install --with-deps
agent-browser --version
```

Warn only if optional helpers are missing:

```bash
command -v psql >/dev/null 2>&1 || echo "[warn] psql missing"
command -v pkill >/dev/null 2>&1 || echo "[warn] pkill missing"
```

### 20.2 GHCR access

Do not print the token.

```bash
echo "$GHCR_TOKEN" | docker login ghcr.io -u "$GHCR_USER" --password-stdin >/tmp/ghcr-login.txt 2>&1
grep -q "Login Succeeded" /tmp/ghcr-login.txt \
  || { echo "[FAIL] Cannot authenticate to ghcr.io"; cat /tmp/ghcr-login.txt; exit 1; }
```

### 20.3 Dokploy access

```bash
[ -n "${DOKPLOY_VALIDATE_ACCESS_SNIPPET:-}" ] \
  || { echo "[FAIL] DOKPLOY_VALIDATE_ACCESS_SNIPPET missing"; exit 1; }
eval "$DOKPLOY_VALIDATE_ACCESS_SNIPPET"
```

### 20.4 Test credentials check

When `TEST_ACCOUNT_MODE=preprovisioned`, credentials must already be loaded from environment variables.
When `TEST_ACCOUNT_MODE=per_run`, skip this check here and create the isolated account in Phase G.

```bash
if [ "$TEST_ACCOUNT_MODE" = "preprovisioned" ]; then
  [ -z "${E2E_TEST_EMAIL:-}" ] && { echo "[FAIL] E2E_TEST_EMAIL not set"; exit 1; }
  [ -z "${E2E_TEST_PASSWORD:-}" ] && { echo "[FAIL] E2E_TEST_PASSWORD not set"; exit 1; }
  echo "[ok] Pre-provisioned test credentials loaded from environment"
fi

if [ "$TEST_ACCOUNT_MODE" = "per_run" ]; then
  echo "[ok] Test account will be created during Phase G"
fi
```

---

## 21. PHASE C — LOCAL CANDIDATE GATES

All must pass before touching Dokploy.

### 21.1 env.required validation

```bash
[ -f env.required ] || { echo "[FAIL] env.required not found"; exit 1; }

MISSING_VARS=0
while IFS= read -r var || [ -n "$var" ]; do
  [[ "$var" =~ ^#.*$ || -z "$var" ]] && continue
  var="$(echo "$var" | xargs)"
  if [ -z "${!var:-}" ]; then
    echo "[MISSING] $var" | tee -a "${CI_DIR}/env-required-check.txt"
    MISSING_VARS=$((MISSING_VARS + 1))
  fi
done < env.required

[ "$MISSING_VARS" -eq 0 ] || { echo "[FAIL] Missing env.required variables"; exit 1; }
```

### 21.2 Docker compose build/up

Do not invent service names — use manifest hooks.

```bash
docker compose --env-file .env.local build \
  2>&1 | tee "${CI_DIR}/compose-build.log" || exit 1
docker compose --env-file .env.local up -d \
  2>&1 | tee "${CI_DIR}/compose-up.log" || exit 1
touch "$COMPOSE_FLAG"
```

### 21.3 Backend health gate

```bash
[ -n "${BACKEND_HEALTHCHECK_URL:-}" ] \
  || { echo "[FAIL] BACKEND_HEALTHCHECK_URL missing"; docker compose down; exit 1; }

TRIES=0; MAX_TRIES=45
until curl -sf "$BACKEND_HEALTHCHECK_URL" >/dev/null 2>&1; do
  TRIES=$((TRIES + 1))
  if [ "$TRIES" -ge "$MAX_TRIES" ]; then
    echo "[FAIL] Backend health gate failed"
    docker compose ps | tee "${CI_DIR}/compose-ps-on-failure.log"
    docker compose logs --tail=200 2>&1 | tee "${CI_DIR}/compose-logs-on-failure.log"
    docker compose down
    exit 1
  fi
  sleep 2
done

curl -sf "$BACKEND_HEALTHCHECK_URL" | tee "${CI_DIR}/health-check-response.json" >/dev/null
```

### 21.4 Django checks / migration drift

```bash
docker compose exec -T "${BACKEND_SERVICE_NAME}" python manage.py check --deploy \
  2>&1 | tee "${CI_DIR}/django-check-deploy.log" \
  || { docker compose down; exit 1; }

docker compose exec -T "${BACKEND_SERVICE_NAME}" python manage.py makemigrations --check --dry-run \
  2>&1 | tee "${CI_DIR}/migration-check.log" \
  || { docker compose down; exit 1; }
```

**UFOP Rust backend (if containerized):**

```bash
# If the backend is a Rust service, replace Django checks with:
# cargo check 2>&1 | tee "${CI_DIR}/cargo-check.log" || { docker compose down; exit 1; }
# cargo test --lib 2>&1 | tee "${CI_DIR}/cargo-test.log" || { docker compose down; exit 1; }
```

```bash
# UFOP pnpm workspace checks
if [ -f "pnpm-workspace.yaml" ]; then
  pnpm lint 2>&1 | tee "${CI_DIR}/pnpm-lint.log" \
    || { docker compose down; exit 1; }
  pnpm test 2>&1 | tee "${CI_DIR}/pnpm-test.log" \
    || { docker compose down; exit 1; }
fi
```

### 21.5 Worker/redis gate (if applicable)

```bash
[ -n "${WORKER_SERVICE_NAME:-}" ] && \
  docker compose ps "${WORKER_SERVICE_NAME}" | tee "${CI_DIR}/worker-status.log"
```

### 21.6 Smoke gate

```bash
if [ -f scripts/smoke_tests.sh ]; then
  bash scripts/smoke_tests.sh 2>&1 | tee "${CI_DIR}/smoke-tests.log" \
    || { docker compose down; exit 1; }
else
  echo "[note] smoke_tests.sh not found; fallback check" | tee "${CI_DIR}/smoke-tests.log"
  curl -sf "$BACKEND_HEALTHCHECK_URL" >/dev/null \
    || { docker compose down; exit 1; }
fi
```

### 21.7 Frontend API wiring gate

Verify the frontend is wired to the correct staging API via the exact `FRONTEND_API_BUILD_ARG` and/or `FRONTEND_API_ENV_VAR` from the manifest. Do not assume variable names.

### 21.8 CI gate summary

```bash
echo "ALL LOCAL CANDIDATE GATES PASSED — STAGING DEPLOY AUTHORIZED" \
  | tee "${CI_DIR}/summary.txt"
docker compose down 2>&1 | tee "${CI_DIR}/compose-down.log"
```

---

## 22. PHASE D — PRE-DEPLOY RELEASE SNAPSHOT

Capture the currently deployed Dokploy release **before** building or redeploying.

Required outputs:

* `${STATE_DIR}/deployment-metadata.json`
* `${LOG_DIR}/rollback-state-predeploy.log`

```bash
[ -n "${DOKPLOY_GET_CURRENT_RELEASE_SNIPPET:-}" ] \
  || { echo "[FAIL] DOKPLOY_GET_CURRENT_RELEASE_SNIPPET missing"; exit 1; }
eval "$DOKPLOY_GET_CURRENT_RELEASE_SNIPPET"
```

The captured data must be sufficient to rollback to the prior known-good release.

---

## 23. PHASE E — BUILD + PUSH TARGET IMAGES

Use only the Dockerfile paths from the manifest. Do not hardcode paths.

```bash
docker build -t "${GHCR_REPO}/backend:${DEPLOY_IMAGE_TAG}" \
  -f "${DOCKERFILE_BACKEND}" . \
  2>&1 | tee "${LOG_DIR}/docker-build-backend.log" || exit 1

docker build --build-arg "${FRONTEND_API_BUILD_ARG}=${STAGING_BACKEND_URL}" \
  -t "${GHCR_REPO}/frontend:${DEPLOY_IMAGE_TAG}" \
  -f "${DOCKERFILE_FRONTEND}" . \
  2>&1 | tee "${LOG_DIR}/docker-build-frontend.log" || exit 1
```

**UFOP build note:** For the admin console, the Docker build context must be the workspace root (not `admin/`) to include `@ufop/design-tokens` and `@ufop/ui-components` shared packages:

```bash
# UFOP admin console build — workspace root context
docker build --build-arg "${FRONTEND_API_BUILD_ARG}=${STAGING_BACKEND_URL}" \
  -t "${GHCR_REPO}/frontend:${DEPLOY_IMAGE_TAG}" \
  -f admin/Dockerfile . \
  2>&1 | tee "${LOG_DIR}/docker-build-frontend.log" || exit 1
```

Push both and record digests if available:

```bash
docker push "${GHCR_REPO}/backend:${DEPLOY_IMAGE_TAG}" \
  2>&1 | tee "${LOG_DIR}/docker-push-backend.log" || exit 1
docker push "${GHCR_REPO}/frontend:${DEPLOY_IMAGE_TAG}" \
  2>&1 | tee "${LOG_DIR}/docker-push-frontend.log" || exit 1
```

---

## 24. PHASE F — GUARDED STAGING DEPLOY THROUGH DOKPLOY

### 24.1 Set backend image

```bash
[ -n "${DOKPLOY_SET_BACKEND_IMAGE_SNIPPET:-}" ] \
  || { echo "[FAIL] DOKPLOY_SET_BACKEND_IMAGE_SNIPPET missing"; exit 1; }
eval "$DOKPLOY_SET_BACKEND_IMAGE_SNIPPET"
```

### 24.2 Set frontend image (if separate)

```bash
if [ -n "${DOKPLOY_FRONTEND_APP_ID:-}" ]; then
  [ -n "${DOKPLOY_SET_FRONTEND_IMAGE_SNIPPET:-}" ] \
    || { echo "[FAIL] DOKPLOY_SET_FRONTEND_IMAGE_SNIPPET missing"; exit 1; }
  eval "$DOKPLOY_SET_FRONTEND_IMAGE_SNIPPET"
fi
```

### 24.3 Trigger deploy

```bash
[ -n "${DOKPLOY_TRIGGER_DEPLOY_SNIPPET:-}" ] \
  || { echo "[FAIL] DOKPLOY_TRIGGER_DEPLOY_SNIPPET missing"; exit 1; }
eval "$DOKPLOY_TRIGGER_DEPLOY_SNIPPET"
```

### 24.4 Poll deploy status

```bash
[ -n "${DOKPLOY_FETCH_DEPLOY_STATUS_SNIPPET:-}" ] \
  || { echo "[FAIL] DOKPLOY_FETCH_DEPLOY_STATUS_SNIPPET missing"; exit 1; }
eval "$DOKPLOY_FETCH_DEPLOY_STATUS_SNIPPET"
```

Stop on timeout or Dokploy-declared failure.

---

## 25. PHASE G — READINESS + MIGRATIONS

### 25.1 Backend readiness gate

```bash
TRIES=0; MAX_TRIES=45
until curl -sf "$BACKEND_HEALTHCHECK_URL" >/dev/null 2>&1; do
  TRIES=$((TRIES + 1))
  [ "$TRIES" -lt "$MAX_TRIES" ] || { echo "[FAIL] backend readiness failed"; exit 1; }
  sleep 2
done
echo "[ok] backend ready"
```

### 25.2 Frontend readiness gate

```bash
TRIES=0; MAX_TRIES=45
until curl -sf "$STAGING_FRONTEND_URL" | grep -q "<html" 2>/dev/null; do
  TRIES=$((TRIES + 1))
  [ "$TRIES" -lt "$MAX_TRIES" ] || { echo "[FAIL] frontend readiness failed"; exit 1; }
  sleep 2
done
echo "[ok] frontend ready"
```

### 25.3 Run migrations through Dokploy

```bash
[ -n "${DOKPLOY_EXEC_COMMAND_SNIPPET:-}" ] \
  || { echo "[FAIL] DOKPLOY_EXEC_COMMAND_SNIPPET missing"; exit 1; }
[ -n "${MIGRATION_COMMAND:-}" ] \
  || { echo "[FAIL] MIGRATION_COMMAND missing"; exit 1; }
eval "$DOKPLOY_EXEC_COMMAND_SNIPPET"
```

**UFOP note:** SQLite migrations via rusqlite run automatically on Rust backend startup. The `MIGRATION_COMMAND` hook may simply confirm migration status from container logs rather than executing a separate command.

### 25.4 Post-deploy smoke checks

Confirm:

* backend health responds correctly
* frontend returns real HTML
* no obvious staging-wide outage

### 25.5 Create or load staging E2E test account

* If `TEST_ACCOUNT_MODE=preprovisioned`, use `E2E_TEST_EMAIL` and `E2E_TEST_PASSWORD` from environment variables.
* If `TEST_ACCOUNT_MODE=per_run`, use `STAGING_TEST_USER_CREATE_SNIPPET` and `STAGING_TEST_USER_PERMISSION_SNIPPET` to create the isolated account for this run.

Do not assume a `username` field exists.

For `per_run`, store temporary credentials only in `${STATE_DIR}` and apply `chmod 600` to the credential file.

---

## 26. PHASE H — STAGING-SPECIFIC DISCOVERY VALIDATION

Re-check the staging-specific risks local cannot prove.

Required focus areas:

* auth callbacks
* CSRF / CORS headers
* frontend API base URL correctness in the built image
* migration-applied feature correctness
* object storage / email / third-party staging integrations
* worker/background job visibility where observable
* tenant routing / hostname mapping
* Dokploy release behaving as intended after deployment

**UFOP-specific focus areas:**
* shadcn/ui CSS/Tailwind classes loading correctly in staging build
* `@ufop/design-tokens` and `@ufop/ui-components` packages bundled correctly
* Tauri IPC stubs/fallbacks working in admin console context (non-Tauri environment)
* Zustand persist middleware rehydrating correctly in staging context
* Next.js App Router rendering without hydration errors

### 26.1 Multi-tenancy verification

```bash
if [ "${MULTI_TENANCY_MODE:-none}" != "none" ]; then
  [ -n "${TENANT_STAGING_DOMAIN_CHECK_SNIPPET:-}" ] \
    || { echo "[FAIL] tenant/domain check hook missing"; exit 1; }
  eval "$TENANT_STAGING_DOMAIN_CHECK_SNIPPET"
fi
```

Failure here is usually **Critical**.

---

## 27. PHASE I — CHECKLIST LOADING

### 27.1 Presence check

```bash
[ -f "$CHECKLIST_FILE" ] \
  || { echo "[FAIL] Checklist file missing: $CHECKLIST_FILE"; exit 1; }
```

### 27.2 Expected checklist schema

The checklist should contain:

* stable `CL-XXX` identifiers
* explicit pass tags: `[SMOKE]` or `[DEEP]`
* explicit evidence expectations where needed

Do not mix incompatible checklist formats.

---

## 28. PHASE J — CHECKLIST EXECUTION ENGINE

### 28.1 Execution order

Execute in document order:

1. all `[SMOKE]` items
2. all `[DEEP]` items

No jumping, no inference, and no batch-level pass/fail shortcuts.
If batch segmentation is active under Section 13, each item within the batch must still be tested and classified individually in document order.

### 28.2 Narrated action protocol

Before each major browser action, write one concise sentence:

* `Opening staging login page`
* `Typing staging admin email`
* `Submitting login form`
* `Opening dashboard from sidebar`
* `Testing required-field validation on the create form`

After each major screenshot, write **1–3 concise sentences** describing what it shows.

### 28.3 Pass 1 — SMOKE

For each `[SMOKE]` item:

1. narrate intended action
2. navigate visibly in browser
3. capture page-load timing
4. take screenshot
5. describe screenshot
6. perform the interaction visibly
7. take result screenshot
8. capture relevant console/errors
9. perform VBEP self-check (Section 10)
10. classify and update the tracking table

No fixing during smoke.

### 28.4 Pass 2 — DEEP

For each `[DEEP]` item:

1. narrate intended action
2. pre-check obvious dependencies if relevant
3. navigate visibly in browser
4. capture timing
5. inspect interactive elements
6. perform UI interactions visibly
7. capture screenshots after significant interactions
8. capture console/errors/network evidence
9. use DB verification only if explicitly required and only as supplementary evidence
10. perform VBEP self-check (Section 10)
11. assign severity if not passed
12. update tracking table and defect register

### 28.5 Time-boxing

* standard item: 3 minutes max
* complex item: 5 minutes max

On expiry: capture current state, classify honestly, move on.

### 28.6 Browser-only evidence rule

Every checklist row must contain at least one browser screenshot path.

### 28.7 Blocker protocol

1. record the blocker within 60 seconds
2. capture screenshot and console/network evidence
3. add blocker entry to blocker-registry.md
4. continue attempting dependent items
5. classify them honestly based on evidence

### 28.8 Pattern failure escalation

1. pause brief item-by-item repetition
2. investigate common failure for up to 3 minutes
3. write a pattern-failure entry in pattern-failure-registry.md
4. classify affected items consistently
5. continue with unaffected scope

### 28.9 Checkpoint and batch protocol

After every 10 items: save tracking-table state, execution log, registries, resume pointer.

After every 25 items: additionally save `run-state.json`, update batch-segments.md, and emit the Batch Handoff message (Section 13).

---

## 29. PHASE K — RESPONSIVE, ACCESSIBILITY, AND REGRESSION VALIDATION

### 29.1 Responsive smoke

Test key pages at: 375x812, 812x375, 768x1024, 1280x720, 1440x900. Capture screenshots.

**UFOP note:** The admin console uses Tailwind CSS responsive classes and shadcn/ui components. Verify that responsive breakpoints work correctly and that TanStack Table/Virtual components adapt to viewport size.

### 29.2 Accessibility smoke

Check at minimum:

* keyboard navigation
* visible focus indicators
* form labels
* obvious contrast problems
* visible error states/messages

Capture screenshots and findings.

**UFOP note:** React Aria components (used in the UFOP admin console) should provide baseline accessibility. Verify ARIA attributes are present and focus management works correctly.

### 29.3 Regression rule

A regression is: **passed locally, failed in staging**.

Attach local/staging evidence references side-by-side when possible.

---

## 30. PHASE L — CLEANUP

Cleanup may target only isolated test data created by this run.

### 30.1 Remove staging E2E user

```bash
if [ "$TEST_ACCOUNT_MODE" = "per_run" ]; then
  eval "$STAGING_TEST_USER_CLEANUP_SNIPPET"
fi
```

Do not assume a `username` field exists.

### 30.2 Remove other isolated test data

```bash
eval "$STAGING_TEST_DATA_CLEANUP_SNIPPET"
```

### 30.3 Remove transient credential files

```bash
find "${STATE_DIR}" -name "*.creds" -type f -delete 2>/dev/null || true
```

### 30.4 Preserve artifacts

Never delete: screenshots, logs, tracking tables, blocker/pattern registries, console/network evidence, final report.

---

## 31. PHASE M — REPORTING

Always export the final report automatically. Never prompt for it.

Canonical report path:

```text
{project_name}/docs/playwright-devtools/{STAGING_RUN_ID}/report/e2e-staging-report.md
```

### 31.1 Required report sections

* run metadata (RUN_ID, commit, project, timestamp)
* local prerequisite reference
* manifest summary (resolved vs gaps)
* Dokploy deploy metadata (image tags, digests)
* pre-deploy rollback state reference
* local gate summary
* readiness summary
* checklist summary (by section and severity)
* severity summary table
* blocker registry summary
* pattern-failure summary
* responsive findings
* accessibility findings
* regression findings
* cleanup summary
* visual-browser compliance audit (see Section 31.3)
* final verdict with explicit rollback recommendation if applicable
* evidence inventory (all screenshot paths)
* rollback event record (if rollback occurred — use template in Section 32.2)
* test account model used (`preprovisioned` or `per_run`) and its outcome

### 31.2 Machine-readable state

```bash
cat > "${REPORT_SUBDIR}/run-state.json" << EOF
{
  "staging_run_id": "${STAGING_RUN_ID}",
  "project_name": "${PROJECT_NAME}",
  "deploy_commit": "${DEPLOY_COMMIT}",
  "final_verdict": "PENDING",
  "critical": 0,
  "high": 0,
  "medium": 0,
  "low": 0,
  "regression": 0,
  "total_items": 0,
  "passed": 0,
  "failed": 0,
  "partial": 0,
  "blocked": 0,
  "rollback_performed": false,
  "last_completed_cl": null,
  "next_cl_to_execute": null,
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
}
EOF
```

### 31.3 Visual-browser compliance audit

The report must explicitly answer:

* Was the browser headed and visible for the entire run?
* Was human-followable pacing maintained?
* Were first visits navigated through the visible UI where applicable?
* Were any items passed primarily by backend/API evidence?
* Were any hidden shortcuts used?
* Were any items missing screenshots or screenshot descriptions?

### 31.4 Final verdict rules

| Condition                            | Verdict                                   |
| ------------------------------------ | ----------------------------------------- |
| Any Critical                         | Rollback recommended                      |
| 2+ High                              | Rollback recommended                      |
| 1 High + multiple Medium             | Rollback strongly recommended             |
| Only Medium/Low                      | Conditional pass — log for fix cycle      |
| No Critical/High, core journeys pass | Approved — staging production candidate   |

---

## 32. PHASE N — ROLLBACK PROCEDURE

If the verdict requires rollback:

1. record rollback decision
2. restore prior known-good release using the exact Dokploy rollback hook
3. trigger Dokploy redeploy/restart
4. wait for backend and frontend readiness
5. verify health endpoints
6. run post-rollback smoke checks
7. complete rollback report entry (Section 32.2)

```bash
[ -n "${DOKPLOY_ROLLBACK_SNIPPET:-}" ] \
  || { echo "[FAIL] DOKPLOY_ROLLBACK_SNIPPET missing"; exit 1; }
eval "$DOKPLOY_ROLLBACK_SNIPPET"
```

### 32.1 Migration warning

Rolling back images does **not** automatically reverse migrations.
This must always be documented explicitly in the report.

**UFOP note:** SQLite migrations via rusqlite in `all_migrations()` are forward-only. Rolling back the Rust backend image may cause schema version mismatches if new migrations were applied. Document this risk explicitly.

### 32.2 Rollback report entry template

This entry is required for every rollback event. No field may be omitted.

```markdown
## Rollback Event — [STAGING_RUN_ID]

| Field | Value |
|-------|-------|
| Triggered at | [timestamp] |
| Triggering condition | [e.g., "2 Critical issues: CL-014, CL-031"] |
| Pre-deploy backend image/digest | [from deployment-metadata.json] |
| Pre-deploy frontend image/digest | [from deployment-metadata.json or N/A] |
| Rolled back to (backend) | [image/digest] |
| Rolled back to (frontend) | [image/digest or N/A] |
| Dokploy restart triggered at | [timestamp] |
| Backend health post-rollback | PASS / FAIL |
| Frontend health post-rollback | PASS / FAIL |
| Post-rollback smoke result | PASS / FAIL — [details] |
| Migration rollback required | Yes / No |
| Migration rollback action taken | [describe or N/A] |
| Rollback complete at | [timestamp] |
| Notes | [any additional context] |
```

---

## 33. PHASE O — CHECKPOINT / RESUME PROTOCOL

If execution must stop before completion:

1. save tracking table to `${CHECKLIST_DIR}/tracking-table-checkpoint-[N].md`
2. update `run-state.json` with `last_completed_cl` and `next_cl_to_execute`
3. save blocker and pattern-failure registries
4. emit a partial report with the exact resume point
5. resume from the first unfinished checklist item in the next session

---

## 34. DEFINITIVE GUARANTEES OF V6.3

When followed correctly, this runbook guarantees:

1. staging behavior is validated in a visible browser, not inferred from local
2. local sanity gates run before staging is touched
3. rollback state is captured before Dokploy deploy
4. Dokploy actions are isolated behind explicit manifest hooks — never guessed
5. project-specific values are isolated in the manifest — policy never changes between projects
6. first-time page visits favor visible UI navigation
7. every checklist item receives an explicit status
8. blocker handling does not permit silent skipping
9. pattern failures are escalated systematically
10. report output is canonical and consistent
11. secret handling rules reduce accidental leakage — includes safe chmod handling for credential files
12. test-user creation/cleanup do not assume a `username` field
13. multi-tenant staging checks are explicit
14. final report includes a visual-browser compliance audit
15. rollback guidance is built in and evidence-based with a required structured entry
16. test credentials are never hardcoded; the manifest explicitly selects either `preprovisioned` or `per_run`
17. token budget management is explicit — checklists >100 items batch in groups of 25 without batch-level shortcutting
18. manifest derivation has a 10-minute timeout before surfacing gaps to the operator

---

## 35. FINAL EXECUTION INSTRUCTION

After receiving this runbook:

1. fill in Layer 2 for this project
2. resolve all manifest hooks
3. validate local candidate gates
4. capture Dokploy rollback state
5. build and deploy through the approved Dokploy path
6. verify readiness
7. execute visible-browser E2E against the checklist
8. classify defects honestly
9. clean up isolated test data
10. export the canonical report
11. recommend rollback if required by the evidence

**This runbook is for STAGING only.**
**Deployment platform: DOKPLOY only.**
**To reuse for a new project: fill in Layer 2. Do not modify Layers 1 or 3.**
**Execute completely.**

*Customized for UFOP: Tauri 2.0 + Rust + React/Vite + Next.js Admin + SQLite/rusqlite + pnpm workspaces + Zustand + shadcn/ui*
