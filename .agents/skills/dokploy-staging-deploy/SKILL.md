---
name: dokploy-staging-deploy
description: Use when deploying to staging via Dokploy, when asked to "deploy to staging", "stage this release", "push to staging", "dokploy deploy", or any staging deployment request. Handles full autonomous pipeline — pre-deploy discovery, safety gates, ordered deployment, verification, rollback logic, and final reporting. Platform is Dokploy (NOT Coolify). Tech stack context — UFOP project with Next.js admin console, Rust/Tauri backend services, SQLite (rusqlite), pnpm workspaces, Zustand, shadcn/ui, Tailwind CSS, React Aria, TanStack; runbook also supports Django, Celery, PostgreSQL, Redis stacks for multi-project Dokploy hosts.
---

# DOKPLOY STAGING DEPLOYMENT + VERIFICATION AGENT

## Claude Code Instruction Runbook — v3.0

### Stacks: Django · Next.js · PostgreSQL · Redis · Celery · Docker · GHCR

**UFOP Project Stack Context:**
- **Desktop App:** Tauri 2.0 shell + Rust core engine (tokio async) + React/TypeScript/Vite frontend
- **Admin Console:** Next.js + TypeScript (port 3001 dev / production port varies)
- **State Management:** Zustand (with `persist()` middleware)
- **UI:** shadcn/ui + React Aria + Tailwind CSS + lucide-react icons
- **Data:** SQLite via rusqlite (`Arc<Mutex<Connection>>` pool, WAL mode)
- **Shared Packages:** `@ufop/design-tokens`, `@ufop/ui-components`
- **Package Manager:** pnpm 10.27.0 workspaces
- **IPC Boundary:** Tauri IPC commands (`#[tauri::command]` in Rust, `tauriInvoke<T>()` on frontend)
- **Deployment Platform:** Dokploy (exclusive — never Coolify)

> **Note:** This runbook supports the full Django + Next.js + PostgreSQL + Redis + Celery stack for multi-project Dokploy hosts. For UFOP-specific deployments (admin console, Rust backend services), adapt the `HAS_*` service flags in Section 0 accordingly. Services not present in UFOP (e.g., Django, Celery) should be set to `no` — the runbook gracefully skips them while still verifying shared infrastructure.

---

## SECTION 0 — MISSION VARIABLES

**Fill these before giving this runbook to Claude Code.**
Every `<required>` MUST be resolved before any write action is taken.
If a value is unknown, the agent derives it from the environment — never guesses.

```bash
# ── ENVIRONMENT ────────────────────────────────────────────────────────────
TARGET_ENVIRONMENT=staging
RUN_ID="$(date +%Y%m%d-%H%M%S)"

# ── DOKPLOY SERVER ──────────────────────────────────────────────────────────
DOKPLOY_HOST="<required>"           # e.g. staging.myserver.com or 192.168.1.100
DOKPLOY_PORT="3000"                 # Dokploy panel default — change if different
DOKPLOY_API_TOKEN="<required>"      # Dokploy API token for programmatic access

# ── TARGET APPLICATION ──────────────────────────────────────────────────────
DOKPLOY_PROJECT_NAME="<required>"   # Exact project name inside Dokploy
TARGET_APP_NAME="<required>"        # Exact application name inside Dokploy
TARGET_DOMAINS="<required>"         # e.g. staging.myapp.com (comma-separated if multiple)

# ── SERVICES IN THIS APPLICATION (set each to yes/no) ──────────────────────
HAS_DJANGO_API=yes                  # yes | no
HAS_NEXTJS_FRONTEND=yes             # yes | no — UFOP admin console uses Next.js
HAS_CELERY_WORKER=yes               # yes | no
HAS_CELERY_BEAT=yes                 # yes | no — scheduler/periodic tasks
HAS_POSTGRESQL=yes                  # yes | no
HAS_REDIS=yes                       # yes | no
HAS_NGINX_PROXY=no                  # yes | no — internal nginx sidecar (not Dokploy proxy)
HAS_RUST_BACKEND=no                 # yes | no — UFOP Rust/Tauri backend service (if containerized)
HAS_SQLITE=no                       # yes | no — UFOP uses SQLite via rusqlite (WAL mode)

# ── RELEASE IDENTITY ────────────────────────────────────────────────────────
TARGET_IMAGE_TAG="<required>"       # MUST be a pinned tag — NEVER "latest"
TARGET_GIT_COMMIT="<required>"      # full git SHA for audit trail

# ── REGISTRY ────────────────────────────────────────────────────────────────
IMAGE_REGISTRY="ghcr.io"           # ghcr.io | docker.io | other
GHCR_ORG="<required>"              # GitHub org or username
DJANGO_IMAGE="${IMAGE_REGISTRY}/${GHCR_ORG}/backend:${TARGET_IMAGE_TAG}"
NEXTJS_IMAGE="${IMAGE_REGISTRY}/${GHCR_ORG}/frontend:${TARGET_IMAGE_TAG}"

# ── DATABASE ─────────────────────────────────────────────────────────────────
DB_SERVICE_NAME="<required>"        # Dokploy service name for PostgreSQL
DB_NAME="<required>"
DB_USER="<required>"
# Never store DB_PASSWORD here — verify its presence in Dokploy env, not value

# ── REDIS ────────────────────────────────────────────────────────────────────
REDIS_SERVICE_NAME="<required>"     # Dokploy service name for Redis

# ── MIGRATION SAFETY ─────────────────────────────────────────────────────────
RUN_MIGRATIONS=yes                  # yes | no | review-first
MIGRATION_RISK="<required>"         # none | safe | caution | high-risk (set after review)

# ── ROLLBACK ─────────────────────────────────────────────────────────────────
ROLLBACK_IMAGE_TAG="<required>"     # Last known-good tag — discover from Dokploy history
ROLLBACK_TRIGGER=manual             # manual | auto
ROLLBACK_AUTO_FAIL_THRESHOLD=3      # Health check failures before auto-rollback triggers
```

### UFOP-Specific Variable Examples

For deploying the UFOP admin console only (no Django/Celery):

```bash
HAS_DJANGO_API=no
HAS_NEXTJS_FRONTEND=yes             # UFOP admin console (Next.js)
HAS_CELERY_WORKER=no
HAS_CELERY_BEAT=no
HAS_POSTGRESQL=no                   # Still verify health if shared host has it
HAS_REDIS=no
HAS_RUST_BACKEND=no                 # Set yes if Rust backend is containerized separately
HAS_SQLITE=no                       # Set yes if SQLite DB file is inside a container volume
HAS_NGINX_PROXY=no
RUN_MIGRATIONS=no
```

---

## SECTION 1 — BINDING EXECUTION DIRECTIVES

You are an autonomous **Dokploy staging deployment and verification agent**.

**Start executing immediately.** Do not ask for permission, do not offer options, do not reduce scope.

These rules are non-negotiable:

### Platform rule
Use **Dokploy and only Dokploy** as the deployment platform for all write actions.
- Never issue Coolify commands
- Never use docker CLI for deploy actions (read-only inspection is permitted)
- Never bypass Dokploy state with direct container recreation unless recovery requires it and you document why Dokploy-native action was impossible
- All application state is managed through Dokploy — treat it as the source of truth

### Stack rule
The target stacks are **Django (Python), Next.js (TypeScript), PostgreSQL, Redis, Celery** — and for UFOP projects additionally **Rust/Tauri, SQLite (rusqlite), Zustand, pnpm workspaces**.
Every diagnostic, health check, migration step, and log check must be written for these specific stacks.
Do not run Rails, Spring, or other framework commands.

### Non-stop rule
A blocker on one service or component **never stops** execution of independent checks.
Classify every blocker, document it, isolate its blast radius, and continue all safe remaining work.

### No-assumption rule
Never assume:
- image tags or digests
- service names or container IDs
- health endpoint paths
- env var values
- migration state
- rollback target
- which services are actually running

Discover every value from the environment before acting on it.

### Verification rule
Deployment logs showing "success" are not sufficient.
Every service requires runtime evidence before it may be marked verified.

### No-production-contamination rule
Staging must never contain:
- production database credentials
- production domain names in CORS/CSRF/ALLOWED_HOSTS
- production payment keys, email provider keys, OAuth secrets
- production S3/storage bucket names

If any are found: stop write actions, register as Tier-3 blocker, escalate.

---

## SECTION 2 — PRE-DEPLOY DISCOVERY

> Do NOT execute any write action before completing this section.

### 2.1 Dokploy topology discovery

Connect to Dokploy and confirm the following. Record every value found.

**Dokploy platform checks:**
```
- Dokploy panel accessible at DOKPLOY_HOST:DOKPLOY_PORT? → yes / no
- Dokploy API responding at /api/health? → yes / no
- API token valid? → yes / no
- Dokploy version: ___________
```

**Project and application:**
```
- Project DOKPLOY_PROJECT_NAME exists in Dokploy? → yes / no
- Application TARGET_APP_NAME exists within that project? → yes / no
- Application type in Dokploy: docker-compose | dockerfile | image | nixpacks | buildpacks
- Current deployment method: ___________
- Last deployed at: ___________
- Last deployed by: ___________
```

**Service inventory (for this application only):**

| Service | Dokploy Service Name | Container ID | Status | Current Image Tag | Restart Count |
|---------|----------------------|--------------|--------|-------------------|---------------|
| Django API | | | | | |
| Next.js frontend | | | | | |
| Celery worker | | | | | |
| Celery Beat | | | | | |
| PostgreSQL | | | | | |
| Redis | | | | | |
| Rust backend (UFOP) | | | | | |
| Any others found | | | | | |

**Dokploy reverse proxy:**
```
- Traefik proxy running on host? → yes / no
- Domain TARGET_DOMAINS configured in Dokploy routing? → yes / no
- TLS certificate status for TARGET_DOMAINS: valid / expired / missing / expiry date: ___
- Proxy routing TARGET_DOMAINS → correct application? → yes / no
```

### 2.2 Stack-specific file discovery

Scan the project codebase to confirm the following:

**Django / Python (skip if HAS_DJANGO_API=no):**
```
- manage.py present? → yes / no (location: ___)
- requirements.txt or pyproject.toml present? → yes / no
- Django version: ___________
- DRF (djangorestframework) in requirements? → yes / no
- Celery in requirements? → yes / no
- django-celery-beat in requirements? → yes / no
- Health check endpoint defined? Search for /health, /api/health/, /healthz → found at: ___
- DJANGO_SETTINGS_MODULE for staging env: ___________
- ALLOWED_HOSTS for staging: ___________
- CORS_ALLOWED_ORIGINS for staging: ___________
```

**Next.js / TypeScript:**
```
- package.json present? → yes / no
- Next.js version: ___________
- next.config.js / next.config.ts present? → yes / no
- NEXT_PUBLIC_API_URL set in staging env? → yes / no (key presence only)
- /api/health route defined? Search app/api/health → found at: ___
- Is this a standalone output build (output: 'standalone' in next.config)? → yes / no
```

**UFOP-Specific (if HAS_RUST_BACKEND=yes or HAS_NEXTJS_FRONTEND=yes for admin console):**
```
- pnpm-workspace.yaml present? → yes / no
- admin/package.json present? → yes / no (UFOP admin console)
- Admin console Next.js version: ___________
- packages/design-tokens present? → yes / no (@ufop/design-tokens)
- packages/ui-components present? → yes / no (@ufop/ui-components)
- Rust backend Cargo.toml present (src-tauri/)? → yes / no
- Tauri version: ___________
- SQLite database path configured? → ___________
```

**PostgreSQL (skip if HAS_POSTGRESQL=no):**
```
- DATABASE_URL or individual DB_* vars expected? ___________
- Django DATABASES setting points to: ___________
- Number of migration files: ___________
- Pending migrations (if DB accessible): ___________
```

**Redis (skip if HAS_REDIS=no):**
```
- REDIS_URL or CELERY_BROKER_URL expected in env? → yes / no
- CACHE_BACKEND uses Redis? → yes / no
- Celery BROKER configured to Redis? → yes / no
- Celery RESULT_BACKEND configured to Redis? → yes / no
```

### 2.3 Current deployment state snapshot

Before any write action, capture and record:

```
SNAPSHOT TIME: ___________

Django API (skip if HAS_DJANGO_API=no):
  - Container status: running | stopped | restarting | absent
  - Image tag currently running: ___________
  - Uptime: ___________
  - Last restart: ___________
  - /api/health/ response: ___________

Next.js:
  - Container status: ___________
  - Image tag currently running: ___________
  - Uptime: ___________
  - Root route HTTP response: ___________

Rust Backend (UFOP — skip if HAS_RUST_BACKEND=no):
  - Container status: ___________
  - Image tag currently running: ___________
  - Uptime: ___________
  - Health endpoint response: ___________

Celery Worker (skip if HAS_CELERY_WORKER=no):
  - Container status: ___________
  - Image tag currently running: ___________
  - Last heartbeat (from logs): ___________
  - Active task queue: ___________

Celery Beat (skip if HAS_CELERY_BEAT=no):
  - Container status: ___________
  - Image tag currently running: ___________
  - Last scheduled task run: ___________

PostgreSQL (skip if HAS_POSTGRESQL=no):
  - Container status: ___________
  - Version: ___________
  - Accepting connections: yes / no
  - Active connection count: ___________
  - Migration state (current head): ___________

Redis (skip if HAS_REDIS=no):
  - Container status: ___________
  - Version: ___________
  - PING response: PONG / no response
  - Connected clients: ___________
  - Memory used: ___________

Pre-existing issues (found BEFORE this deployment — do NOT attribute to new release):
  - ___________
```

This snapshot is the rollback baseline. Record `ROLLBACK_IMAGE_TAG` here if not already set.

### 2.4 Environment variable audit

**RULE: Never log or display secret values. Verify key presence only.**

Scan the Django and Next.js source for all `os.environ`, `os.getenv`, `process.env`, `env()` references.
Cross-reference against Dokploy env vars configured for this application.

**Required Django env keys (skip if HAS_DJANGO_API=no) — mark each PRESENT or MISSING:**
```
SECRET_KEY                    → ___
DEBUG                         → ___  (must be False for staging)
DJANGO_SETTINGS_MODULE        → ___
DATABASE_URL or DB_HOST/PORT/NAME/USER/PASSWORD  → ___
REDIS_URL or CELERY_BROKER_URL  → ___
CELERY_RESULT_BACKEND         → ___
ALLOWED_HOSTS                 → ___ (must include staging domain, not production)
CORS_ALLOWED_ORIGINS          → ___ (must include staging frontend URL)
CSRF_TRUSTED_ORIGINS          → ___ (must include staging domain)
Any EMAIL_* vars if used      → ___
Any OAUTH/SOCIAL_AUTH vars    → ___
Any storage vars (AWS_S3 etc) → ___
Any payment vars              → ___
```

**Required Next.js env keys — mark each PRESENT or MISSING:**
```
NEXT_PUBLIC_API_URL           → ___ (must point to staging backend, not production)
NEXTAUTH_URL                  → ___ (must be staging domain)
NEXTAUTH_SECRET               → ___
Any NEXT_PUBLIC_* vars        → ___
Any server-side API keys      → ___
```

**UFOP Admin Console env keys (if deploying UFOP admin — check admin/.env or Dokploy env):**
```
NEXT_PUBLIC_API_URL           → ___ (must point to staging, not production)
Any @ufop/* package config    → ___
```

**Contamination check — verify NONE of these production patterns exist in staging env:**
```
- API URLs containing production domain → none found / FOUND (blocker)
- Payment keys containing live/prod prefix → none found / FOUND (blocker)
- S3 bucket names containing production name → none found / FOUND (blocker)
- OAuth redirect URIs pointing to production domain → none found / FOUND (blocker)
```

### 2.5 Migration risk classification

Compare migration files between `ROLLBACK_IMAGE_TAG` and `TARGET_IMAGE_TAG`:

```bash
# Run this to identify new migrations in this release:
git diff ${ROLLBACK_IMAGE_TAG}..${TARGET_GIT_COMMIT} -- '*/migrations/*.py' --name-only
```

For each new migration file found, classify:

| Migration file | Type | Risk level | Reversible? |
|----------------|------|------------|-------------|
| | AddField/CreateModel | SAFE | yes |
| | AlterField | CAUTION | maybe |
| | RemoveField/DeleteModel | HIGH-RISK | no |
| | Data migration | CAUTION/HIGH | depends |
| | Custom RunSQL/RunPython | CAUTION/HIGH | depends |

**UFOP SQLite migrations (if HAS_SQLITE=yes):**

Check `src-tauri/src/storage/migrations.rs` for changes to `all_migrations()`:

```bash
git diff ${ROLLBACK_IMAGE_TAG}..${TARGET_GIT_COMMIT} -- 'src-tauri/src/storage/migrations.rs' --name-only
```

| Migration version | Type | Risk level | Reversible? |
|-------------------|------|------------|-------------|
| | CREATE TABLE | SAFE | yes |
| | ALTER TABLE ADD COLUMN | SAFE | yes |
| | ALTER TABLE DROP COLUMN | HIGH-RISK | no |
| | Data transform | CAUTION/HIGH | depends |

**Final migration risk classification:**
- `NONE` → no new migration files
- `SAFE` → additive only (new tables, new nullable/defaulted columns)
- `CAUTION` → field type changes, index changes, data transforms
- `HIGH-RISK` → column/table removal, irreversible data changes → **STOP — human confirmation required before proceeding**

Set `MIGRATION_RISK` in the mission variables based on this analysis.

### 2.6 GHCR image verification

Confirm the target images exist and are pullable before deploying:

```bash
# Verify Django image exists (skip if HAS_DJANGO_API=no)
docker manifest inspect ${DJANGO_IMAGE}
# Expected: image manifest JSON — not an error

# Verify Next.js image exists
docker manifest inspect ${NEXTJS_IMAGE}
# Expected: image manifest JSON — not an error

# Verify Dokploy has GHCR credentials configured
# Check: Dokploy → Settings → Registries → ghcr.io entry exists with valid token
```

Record:
```
Django image ${TARGET_IMAGE_TAG} exists in GHCR: yes / no / N/A
Next.js image ${TARGET_IMAGE_TAG} exists in GHCR: yes / no
Dokploy GHCR credentials configured: yes / no
Image digest (Django):  ___________
Image digest (Next.js): ___________
```

---

## SECTION 3 — PRE-DEPLOY SAFETY GATES

All six gates must be evaluated in order. A FAIL on any gate is registered in the Blocker Registry — it does NOT silently prevent the remaining gates from being checked.

**Gate results format:** `PASS | FAIL | BLOCKED` — one line per item.

### Gate 1 — Release identity
```
[ ] TARGET_IMAGE_TAG is a pinned semantic tag or SHA — not "latest", not "main"
[ ] TARGET_GIT_COMMIT is a full 40-character SHA
[ ] Both DJANGO_IMAGE and NEXTJS_IMAGE exist in GHCR at TARGET_IMAGE_TAG (skip N/A services)
[ ] Target environment is confirmed as staging — not production
[ ] No production domains found in staging env (from Section 2.4 contamination check)
```

### Gate 2 — Dokploy access and application state
```
[ ] Dokploy API is accessible and API token is valid
[ ] DOKPLOY_PROJECT_NAME exists in Dokploy
[ ] TARGET_APP_NAME exists within that project
[ ] Dokploy service definitions match expected services (HAS_* variables)
[ ] Dokploy reverse proxy is running and TARGET_DOMAINS are routed
```

### Gate 3 — Environment completeness
```
[ ] All required Django env keys PRESENT in Dokploy env (Section 2.4) — or N/A if no Django
[ ] All required Next.js env keys PRESENT in Dokploy env (Section 2.4)
[ ] DEBUG=False confirmed for staging (Django) or NODE_ENV=production (Next.js)
[ ] ALLOWED_HOSTS does not contain production domains
[ ] NEXT_PUBLIC_API_URL points to staging backend, not production
[ ] No MISSING keys found in Section 2.4 audit
```

### Gate 4 — Dependency health
```
[ ] PostgreSQL container running and accepting connections (if HAS_POSTGRESQL=yes)
[ ] Redis container running and responding to PING (if HAS_REDIS=yes)
[ ] Celery broker URL reachable (Redis ping via broker address) (if HAS_CELERY_WORKER=yes)
[ ] Any external third-party APIs referenced in env are reachable (HEAD/ping only)
[ ] Storage service reachable if S3/compatible bucket is configured
[ ] SQLite database file accessible and not locked (if HAS_SQLITE=yes)
```

### Gate 5 — Migration safety
```
[ ] MIGRATION_RISK classified as none | safe | caution | high-risk
[ ] If HIGH-RISK: human confirmation obtained before proceeding — deployment PAUSED
[ ] If CAUTION: risk documented, mitigation plan stated, proceeding with caution
[ ] If NONE or SAFE: proceed
[ ] Rollback migration script exists if RUN_MIGRATIONS=yes
```

### Gate 6 — Rollback readiness
```
[ ] ROLLBACK_IMAGE_TAG confirmed and exists in GHCR
[ ] Current running image tags recorded (snapshot from Section 2.3)
[ ] Rollback steps documented for each service
[ ] If migrations ran: down-migration exists OR data loss risk on rollback is documented
[ ] All services that must roll back together are identified
```

---

## SECTION 4 — DEPLOYMENT EXECUTION

> Only proceed after all 6 gates are evaluated and no unresolved Tier-3 blockers exist.

### 4.1 Deployment order

Services must be deployed in this order to respect dependencies:

```
1. PostgreSQL       — no action needed (managed service, do not redeploy unless image changed)
2. Redis            — no action needed (managed service, do not redeploy unless image changed)
3. Django API       — deploy new image, then run migrations before workers start
   (OR Rust Backend — if UFOP: deploy Rust service, verify SQLite migrations)
4. Celery Worker    — deploy after Django migrations complete
5. Celery Beat      — deploy after Celery Worker is verified healthy
6. Next.js frontend — deploy after backend API health is confirmed
```

**Do not redeploy PostgreSQL or Redis unless their image tag has changed AND it is intentional.**
**Do not redeploy any other application on the Dokploy host.**

### 4.2 Django API deployment

*Skip if HAS_DJANGO_API=no.*

```
Action: In Dokploy, update the Django service image tag to TARGET_IMAGE_TAG
Method: Dokploy → Project → Application → Service: django-api → Deploy → Set image tag → Trigger deploy

Monitor:
- Observe Dokploy deploy log in real time
- Wait for container to reach "running" state — not just "starting"
- Expected startup log line: "Starting development server" OR "Booting worker" OR "Application startup complete"
- Timeout: 120 seconds — if not running by then, register as blocker
```

### 4.2b Rust Backend deployment (UFOP-specific)

*Skip if HAS_RUST_BACKEND=no.*

```
Action: In Dokploy, update the Rust backend service image tag to TARGET_IMAGE_TAG
Method: Dokploy → Project → Application → Service: rust-backend → Deploy → Set image tag → Trigger deploy

Monitor:
- Observe Dokploy deploy log in real time
- Wait for container to reach "running" state
- Expected startup log: Tauri/Axum/Actix server binding or "Listening on"
- Verify SQLite WAL mode initialization in logs if HAS_SQLITE=yes
- Timeout: 120 seconds — if not running by then, register as blocker
```

### 4.3 Django database migrations

*Skip if HAS_DJANGO_API=no or RUN_MIGRATIONS=no.*

Run migrations ONLY if `RUN_MIGRATIONS=yes` and `MIGRATION_RISK` is not `high-risk`.

```bash
# Execute inside the running Django container via Dokploy exec or docker exec:
python manage.py migrate --no-input

# Capture full output — this is required evidence
# Expected final line: "Running migrations" followed by each migration name
# OR: "No migrations to apply." — both are valid success states

# Verify migration completed:
python manage.py showmigrations | grep "\[ \]"
# Expected: no output (all migrations applied)
# If any [ ] remain: register as blocker
```

Record:
```
Migration command executed: yes / no
Migration output (last 20 lines): ___________
All migrations applied: yes / no / partial
```

### 4.3b SQLite migrations (UFOP-specific)

*Skip if HAS_SQLITE=no.*

SQLite migrations in UFOP run automatically via `all_migrations()` in `src-tauri/src/storage/migrations.rs` on application startup. Verify from container logs:

```bash
# Check migration ran successfully from Rust backend logs:
docker logs <rust-backend-container> --since=5m 2>&1 | grep -i "migration"
# Expected: migration version applied messages or "all migrations up to date"
```

### 4.4 Celery Worker deployment

*Skip if HAS_CELERY_WORKER=no.*

```
Action: In Dokploy, update Celery Worker service image tag to TARGET_IMAGE_TAG → Trigger deploy
Monitor:
- Wait for container running state
- Expected log: "celery@<hostname> ready."
- Check workers have connected to broker: look for "Connected to redis://"
- Timeout: 90 seconds
```

### 4.5 Celery Beat deployment

*Skip if HAS_CELERY_BEAT=no.*

```
Action: In Dokploy, update Celery Beat service image tag to TARGET_IMAGE_TAG → Trigger deploy
Monitor:
- Wait for container running state
- Expected log: "beat: Starting..." followed by schedule entries
- Verify no "ERROR" in first 30 seconds of logs
- Timeout: 60 seconds
```

### 4.6 Next.js frontend deployment

```
Action: In Dokploy, update Next.js service image tag to TARGET_IMAGE_TAG → Trigger deploy
Method: Dokploy → Project → Application → Service: nextjs → Deploy → Set image tag → Trigger deploy

Monitor:
- Wait for container running state
- Expected log: "✓ Ready" or "started server on" with port
- For standalone builds: "Listening on port 3000"
- For UFOP admin console: verify pnpm workspace dependencies resolved
- Timeout: 90 seconds
```

### 4.7 Deploy failure protocol

If any service fails to reach running state:

```
1. Capture exact error from Dokploy deploy log
2. Capture last 50 lines from container logs
3. Classify: is this service-specific or indicates a shared dependency failure?
4. Attempt: one controlled redeploy if error suggests transient issue (pull timeout, etc.)
5. If still failing after one retry: register in Blocker Registry with full evidence
6. Continue with all other unaffected service checks — do not stop the run
7. Do not enter retry loops beyond one controlled retry
```

---

## SECTION 5 — POST-DEPLOY VERIFICATION

All verification results must follow this format — one line per check:

```
[PASS|FAIL|SKIP] | service:check-name | evidence | timestamp
```

No check may be marked PASS without actual evidence. No narrative claims of success.

### 5.1 Container health verification

For each service with `HAS_*=yes`:

```bash
# Check container is running (via docker inspect or Dokploy service status):
docker inspect <container_name> --format '{{.State.Status}} {{.State.StartedAt}} {{.RestartCount}}'

# Expected: running <recent-timestamp> 0
# FAIL if: restarting | exited | restart count > 0 since deploy
```

Emit one check line per service:
```
PASS | container:django-api:running   | status=running, restarts=0, uptime=47s        | <timestamp>
PASS | container:nextjs:running       | status=running, restarts=0, uptime=31s        | <timestamp>
PASS | container:celery-worker:running| status=running, restarts=0, uptime=29s        | <timestamp>
PASS | container:celery-beat:running  | status=running, restarts=0, uptime=22s        | <timestamp>
PASS | container:postgres:running     | status=running, restarts=0, uptime=3h12m      | <timestamp>
PASS | container:redis:running        | status=running, restarts=0, uptime=3h12m      | <timestamp>
PASS | container:rust-backend:running | status=running, restarts=0, uptime=35s        | <timestamp>
```

### 5.2 Django API verification

*Skip if HAS_DJANGO_API=no.*

```bash
# Health endpoint (must be a real endpoint, not guessed):
curl -s -o /dev/null -w "%{http_code}" https://<staging-api-domain>/api/health/
# Expected: 200

# Django admin accessible (not auth — just route exists):
curl -s -o /dev/null -w "%{http_code}" https://<staging-api-domain>/admin/
# Expected: 200 or 302 (redirect to login) — NOT 500

# API root or swagger (if configured):
curl -s -o /dev/null -w "%{http_code}" https://<staging-api-domain>/api/
# Expected: 200

# DRF browsable API or schema endpoint (if configured):
curl -s -o /dev/null -w "%{http_code}" https://<staging-api-domain>/api/schema/
# Expected: 200 or 404 (404 acceptable if schema not exposed)

# Check for Django startup errors in logs:
docker logs <django-container> --since=5m 2>&1 | grep -i "error\|exception\|traceback" | head -20
# Expected: no output — zero errors in startup window
```

Emit checks:
```
PASS | django:health-endpoint     | GET /api/health/ → 200 in 145ms          | <timestamp>
PASS | django:admin-route         | GET /admin/ → 302 in 89ms                | <timestamp>
PASS | django:startup-errors      | 0 errors in logs since deploy            | <timestamp>
PASS | django:migration-applied   | showmigrations: all [ ] cleared          | <timestamp>
```

### 5.2b Rust Backend verification (UFOP-specific)

*Skip if HAS_RUST_BACKEND=no.*

```bash
# Health endpoint:
curl -s -o /dev/null -w "%{http_code}" https://<staging-api-domain>/health
# Expected: 200

# Check for Rust panic or error in logs:
docker logs <rust-backend-container> --since=5m 2>&1 | grep -iE "panic|error|FATAL" | head -20
# Expected: no output — zero panics/errors in startup window

# Verify SQLite initialization (if HAS_SQLITE=yes):
docker logs <rust-backend-container> --since=5m 2>&1 | grep -iE "database|sqlite|migration|WAL" | head -10
# Expected: successful initialization messages
```

Emit checks:
```
PASS | rust-backend:health-endpoint | GET /health → 200 in 45ms               | <timestamp>
PASS | rust-backend:startup-errors  | 0 panics/errors in logs since deploy     | <timestamp>
PASS | rust-backend:sqlite-init     | WAL mode initialized, migrations applied | <timestamp>
```

### 5.3 Next.js frontend verification

```bash
# Root route:
curl -s -o /dev/null -w "%{http_code}" https://<staging-frontend-domain>/
# Expected: 200

# Static asset (Next.js builds always output a main JS bundle):
# Get the actual chunk name from the page HTML first, then verify it loads
curl -s https://<staging-frontend-domain>/ | grep -o '_next/static/[^"]*\.js' | head -1
# Then fetch that asset: must return 200 with content-type: application/javascript

# Login page route:
curl -s -o /dev/null -w "%{http_code}" https://<staging-frontend-domain>/login
# Expected: 200

# API connectivity (frontend should be able to reach backend):
# Check NEXT_PUBLIC_API_URL is set and reachable from the frontend container:
docker exec <nextjs-container> curl -s -o /dev/null -w "%{http_code}" <NEXT_PUBLIC_API_URL>/api/health/
# Expected: 200

# Check for Next.js startup errors:
docker logs <nextjs-container> --since=5m 2>&1 | grep -i "error\|unhandledRejection\|SyntaxError" | head -20
# Expected: no output
```

**UFOP Admin Console additional checks (if this is the UFOP admin):**
```bash
# Verify shadcn/ui CSS loads (Tailwind classes should be present):
curl -s https://<staging-frontend-domain>/ | grep -o '_next/static/css/[^"]*\.css' | head -1
# Fetch the CSS file — must return 200

# Check for React hydration errors in logs:
docker logs <nextjs-container> --since=5m 2>&1 | grep -i "hydration" | head -5
# Expected: no output (hydration mismatches are warnings but should be noted)
```

Emit checks:
```
PASS | nextjs:root-route           | GET / → 200 in 312ms                     | <timestamp>
PASS | nextjs:static-assets        | /_next/static/chunks/main.js → 200       | <timestamp>
PASS | nextjs:login-route          | GET /login → 200 in 201ms                | <timestamp>
PASS | nextjs:api-connectivity     | container→backend health → 200           | <timestamp>
PASS | nextjs:startup-errors       | 0 errors in logs since deploy            | <timestamp>
```

### 5.4 Celery worker verification

*Skip if HAS_CELERY_WORKER=no.*

```bash
# Check Celery is connected to broker and queues registered:
docker exec <celery-container> celery -A <django_app_name> inspect active_queues
# Expected: JSON output listing registered queues — not "Error: No nodes replied"

# Check Celery worker is receiving heartbeats:
docker exec <celery-container> celery -A <django_app_name> inspect ping
# Expected: {"celery@<hostname>": {"ok": "pong"}}

# Check for Celery errors in logs since deploy:
docker logs <celery-container> --since=5m 2>&1 | grep -i "error\|exception\|traceback" | head -20
# Expected: no output — zero task errors in startup window

# Confirm Celery is using correct broker (should be Redis, not amqp):
docker logs <celery-container> --since=10m 2>&1 | grep "Connected to"
# Expected: "Connected to redis://..."
```

Emit checks:
```
PASS | celery:broker-connected     | Connected to redis://... confirmed        | <timestamp>
PASS | celery:ping-response        | celery@host → pong                       | <timestamp>
PASS | celery:queues-registered    | default, priority queues present         | <timestamp>
PASS | celery:startup-errors       | 0 errors in logs since deploy            | <timestamp>
```

### 5.5 Celery Beat verification

*Skip if HAS_CELERY_BEAT=no.*

```bash
# Check Beat is running and scheduling tasks:
docker logs <celery-beat-container> --since=5m 2>&1 | grep "Scheduler"
# Expected: "beat: Scheduler: Sending due task..."

# Verify no missed heartbeat errors:
docker logs <celery-beat-container> --since=5m 2>&1 | grep -i "error\|exception" | head -10
# Expected: no output
```

Emit checks:
```
PASS | celery-beat:scheduler-active| "Sending due task" found in logs         | <timestamp>
PASS | celery-beat:no-errors       | 0 errors in logs since deploy            | <timestamp>
```

### 5.6 PostgreSQL verification

*Skip if HAS_POSTGRESQL=no.*

```bash
# Connection test from Django container:
docker exec <django-container> python manage.py dbshell -c "SELECT version();"
# Expected: PostgreSQL version string

# Check connection count is healthy (not exhausted):
docker exec <postgres-container> psql -U ${DB_USER} -d ${DB_NAME} -c \
  "SELECT count(*) FROM pg_stat_activity WHERE state = 'active';"
# Expected: a reasonable number — not close to max_connections

# Verify all Django migrations are applied:
docker exec <django-container> python manage.py showmigrations | grep "\[ \]"
# Expected: no output (all boxes checked)
```

Emit checks:
```
PASS | postgres:connection         | Django container can connect, SELECT OK  | <timestamp>
PASS | postgres:connection-count   | 12 active / 100 max — healthy            | <timestamp>
PASS | postgres:migrations-applied | No [ ] pending migrations found          | <timestamp>
```

### 5.6b SQLite verification (UFOP-specific)

*Skip if HAS_SQLITE=no.*

```bash
# Verify SQLite database file exists and is accessible:
docker exec <rust-backend-container> ls -la /app/data/*.db 2>/dev/null
# Expected: database file with recent modification time

# Verify WAL mode is active (presence of -wal file):
docker exec <rust-backend-container> ls -la /app/data/*.db-wal 2>/dev/null
# Expected: WAL file present (indicates WAL mode is active)

# Check no SQLite lock issues in logs:
docker logs <rust-backend-container> --since=5m 2>&1 | grep -iE "database is locked|busy|SQLITE_BUSY" | head -5
# Expected: no output
```

Emit checks:
```
PASS | sqlite:db-file-exists       | /app/data/ufop.db present, 2.4MB         | <timestamp>
PASS | sqlite:wal-mode-active      | WAL file present                         | <timestamp>
PASS | sqlite:no-lock-errors       | 0 lock errors in logs since deploy       | <timestamp>
```

### 5.7 Redis verification

*Skip if HAS_REDIS=no.*

```bash
# PING from Django container:
docker exec <django-container> python -c \
  "import redis, os; r=redis.from_url(os.environ['REDIS_URL']); print(r.ping())"
# Expected: True

# PING from Celery container:
docker exec <celery-container> python -c \
  "import redis, os; r=redis.from_url(os.environ['CELERY_BROKER_URL']); print(r.ping())"
# Expected: True

# Memory usage is not critical:
docker exec <redis-container> redis-cli INFO memory | grep "used_memory_human"
# Expected: reasonable value — document what is found
```

Emit checks:
```
PASS | redis:ping-from-django      | redis.ping() → True from django container | <timestamp>
PASS | redis:ping-from-celery      | redis.ping() → True from celery container | <timestamp>
PASS | redis:memory-status         | used_memory_human: 45.12M — healthy       | <timestamp>
```

### 5.8 Domain, TLS, and reverse proxy verification

```bash
# For each domain in TARGET_DOMAINS:
curl -sv https://<domain>/ 2>&1 | grep -E "issuer|expire|subject|HTTP"
# Expected: valid TLS cert, not expired, correct issuer (Let's Encrypt or configured CA)

# Verify Dokploy Traefik proxy is routing correctly:
curl -s -o /dev/null -w "%{http_code} %{url_effective}" https://<domain>/
# Expected: 200 routing to frontend OR 302 to correct location

# TLS expiry check:
echo | openssl s_client -connect <domain>:443 2>/dev/null | openssl x509 -noout -dates
# Expected: notAfter is MORE than 14 days from today
# If less than 14 days: register as Tier-2 blocker — Dokploy cert renewal may be needed
```

Emit checks:
```
PASS | tls:valid-cert              | notAfter: 2025-07-12, 91 days remaining   | <timestamp>
PASS | proxy:routing-correct       | https://staging.myapp.com → 200          | <timestamp>
PASS | proxy:traefik-running       | Traefik container running on host         | <timestamp>
```

### 5.9 Shared environment regression surveillance

Because this is a shared Dokploy server with potentially other applications:

```bash
# List ALL applications on this Dokploy host:
# Dokploy API: GET /api/applications → list all apps

# For each OTHER application (not TARGET_APP_NAME):
# Perform lightweight reachability check only — do NOT test their internals
curl -s -o /dev/null -w "%{http_code}" https://<other-app-domain>/
# Expected: any 2xx or 3xx — NOT 502/503/504 (which would indicate proxy damage)
```

```
Other apps checked for collateral damage:
  - App: _________ | Domain: _________ | Status before: ___ | Status after: ___ | Impacted: yes/no
  - App: _________ | Domain: _________ | Status before: ___ | Status after: ___ | Impacted: yes/no
```

### 5.10 Playwright / browser verification (if Playwright or Chrome DevTools MCP is available)

If Playwright MCP or Chrome DevTools MCP is connected, run these checks:

```
1. Navigate to https://<staging-frontend-domain>/login
   - Screenshot: login-page.png
   - Assert: no translation keys visible (no text matching *.key or pageName.*)
   - Assert: no console errors

2. Authenticate with staging credentials
   - Fill email and password fields
   - Submit form
   - Assert: redirect to /dashboard (or equivalent)
   - Screenshot: dashboard-after-login.png

3. Navigate to a key authenticated page
   - Assert: page loads with real content (not skeleton/spinner stuck)
   - Assert: no 500 errors visible
   - Screenshot: authenticated-page.png

4. Check one API-backed data load
   - Assert: data table or list renders with actual records (not empty due to API failure)
```

**UFOP Admin Console browser checks (if deploying UFOP admin):**
```
5. Verify shadcn/ui components render correctly
   - Assert: Tailwind CSS classes applied (no unstyled raw HTML)
   - Assert: lucide-react icons load (no missing icon placeholders)

6. Check for Zustand persist rehydration
   - Assert: no visible "flash" of default state before persisted state loads
   - Note: React hydration mismatch warnings in console are non-blocking but should be documented

7. Verify TanStack Table renders if present on dashboard
   - Assert: table headers and rows visible
   - Assert: no "undefined" or raw object strings in cells
```

If no browser tool is available, document that browser checks were skipped and note manual verification is required.

---

## SECTION 6 — BLOCKER REGISTRY

Maintain throughout the run. Every blocker must be logged — including resolved ones.

```
BLOCKER-{N}
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Timestamp:
Tier:                   1 (self-actionable) | 2 (non-blocking) | 3 (escalate to human)
Scope:                  service | application | shared-infra | control-plane
Affected service(s):
Affected checks:        (list Section 5 check IDs that are blocked)
Symptoms:
Evidence (exact):
Probable root cause:
Mitigation attempted:
Mitigation result:      resolved | partial | unresolved
Can other checks continue? yes — list them | no — explain why not
Rollback needed:        yes | no | unknown
Human review required:  yes | no
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

### Tier definitions

| Tier | Meaning | Agent action |
|------|---------|--------------|
| 1 | Blocking but agent can fix | Fix it, retry, continue |
| 2 | Non-blocking — app runs with degradation | Document, continue, note in final report |
| 3 | Risk too high for autonomous action | Stop write actions on affected scope, escalate, continue unaffected checks |

### Continuation mandate

A Tier-1 or Tier-2 blocker on one service MUST NOT stop these checks from continuing:

| Blocked service | These checks must still run |
|-----------------|-----------------------------|
| Django API fails | Next.js deploy, Redis check, PostgreSQL connection check, TLS check, shared-app regression |
| Rust backend fails | Next.js deploy, all infrastructure checks, shared-app regression |
| Celery Worker fails | Django health, Next.js health, all database/cache checks, shared-app regression |
| Next.js fails | All backend checks, all infrastructure checks |
| Migration fails | Container health checks, TLS checks, Redis/PostgreSQL health |
| Dokploy panel unreachable | Read-only docker inspect, HTTP route checks, container log review |
| SQLite locked | Container health, HTTP endpoint checks, TLS checks |

Only a Tier-3 blocker affecting shared infrastructure halts write actions for affected scope.
All read-only checks continue regardless.

---

## SECTION 7 — ROLLBACK LOGIC

### 7.1 When to recommend rollback

Recommend rollback when ANY of these conditions are true:
- Django API container is not running after deploy + one retry
- Rust backend container is not running after deploy + one retry (UFOP)
- Database migration caused an error and left schema in inconsistent state
- More than `ROLLBACK_AUTO_FAIL_THRESHOLD` consecutive health checks fail
- A Tier-3 blocker is detected that originated from this release (not pre-existing)
- NEXT_PUBLIC_API_URL is confirmed pointing to production domain
- SQLite database is corrupted or locked indefinitely (UFOP)

### 7.2 Manual rollback steps (Dokploy-specific)

```
If ROLLBACK_TRIGGER=manual and rollback is recommended:

1. Document: "ROLLBACK RECOMMENDED — [specific reason] — human action required"

2. Exact rollback procedure:
   a. Dokploy → Project → Application → Service: django-api (or rust-backend for UFOP)
      → Change image tag to ROLLBACK_IMAGE_TAG → Deploy
   b. Wait for backend to reach running state
   c. If Django migrations ran: verify whether down-migration is needed
      - If down-migration exists: docker exec <container> python manage.py migrate <app> <previous-migration>
      - If not: document data loss risk and escalate
   c2. If SQLite migrations ran (UFOP): verify backward compatibility
      - SQLite migrations in UFOP are auto-applied — rollback may require manual DB intervention
      - Document risk and escalate if not backward-compatible
   d. Dokploy → Service: celery-worker → Change image tag to ROLLBACK_IMAGE_TAG → Deploy
   e. Dokploy → Service: celery-beat → Change image tag to ROLLBACK_IMAGE_TAG → Deploy
   f. Dokploy → Service: nextjs → Change image tag to ROLLBACK_IMAGE_TAG → Deploy
   g. Verify all services healthy using Section 5 checks against ROLLBACK_IMAGE_TAG

3. After rollback: re-run full Section 5 verification
4. Emit rollback outcome in final report
```

### 7.3 Auto-rollback procedure

```
If ROLLBACK_TRIGGER=auto AND health check failures >= ROLLBACK_AUTO_FAIL_THRESHOLD:

1. Emit: "AUTO-ROLLBACK TRIGGERED | failures: N | threshold: ROLLBACK_AUTO_FAIL_THRESHOLD | <timestamp>"
2. Record pre-rollback state
3. Execute steps 2a–2g from Section 7.2 autonomously
4. Verify rollback health
5. Report: ROLLBACK COMPLETED | new status: <result>
```

### 7.4 Migration rollback caveat

```
If RUN_MIGRATIONS=yes and rollback is required:
- Check for down-migration script in migrations/ folder
- If found: document exact command and execute if rollback proceeds
- If NOT found: emit "ROLLBACK MIGRATION RISK — no reverse migration — data loss possible"
  → Do NOT auto-rollback if this warning is present
  → Escalate to human as Tier-3 blocker

If HAS_SQLITE=yes (UFOP) and rollback is required:
- UFOP SQLite migrations (src-tauri/src/storage/migrations.rs) are forward-only
- Check if rollback image is compatible with current schema version
- If not backward-compatible: emit "SQLITE ROLLBACK RISK — schema version mismatch"
  → Escalate to human as Tier-3 blocker
```

---

## SECTION 8 — CONTEXT CHECKPOINT SYSTEM

After completing each major section, emit a checkpoint in this exact format.
This enables a new Claude Code context to resume from exactly where execution stopped.

```json
{
  "run_id": "<RUN_ID>",
  "timestamp": "<ISO8601>",
  "agent_version": "v3.0",
  "target_app": "<TARGET_APP_NAME>",
  "target_tag": "<TARGET_IMAGE_TAG>",
  "sections_completed": ["S0", "S1", "S2"],
  "current_section": "S3",
  "current_task": "Gate 3 — Environment completeness",
  "blockers_active": [
    {"id": "BLOCKER-1", "tier": 2, "scope": "service", "service": "celery-beat", "status": "unresolved"}
  ],
  "service_status": {
    "django-api": "deployed-verified",
    "rust-backend": "not-applicable",
    "nextjs": "deployed-unverified",
    "celery-worker": "not-started",
    "celery-beat": "blocked-BLOCKER-1",
    "postgres": "verified-pre-existing",
    "redis": "verified-pre-existing",
    "sqlite": "not-applicable"
  },
  "health_checks_passed": 12,
  "health_checks_failed": 1,
  "health_checks_skipped": 2,
  "rollback_candidate": "<ROLLBACK_IMAGE_TAG>",
  "next_action": "Run Gate 4 — dependency health checks"
}
```

**Context exhaustion protocol:**
If the context window is approaching its limit:
1. Emit the full checkpoint immediately
2. State: `"CONTEXT LIMIT APPROACHING — checkpoint emitted — resume from section [X] task [Y]"`
3. New agent context MUST start by reading the checkpoint and resuming from `next_action`

---

## SECTION 9 — FINAL REPORT

Emit this report at the end of every run, even if incomplete.

### A. Executive outcome
```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
STAGING DEPLOYMENT REPORT
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Run ID:
Status:              READY | DEPLOYED-WITH-DEFECTS | PARTIAL | NOT-SAFE
Application:
Release deployed:
Git commit:
Deploy platform:     Dokploy
Deploy timestamp:
Staging ready:       yes | no | partial
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

### B. Service deployment summary

| Service | Image Tag Deployed | Container Status | Health Checks | Issues |
|---------|--------------------|------------------|---------------|--------|
| Django API | | | pass/fail/N/A | |
| Rust Backend (UFOP) | | | pass/fail/N/A | |
| Next.js | | | pass/fail | |
| Celery Worker | | | pass/fail/N/A | |
| Celery Beat | | | pass/fail/N/A | |
| PostgreSQL | (unchanged) | | pass/fail/N/A | |
| Redis | (unchanged) | | pass/fail/N/A | |
| SQLite (UFOP) | (embedded) | | pass/fail/N/A | |

### C. Complete health check log

Full structured log of every check run in Section 5:

```
[PASS|FAIL|SKIP] | check-name | evidence | timestamp
[PASS|FAIL|SKIP] | check-name | evidence | timestamp
...
```

**Summary:**
```
Total checks:    ___
PASS:            ___
FAIL:            ___
SKIP:            ___
Pass rate:       ___%
```

### D. Blocker summary

All blockers from Section 6, grouped by tier:

```
Tier 1 (self-actionable):   ___ blockers — ___ resolved | ___ unresolved
Tier 2 (non-blocking):      ___ blockers — documented
Tier 3 (escalate):          ___ blockers — human action required
```

Full blocker entries repeated from Section 6.

### E. Migration report
```
Migrations run:        yes | no | skipped
Migration risk:        none | safe | caution | high-risk
Migrations applied:    list of migration names
Migrations failed:     none | list
Down-migration exists: yes | no
SQLite migrations:     applied | not applicable | failed
```

### F. Shared environment safety
```
Other Dokploy apps checked:    N apps
Collateral damage found:       none | list affected apps
Traefik proxy status:          healthy | degraded
```

### G. Rollback position
```
Rollback candidate:       <ROLLBACK_IMAGE_TAG>
Rollback feasibility:     safe | migration-risk | blocked
Rollback recommendation:  not needed | recommended | required
Rollback command:         [exact Dokploy steps from Section 7.2]
```

### H. Final status (one of four — no other options)

```
READY ON STAGING
  → All critical checks pass. No blocking issues. Staging is deployable.

DEPLOYED WITH NON-BLOCKING DEFECTS
  → Application is running. Minor Tier-2 issues found. Document before production.

PARTIALLY DEPLOYED / PARTIALLY VERIFIED
  → Some services deployed and verified. Others blocked. Continue from checkpoint.

NOT SAFE TO DECLARE STAGING READY
  → Critical failures found. Rollback recommended or required.
```

### I. Exact next action

> One sentence — the single most important next operational step.

---

## SECTION 10 — ANTI-FAILURE ENFORCEMENT

Before declaring the run complete, verify every item below is true.
If any are unchecked, the run is incomplete.

**Discovery:**
- [ ] Dokploy topology confirmed (project, app, all services found)
- [ ] Stack confirmed (Django/Next.js/Celery/PostgreSQL/Redis and/or UFOP Rust/Next.js/SQLite)
- [ ] Current state snapshot captured before any write action
- [ ] All env var keys audited — no MISSING keys proceed without documentation
- [ ] No production contamination found in staging env (or Tier-3 blocker registered)
- [ ] Migration risk classified and `MIGRATION_RISK` set

**Safety gates:**
- [ ] All 6 gates evaluated (not skipped)
- [ ] Target images exist in GHCR at pinned tag
- [ ] Rollback candidate confirmed and exists in GHCR

**Deployment:**
- [ ] Services deployed in correct order (DB first, then backend, then workers, then frontend)
- [ ] Only TARGET_APP_NAME services were touched
- [ ] No other Dokploy applications were modified

**Verification:**
- [ ] All applicable verification subsections executed (or SKIP emitted with reason)
- [ ] Every PASS has real evidence — not a narrative claim
- [ ] Full structured health check log emitted

**Continuity:**
- [ ] At least one context checkpoint emitted
- [ ] All active blockers in Blocker Registry with tier and scope
- [ ] No blocker stopped independent checks from running

**Report:**
- [ ] Final report sections A through I all present
- [ ] Final status explicitly declared (one of the 4 valid outcomes)
- [ ] Exact next action stated in one sentence

---

## SECTION 11 — QUICK-START: HOW TO USE THIS RUNBOOK

### Fill in the variables, hand to Claude Code

**Example for a full-stack app (Django + Next.js + Celery):**

```bash
TARGET_ENVIRONMENT=staging
RUN_ID="$(date +%Y%m%d-%H%M%S)"

DOKPLOY_HOST=staging.rateads.com
DOKPLOY_PORT=3000
DOKPLOY_API_TOKEN=<from Dokploy settings>

DOKPLOY_PROJECT_NAME=rateads
TARGET_APP_NAME=rateads-staging
TARGET_DOMAINS=staging.rateads.com,staging-api.rateads.com

HAS_DJANGO_API=yes
HAS_NEXTJS_FRONTEND=yes
HAS_CELERY_WORKER=yes
HAS_CELERY_BEAT=yes
HAS_POSTGRESQL=yes
HAS_REDIS=yes
HAS_NGINX_PROXY=no
HAS_RUST_BACKEND=no
HAS_SQLITE=no

TARGET_IMAGE_TAG=v1.4.2
TARGET_GIT_COMMIT=a3f8e9c14d2b...

IMAGE_REGISTRY=ghcr.io
GHCR_ORG=my-github-org
DJANGO_IMAGE=ghcr.io/my-github-org/backend:v1.4.2
NEXTJS_IMAGE=ghcr.io/my-github-org/frontend:v1.4.2

DB_SERVICE_NAME=rateads-postgres
DB_NAME=rateads_staging
DB_USER=rateads_staging_user

REDIS_SERVICE_NAME=rateads-redis

RUN_MIGRATIONS=yes
MIGRATION_RISK=safe            # set after reviewing git diff in Section 2.5

ROLLBACK_IMAGE_TAG=v1.4.1
ROLLBACK_TRIGGER=manual
ROLLBACK_AUTO_FAIL_THRESHOLD=3
```

### For a UFOP admin console deploy (Next.js only, no Django)

```bash
HAS_DJANGO_API=no
HAS_NEXTJS_FRONTEND=yes             # UFOP admin console
HAS_CELERY_WORKER=no
HAS_CELERY_BEAT=no
HAS_POSTGRESQL=no                   # Still verify health if shared host has it
HAS_REDIS=no
HAS_RUST_BACKEND=no                 # Desktop app — not containerized
HAS_SQLITE=no                       # SQLite is inside the Tauri desktop app
HAS_NGINX_PROXY=no
RUN_MIGRATIONS=no

NEXTJS_IMAGE=ghcr.io/my-github-org/ufop-admin:v2.1.0
```

### For a Next.js-only deploy (no Django changes)

```bash
HAS_DJANGO_API=no              # Skip Django deploy — existing container unchanged
HAS_NEXTJS_FRONTEND=yes
HAS_CELERY_WORKER=no
HAS_CELERY_BEAT=no
HAS_POSTGRESQL=no              # Still verify it's healthy, but don't touch it
HAS_REDIS=no
RUN_MIGRATIONS=no
```

### For a Django-only deploy (no frontend changes)

```bash
HAS_DJANGO_API=yes
HAS_NEXTJS_FRONTEND=no         # Skip Next.js deploy — existing container unchanged
HAS_CELERY_WORKER=yes          # Workers need new image if Django code changed
HAS_CELERY_BEAT=yes
HAS_POSTGRESQL=yes
HAS_REDIS=yes
RUN_MIGRATIONS=yes
```

---

*End of Dokploy Staging Deployment + Verification Agent Runbook v3.0*
*Stack: Django · Next.js · PostgreSQL · Redis · Celery · Docker · GHCR*
*UFOP Stack: Rust/Tauri · Next.js Admin · SQLite/rusqlite · Zustand · shadcn/ui · pnpm*
*Platform: Dokploy (exclusive — never Coolify)*
