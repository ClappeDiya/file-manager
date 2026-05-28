---
name: production-e2e-verification
description: Use when verifying a production deployment on Dokploy, after deploying to production, when asked to "verify production", "check production", "run post-deploy checks", "is production working", or any production health/readiness validation. The deployment platform is Dokploy. Tech stack: Tauri 2.0 desktop app (React + TypeScript + Vite + Rust), Next.js admin console, pnpm workspaces, SQLite (rusqlite), Zustand, shadcn/ui, Tailwind CSS, React Aria, TanStack (Table/Virtual/Query).
---

# Universal Production E2E Verification Runbook — DOKPLOY — Post-Deployment Live Validation

Version 3.0 — Customized for UFOP (Unified File Operations Platform)

**Tech Stack Context:**
- **Desktop App:** Tauri 2.0 shell + Rust core engine (tokio async) + React/TypeScript/Vite frontend
- **Admin Console:** Next.js + TypeScript (port 3001)
- **State Management:** Zustand (with `persist()` middleware)
- **UI:** shadcn/ui + React Aria + Tailwind CSS + lucide-react icons
- **Data:** SQLite via rusqlite (`Arc<Mutex<Connection>>` pool, WAL mode)
- **Shared Packages:** `@ufop/design-tokens`, `@ufop/ui-components`
- **Package Manager:** pnpm 10.27.0 workspaces
- **IPC Boundary:** Tauri IPC commands (`#[tauri::command]` in Rust, `tauriInvoke<T>()` on frontend)
- **Deployment Platform:** Dokploy

---

## 0. BINDING EXECUTION DIRECTIVE

You are an autonomous **production post-deployment E2E verification agent**.

This document is an instruction set, not a discussion document.

Upon receiving this runbook, begin execution immediately:

- Do **not** ask whether to proceed
- Do **not** ask what to test
- Do **not** reduce scope
- Do **not** silently skip checks
- Do **not** treat logs alone as proof of health
- Do **not** make production changes unless explicitly allowed below
- Begin at **Section 1** and work through in order
- Extract all knowable parameters from the user's prompt, conversation history, and attached files before asking any question
- Only ask the user a question if the **production URL is completely unknowable** from context

This runbook is for **production**. The deployment platform is **Dokploy**. The application is already deployed. Your task is to verify real production behavior quickly, safely, and thoroughly enough to surface issues before they reach real users.

If one verification path is blocked, continue all remaining safe and independent work.

---

## 1. PARAMETER RESOLUTION — SILENT DISCOVERY FIRST

### 1.1 Resolution order

Resolve every operational parameter in this order. Do **not** ask the user unless you reach step 4:

1. **Extract from the user's prompt** — scan for domains, URLs, project names, credentials, SSH details
2. **Extract from conversation history** — prior messages may contain credentials, domains, project context
3. **Extract from attached files or project knowledge** — config files, deployment docs, env files, docker-compose files, CLAUDE.md credentials sections
4. **Ask the user once** — only if the production URL is still unknown after steps 1–3

If you must ask, ask exactly once, requesting only what is missing:

> To verify production, I need the live domain (e.g., `https://app.example.com`). Optionally: SSH access command, login credentials, Dokploy dashboard URL.

### 1.2 Parameters to resolve

| Parameter | Required | Fallback if unknown |
|---|---|---|
| **Production URL** | YES | Cannot proceed — ask user |
| **Project name** | No | Discover from containers or domain |
| **SSH access** (connection string) | No | Switch to HTTP-only mode |
| **Login credentials** (email + password) | No | Skip auth checks, mark BLOCKED |
| **Dokploy dashboard URL** | No | Skip Dokploy UI checks |

### 1.3 State tracking

Create a local state file immediately using the available file-write tool. This file persists all discovered values so they can be referenced throughout the run without relying on shell variable persistence between commands.

**File name:** `verification-state.md`
**Location:** Any writable workspace directory available to the agent

Contents:

```markdown
# Verification State — Updated Throughout Run

## Parameters
- PRODUCTION_URL: <resolved>
- PROJECT_NAME: <resolved or "discovering">
- SSH_COMMAND: <resolved or "unavailable">
- SSH_AVAILABLE: true / false
- VERIFICATION_EMAIL: <resolved or "not provided">
- VERIFICATION_PASSWORD: <resolved or "not provided">
- HAS_CREDENTIALS: true / false
- DOKPLOY_URL: <resolved or "not provided">
- RUN_ID: <YYYYMMDD-HHMMSS>
- OPERATING_MODE: <A / B / C — set in Section 2>

## Discovered Containers
- FRONTEND_CONTAINER: <discovered or "unknown">
- BACKEND_CONTAINER: <discovered or "unknown">
- DB_CONTAINER: <discovered or "unknown">
- CACHE_CONTAINER: <discovered or "unknown">
- WORKER_CONTAINER: <discovered or "unknown">
- SCHEDULER_CONTAINER: <discovered or "unknown">

## Detected Stack
- FRONTEND: <e.g., nextjs, react-spa, vue, static, unknown>
- BACKEND: <e.g., django, fastapi, express, rails, rust-tauri, unknown>
- DATABASE: <e.g., postgres, mysql, mongodb, sqlite, unknown>
- CACHE: <e.g., redis, memcached, none>
- WORKER: <e.g., celery, sidekiq, bullmq, tokio-workers, none>
- I18N: true / false

## Discovered Routes
- LOGIN_PATH: <e.g., /login>
- DASHBOARD_PATH: <e.g., /dashboard>
- API_BASE: <e.g., /api/v1>
- HEALTH_ENDPOINT: <e.g., /api/health>
- ADDITIONAL_ROUTES: <comma-separated list>

## Blocker Count
- Total: 0
- Tier 1: 0
- Tier 2: 0
- Tier 3: 0
```

Update this file every time you discover or confirm a value. Read from it before constructing any command.

---

## 1.4 PLACEHOLDER SUBSTITUTION RULE — MANDATORY

Throughout this runbook, command examples contain **uppercase placeholder tokens** such as `PRODUCTION_URL`, `SSH_COMMAND`, `BACKEND_CONTAINER`, `LOGIN_PATH`, etc.

**Before executing any command**, you must:

1. Read the current value from `verification-state.md`
2. Substitute the placeholder with the real value
3. If the value is `"unknown"` or `"unavailable"`, do **not** execute the command — mark the check as `BLOCKED` with the reason

**You must never execute a command containing a raw placeholder token.** A command like `curl https://PRODUCTION_URL/` is documentation. The executed command must be `curl https://app.example.com/`.

---

## 2. OPERATING MODE SELECTION

Based on available access, select exactly one mode. This determines which checks are possible.

### Mode A: Full Access

**Requirements:** SSH to production VPS works + browser automation tool available + production URL reachable via HTTP

**Capabilities:** All checks — container inspection, Docker logs, browser verification, HTTP checks, database path validation, worker status, Dokploy state

### Mode B: HTTP + Browser Only

**Requirements:** Production URL reachable + browser automation tool available. No SSH.

**Capabilities:** Domain/TLS checks, browser page verification, HTTP API checks, asset loading, auth flow, performance timing, console error capture

**Cannot do:** Container inspection, Docker logs, restart counts, worker status, DB container checks, image tag verification, host-level Dokploy state

**Mark all infrastructure checks as:** `BLOCKED — No SSH access to production host`

### Mode C: HTTP Only

**Requirements:** Production URL reachable via HTTP client. No SSH, no browser automation.

**Capabilities:** Domain/TLS checks, HTTP status codes, API response codes, response timing, header inspection

**Cannot do:** Everything from Mode B exclusions, plus: visual page verification, screenshot capture, console error capture, auth flow testing via browser

**Mark all browser checks as:** `BLOCKED — No browser automation tool available`

### Selection logic

```
IF SSH available AND browser tool available → Mode A
ELSE IF browser tool available (no SSH)     → Mode B
ELSE IF HTTP client can reach production    → Mode C
ELSE → BLOCKED — No production access. Report immediately.
```

Record the selected mode in `verification-state.md`. Every subsequent section states which modes it applies to.

---

## 3. TOOL USAGE GUIDELINES

This runbook is tool-agnostic. Use whatever tools are available in your execution environment.

| Task | Use |
|---|---|
| Run shell commands locally | The available shell/terminal/bash tool |
| Run commands on production VPS | Shell tool executing `ssh <connection> "<command>"` |
| Open pages in a browser | The available browser automation tool (Playwright MCP, Chrome DevTools MCP, or equivalent) |
| Capture screenshots | The browser tool's screenshot capability |
| Capture browser console output | The browser tool's console event listener |
| Make HTTP requests without a browser | Shell tool with `curl`, `wget`, or equivalent |
| Write files (reports, state) | The available file-write/create tool (Write tool) |
| Read files | The available file-read/view tool (Read tool) |
| Edit files | The available file-edit/replace tool (Edit tool) |

### UFOP-Specific Tool Notes

- **Playwright MCP** or **Chrome DevTools MCP** — preferred for browser verification of the Next.js admin console
- For Tauri desktop app verification, browser tools verify the admin console web interface; the desktop app itself requires different verification approaches
- The admin console runs on port 3001 in dev; verify the production deployment URL instead

### If a tool is unavailable

- If **no browser tool**: operate in Mode C. Mark browser checks BLOCKED.
- If **no SSH**: operate in Mode B or C. Mark infrastructure checks BLOCKED.
- If **no HTTP client**: the run cannot proceed. Report immediately.
- **Never install software on the production server.**

### SSH command pattern

All Docker commands run on the remote VPS through SSH, not locally:

```bash
# Correct — executes on the VPS
ssh user@1.2.3.4 "docker ps --format '{{.Names}}: {{.Status}}'"

# Wrong — executes in the local sandbox which has no Docker
docker ps
```

Always substitute the real SSH connection string from `verification-state.md`.

### Time budget

| Phase | Maximum |
|---|---|
| Parameter resolution + mode selection (Sections 1–2) | 3 min |
| Discovery + topology mapping (Section 4) | 5 min |
| Fast-fail critical path (Section 5) | 10 min |
| Extended browser verification (Section 6) | 15 min |
| API, DB, worker, i18n, performance (Sections 8–12) | 10 min |
| Report generation (Section 16) | 5 min |
| **Total** | **48 min max** |

If exceeding 48 minutes: stop active checks, document all findings, deliver a partial report with the `PARTIALLY VERIFIED` verdict.

---

## 4. PRE-VERIFICATION DISCOVERY — MANDATORY

### 4.1 Domain reachability (All modes)

```bash
curl -sS -o /dev/null -w "Status: %{http_code}\nTime: %{time_total}s\nRedirect: %{redirect_url}\n" "https://PRODUCTION_URL/"
```

```bash
echo | openssl s_client -connect PRODUCTION_URL:443 -servername PRODUCTION_URL 2>/dev/null | openssl x509 -noout -dates -subject 2>/dev/null
```

If the domain is unreachable → the entire run is `BLOCKED`. Write the report immediately.

### 4.2 Service topology (Mode A only)

```bash
ssh SSH_COMMAND "docker ps --format '{{.Names}}|||{{.Image}}|||{{.Status}}|||{{.Ports}}|||{{.RunningFor}}'"
```

From the output, identify each service role using these signals:

| Signal in image name or container name | Likely role |
|---|---|
| `node`, `next`, `frontend`, port 3000/3001 | Frontend (Next.js / UFOP Admin Console) |
| `react`, `nginx`, `static`, port 80/443 | Frontend (SPA or static) |
| `python`, `django`, `gunicorn`, `uvicorn`, `backend`, `api`, port 8000 | Backend (Django/FastAPI) |
| `express`, `nest`, `koa`, port 3001/4000/5000 | Backend (Node.js) |
| `rust`, `tauri`, `axum`, `actix` | Backend (Rust — UFOP core engine) |
| `ruby`, `rails`, `puma`, port 3000 | Backend (Rails) |
| `postgres`, `pg`, port 5432 | Database (PostgreSQL) |
| `mysql`, `mariadb`, port 3306 | Database (MySQL) |
| `mongo`, port 27017 | Database (MongoDB) |
| `sqlite` | Database (SQLite — UFOP default) |
| `redis`, port 6379 | Cache / message broker |
| `memcached`, port 11211 | Cache |
| `celery`, `worker` | Async worker (Celery) |
| `sidekiq` | Async worker (Sidekiq) |
| `bull`, `bullmq` | Async worker (BullMQ) |
| `beat`, `scheduler`, `cron` | Periodic task scheduler |

Record every discovered container name in `verification-state.md` under the correct role.

If a role has no matching container, record `"none"`.

### 4.3 Stack detection (Mode A only)

```bash
ssh SSH_COMMAND "docker ps --format '{{.Image}}' | sort -u"
```

### 4.4 I18N detection (Mode A only)

Try each command in order. Stop at the first that succeeds:

```bash
# Try 1: Check for locales directory
ssh SSH_COMMAND "docker exec FRONTEND_CONTAINER ls /app/public/locales/ 2>/dev/null"

# Try 2: Check alternate locale paths
ssh SSH_COMMAND "docker exec FRONTEND_CONTAINER ls /app/src/locales/ 2>/dev/null"
ssh SSH_COMMAND "docker exec FRONTEND_CONTAINER ls /app/locales/ 2>/dev/null"

# Try 3: Check package.json for i18n dependencies
ssh SSH_COMMAND "docker exec FRONTEND_CONTAINER cat /app/package.json 2>/dev/null | grep -iE 'i18next|react-intl|next-intl|vue-i18n|i18n' || echo 'no-i18n-package'"

# Try 4: Search for any i18n config file
ssh SSH_COMMAND "docker exec FRONTEND_CONTAINER find /app -maxdepth 3 -name '*i18n*' -o -name '*locale*' -o -name '*intl*' 2>/dev/null | head -5"
```

If **all** fail or return empty: set `I18N: false` in state file. If **any** succeed: set `I18N: true`.

If the frontend container does not support `ls`, `cat`, or `find` (minimal image): set `I18N: unknown` and attempt detection via browser in Section 9.

### 4.5 Release identity (Mode A only)

```bash
ssh SSH_COMMAND "docker inspect --format '{{.Name}}: {{.Config.Image}} | Started: {{.State.StartedAt}}' \$(docker ps -q) 2>/dev/null"
```

```bash
ssh SSH_COMMAND "docker inspect --format '{{.Name}}: Restarts={{.RestartCount}}' \$(docker ps -q) 2>/dev/null"
```

If any container has >3 restarts, flag it.

### 4.6 Rollback position (Mode A only)

```bash
ssh SSH_COMMAND "docker images --format 'table {{.Repository}}\t{{.Tag}}\t{{.CreatedAt}}' | head -20"
```

### 4.7 Route discovery (All modes)

**Phase 1 — Probe universal routes via HTTP (all modes):**

```bash
for path in "/" "/login" "/signin" "/auth/login" "/auth/signin"; do
  code=$(curl -sS -o /dev/null -w "%{http_code}" "https://PRODUCTION_URL${path}" 2>/dev/null)
  [ "$code" != "404" ] && [ "$code" != "000" ] && echo "LOGIN candidate: ${path} (${code})"
done
```

```bash
for path in "/dashboard" "/app" "/home" "/admin" "/overview"; do
  code=$(curl -sS -o /dev/null -w "%{http_code}" "https://PRODUCTION_URL${path}" 2>/dev/null)
  [ "$code" != "404" ] && [ "$code" != "000" ] && echo "DASHBOARD candidate: ${path} (${code})"
done
```

```bash
for path in "/api/health" "/health" "/healthz" "/api/v1/" "/api/" "/status"; do
  code=$(curl -sS -o /dev/null -w "%{http_code}" "https://PRODUCTION_URL${path}" 2>/dev/null)
  [ "$code" != "404" ] && [ "$code" != "000" ] && echo "HEALTH candidate: ${path} (${code})"
done
```

Use the first non-404 path for each role. Record in `verification-state.md`.

**Phase 2 — Deeper route discovery (Mode A only, best-effort):**

Try each. If a command fails, skip it silently:

```bash
# Next.js built routes (UFOP admin console uses Next.js)
ssh SSH_COMMAND "docker exec FRONTEND_CONTAINER find /app/.next/server/app -maxdepth 3 -name '*.html' 2>/dev/null | head -20"

# Django URL list (only if django backend)
ssh SSH_COMMAND "docker exec BACKEND_CONTAINER python manage.py show_urls 2>/dev/null | head -20"

# Express/Node route listing (rarely available without custom command)
# Skip if not available

# Nginx/proxy route mapping
ssh SSH_COMMAND "docker exec \$(docker ps --filter 'name=nginx\|proxy\|traefik\|caddy' -q | head -1) cat /etc/nginx/conf.d/*.conf 2>/dev/null | grep 'location' | head -15"
```

If deeper discovery produces no results, rely on Phase 1 probes plus any routes the user mentioned.

---

## 5. PRODUCTION SAFETY RULES

### Allowed

- Read-only SSH inspection (docker ps, logs, inspect, stats, images)
- HTTP GET/HEAD to any endpoint
- HTTP POST only to login/auth endpoints with user-provided credentials
- Browser navigation and screenshot capture
- File creation in agent workspace

### Forbidden

- Redeploy, restart, rebuild, or scale any container
- Modify environment variables, config files, or secrets
- Run migrations, schema changes, or seed scripts
- Direct database reads, writes, or queries
- Clear caches, queues, or broker state
- Trigger background jobs or scheduled tasks
- Create, modify, or delete any production data
- Install packages on production servers

### Protected zones — never touch

- Payment, billing, money movement
- Order execution, transactions
- User records, profiles, account data
- Authorization rules, token logic
- Audit trails, compliance logs
- Production database contents (SQLite or otherwise)
- Tauri IPC command handlers (production Rust binary)

---

## 6. FAST-FAIL CRITICAL PATH — EXECUTE IN THIS ORDER

These checks surface the most damaging failures first. If checks 1–3 all fail, the release may be fundamentally broken.

### Check 1: Domain + TLS (All modes)

```bash
curl -sS -o /dev/null -w "HTTPS: %{http_code} | Time: %{time_total}s\n" "https://PRODUCTION_URL/"
```

| Result | Verdict |
|---|---|
| 200, 301, or 302 | PASSED |
| 502, 503 | FAILED Tier 1 — app not serving |
| 000 / connection refused / timeout | FAILED Tier 1 — unreachable |
| TLS handshake error | FAILED Tier 1 — certificate problem |

### Check 2: Container health (Mode A only)

```bash
ssh SSH_COMMAND "docker ps --format '{{.Names}}: {{.Status}}' | grep -iE 'PROJECT_NAME\|frontend\|backend\|worker\|redis\|postgres\|mysql\|mongo\|next\|admin'"
```

Flag: any service showing `Restarting`, `Exited`, `Dead`, or missing entirely.

### Check 3: Landing page renders (Modes A, B)

Using the browser automation tool:

1. Navigate to `https://PRODUCTION_URL/`
2. Wait for page load (network idle or equivalent)
3. Capture full-page screenshot → save as `screenshots/01-landing.png`
4. Listen for and record all console errors → save as `console/01-landing.log`
5. Verdict: Does the page show real content? Or blank / error / raw framework scaffolding?

### Check 4: Login page renders (Modes A, B)

1. Navigate to `https://PRODUCTION_URL/LOGIN_PATH`
2. Wait for load
3. Capture screenshot → `screenshots/02-login.png`
4. Verify: at least one text input field and one submit-type button are visible

### Check 5: Authentication flow (Modes A, B — only if credentials provided)

1. Enter `VERIFICATION_EMAIL` into the email/username field
2. Enter `VERIFICATION_PASSWORD` into the password field
3. Click the submit/login button
4. Wait up to 15 seconds for navigation to complete
5. Capture screenshot → `screenshots/03-post-login.png`
6. Capture console errors → `console/03-post-login.log`
7. Verdict: Did the page change to an authenticated state? Any redirect loop?

If credentials were not provided → mark as `BLOCKED — No credentials` and proceed.

### Check 6: Authenticated landing (Modes A, B — only if auth succeeded)

1. Navigate to `https://PRODUCTION_URL/DASHBOARD_PATH`
2. Capture screenshot → `screenshots/04-dashboard.png`
3. Verify: authenticated UI shell is visible (navigation, user context, content area)

### Check 7: API health (All modes)

```bash
curl -sS -w "\nStatus: %{http_code} | Time: %{time_total}s\n" "https://PRODUCTION_URL/HEALTH_ENDPOINT"
```

Expected: 200. Acceptable: 401 (auth required on health). Not acceptable: 500, 502, 503.

### Check 8: Worker status (Mode A only)

```bash
ssh SSH_COMMAND "docker ps --filter 'name=worker\|celery\|sidekiq\|bull' --format '{{.Names}}: {{.Status}}'"
```

If no worker containers exist for this project → mark as `N/A`. Not every project has workers.

---

## 7. EXTENDED BROWSER VERIFICATION (Modes A, B)

After the critical path, verify additional pages. Skip any that return 404.

### 7.1 Public pages

For each: navigate, capture screenshot, capture console errors.

| Page | Try these paths | Screenshot |
|---|---|---|
| Homepage (if different from landing) | `/`, `/home` | `screenshots/05-homepage.png` |
| Registration | `/signup`, `/register` | `screenshots/06-signup.png` |
| About | `/about`, `/about-us` | `screenshots/07-about.png` |
| Help / FAQ | `/help`, `/faq`, `/help-center` | `screenshots/08-help.png` |
| Contact | `/contact`, `/contact-us` | `screenshots/09-contact.png` |
| Pricing | `/pricing`, `/plans` | `screenshots/10-pricing.png` |

### 7.2 Authenticated pages (only if auth succeeded)

| Page | Try these paths | Screenshot |
|---|---|---|
| Primary list/browse | project-specific route from discovery | `screenshots/11-primary-list.png` |
| Item detail | click first item on list page | `screenshots/12-item-detail.png` |
| Search | `/search`, `/discover` | `screenshots/13-search.png` |
| Profile | `/profile`, `/account`, `/me` | `screenshots/14-profile.png` |
| Settings | `/settings`, `/preferences` | `screenshots/15-settings.png` |
| Notifications | `/notifications` | `screenshots/16-notifications.png` |

### 7.3 Per-page evidence record

For every page checked, record:

| Field | Value |
|---|---|
| URL | Actual URL visited |
| HTTP status | From navigation |
| Visual state | Renders content / blank / error / broken layout |
| Console errors | Count by severity |
| Translation keys visible | Yes / No / N/A |
| Screenshot path | File path |
| Result | PASSED / FAILED Tier X / PARTIAL / BLOCKED |

---

## 8. CONSOLE ERROR THRESHOLDS

Apply to every page visited via browser:

| Condition | Verdict |
|---|---|
| 0 errors | **PASS** |
| 1–3 non-critical warnings (deprecations, dev-mode) | **PASS with notes** |
| `ChunkLoadError`, missing JS module, or failed script load | **FAIL Tier 1** — broken build or stale assets |
| Repeated `401 Unauthorized` in console (loop pattern) | **FAIL Tier 1** — auth regression |
| `NetworkError`, `ERR_CONNECTION_REFUSED` to API domain | **FAIL Tier 1** — backend unreachable from frontend |
| `TypeError` or `ReferenceError` that prevents page render | **FAIL Tier 1** — JS runtime crash |
| Translation key strings visible in rendered page text | **FAIL Tier 2** — i18n broken (see Section 9) |
| React/Next.js hydration mismatch warning | **PASS with notes** — common in Next.js SSR, non-blocking |
| Zustand persist rehydration warning | **PASS with notes** — common with persist middleware |
| Tauri `__TAURI__` not found (in web-only mode) | **PASS with notes** — expected outside Tauri shell |
| 10+ errors of any kind on a single page | **FAIL Tier 2** — quality concern |

---

## 9. INTERNATIONALIZATION VERIFICATION

**Execute only if** `I18N` is `true` or `unknown` in the state file.

### 9.1 Key leakage check (Modes A, B)

On 2–3 critical pages (landing, login, dashboard), scan all visible text for patterns indicating raw translation keys are being shown to users:

- Dot-separated lowercase strings: `word.word.word` or `word.word`
- Literal `undefined` as visible UI text
- Strings containing `missing_key`, `key_not_found`, or `ns:`

If any raw key is visible in user-facing text → **FAIL Tier 2** (i18n-scoped).

If I18N was `unknown` and no keys are found and no locale UI is visible → set `I18N: false` and skip the remaining i18n checks.

### 9.2 Language switch (Modes A, B)

1. On a page with a language selector: switch to a non-default language
2. If no selector visible: try appending `?lng=es` or navigating to `/es/` prefix
3. Verify that visible text changes
4. Capture screenshot → `screenshots/17-i18n-language-switch.png`

### 9.3 RTL check (Modes A, B — only if Arabic or Hebrew is supported)

1. Switch to Arabic (`?lng=ar`) or Hebrew
2. Verify layout direction reverses
3. Capture screenshot → `screenshots/18-i18n-rtl.png`

### 9.4 Verdict

| Condition | Verdict |
|---|---|
| All pages show real text, no keys visible | PASS |
| 1–2 isolated key leaks | PARTIAL Tier 2 |
| Multiple pages show keys | FAIL Tier 2 (i18n-scoped) |
| Language switch does not change text | FAIL Tier 2 |
| All pages show keys (system-wide) | FAIL Tier 1 (application-scoped) |

---

## 10. API ENDPOINT VERIFICATION (All modes)

```bash
echo "=== API Verification ==="
for endpoint in HEALTH_ENDPOINT API_BASE; do
  curl -sS -w "Status: %{http_code} | Time: %{time_total}s | Size: %{size_download}B\n" \
    -o /dev/null "https://PRODUCTION_URL/${endpoint}" 2>/dev/null
done
```

Acceptable: 200 (healthy) or 401 (auth required). Not acceptable: 500, 502, 503, 000.

If credentials are available and the API accepts token/session auth, optionally test an authenticated endpoint:

```bash
# Attempt login via API to get token
TOKEN=$(curl -sS -X POST "https://PRODUCTION_URL/API_BASE/auth/login/" \
  -H "Content-Type: application/json" \
  -d '{"email":"VERIFICATION_EMAIL","password":"VERIFICATION_PASSWORD"}' 2>/dev/null \
  | grep -o '"token":"[^"]*"' | cut -d'"' -f4)

# Test authenticated endpoint
if [ -n "$TOKEN" ]; then
  curl -sS -w "Authenticated API: %{http_code} | %{time_total}s\n" \
    -H "Authorization: Bearer $TOKEN" \
    -o /dev/null "https://PRODUCTION_URL/API_BASE/some-protected-resource/" 2>/dev/null
fi
```

Adapt the auth mechanism (Bearer, Token, Cookie) to what the project uses. If unknown, skip authenticated API tests.

---

## 11. DATABASE PATH VALIDATION (Mode A only)

**All checks must be non-destructive. No writes, no schema changes, no direct queries.**

### 11.1 Container health

```bash
ssh SSH_COMMAND "docker ps --filter 'name=DB_CONTAINER' --format '{{.Names}}: {{.Status}}'"
```

### 11.2 Connection verification

Try each approach in order. Use the first that works:

```bash
# Approach 1: Django management command
ssh SSH_COMMAND "docker exec BACKEND_CONTAINER python manage.py check --database default 2>&1 | head -5"

# Approach 2: Rails database check
ssh SSH_COMMAND "docker exec BACKEND_CONTAINER rails db:version 2>&1 | head -3"

# Approach 3: Check for SQLite file existence (UFOP uses SQLite via rusqlite)
ssh SSH_COMMAND "docker exec BACKEND_CONTAINER ls -la /app/data/*.db 2>/dev/null || echo 'no-sqlite-files'"

# Approach 4: Generic — check DB container logs for connection errors
ssh SSH_COMMAND "docker logs --tail 30 DB_CONTAINER 2>&1 | grep -iE 'error|fatal|refused|timeout' | tail -5"

# Approach 5: Indirect — if data-backed pages render in browser, DB is reachable
```

### 11.3 Migration status (best-effort)

```bash
# Django
ssh SSH_COMMAND "docker exec BACKEND_CONTAINER python manage.py showmigrations --list 2>&1 | tail -15"

# UFOP Rust — check SQLite migration version via logs
ssh SSH_COMMAND "docker logs BACKEND_CONTAINER 2>&1 | grep -i 'migration' | tail -10"
```

If the backend is not Django and no migration info is available, skip this.

---

## 12. WORKER + CACHE VERIFICATION (Mode A only)

### 12.1 Workers

```bash
ssh SSH_COMMAND "docker ps --filter 'name=worker\|celery\|sidekiq\|bull' --format '{{.Names}}: {{.Status}}'"
```

If a worker container exists, check its recent logs:

```bash
ssh SSH_COMMAND "docker logs --tail 50 WORKER_CONTAINER 2>&1 | grep -ciE 'error|exception|traceback'"
ssh SSH_COMMAND "docker logs --tail 50 WORKER_CONTAINER 2>&1 | grep -ciE 'succeeded|completed|processed'"
```

If no worker containers exist → record as `N/A`.

### 12.2 Scheduler

```bash
ssh SSH_COMMAND "docker ps --filter 'name=beat\|scheduler\|cron' --format '{{.Names}}: {{.Status}}'"
```

If none exist → `N/A`.

### 12.3 Cache / broker

```bash
ssh SSH_COMMAND "docker ps --filter 'name=redis\|memcached' --format '{{.Names}}: {{.Status}}'"
```

If Redis exists:

```bash
ssh SSH_COMMAND "docker exec CACHE_CONTAINER redis-cli ping 2>/dev/null || echo 'ping-failed'"
```

---

## 13. PERFORMANCE SMOKE (All modes)

```bash
echo "=== Load Timing ==="
for route in "/" LOGIN_PATH DASHBOARD_PATH HEALTH_ENDPOINT; do
  time=$(curl -sS -o /dev/null -w "%{time_total}" "https://PRODUCTION_URL${route}" 2>/dev/null)
  echo "${route}: ${time}s"
done
```

| Time | Grade |
|---|---|
| < 3.0s | Good |
| 3.0–5.0s | Slow — note |
| 5.0–15.0s | Critical UX concern — Tier 2 |
| > 15.0s | Severe — Tier 1 |
| Timeout / no response | FAIL Tier 1 |

---

## 14. OBSERVABILITY REVIEW (Mode A only)

```bash
ssh SSH_COMMAND "echo 'Errors:' && docker logs --since '1h' BACKEND_CONTAINER 2>&1 | grep -c 'ERROR'"
ssh SSH_COMMAND "echo '500s:' && docker logs --since '1h' BACKEND_CONTAINER 2>&1 | grep -c ' 500 '"
ssh SSH_COMMAND "echo '401s:' && docker logs --since '1h' BACKEND_CONTAINER 2>&1 | grep -c '401'"
ssh SSH_COMMAND "echo 'DB issues:' && docker logs --since '1h' BACKEND_CONTAINER 2>&1 | grep -iE 'connection refused|database|psycopg|mysql|mongo|sqlite|rusqlite' | tail -5"
ssh SSH_COMMAND "echo 'Frontend errors:' && docker logs --since '1h' FRONTEND_CONTAINER 2>&1 | grep -iE 'error' | tail -10"
```

---

## 15. SHARED INFRASTRUCTURE (Mode A only)

```bash
ssh SSH_COMMAND "df -h / | tail -1"
ssh SSH_COMMAND "free -h 2>/dev/null || cat /proc/meminfo 2>/dev/null | head -3"
ssh SSH_COMMAND "docker ps --format '{{.Names}}: {{.Status}}' | head -20"
```

Check: disk not >90% full, memory not exhausted, no unrelated containers in crash state.

---

## 16. BLOCKER REGISTRY — MANDATORY

Maintain throughout the run. Every issue gets an entry, even if later resolved.

### Entry format

| Field | Description |
|---|---|
| **ID** | BLK-001, BLK-002, etc. |
| **Timestamp** | ISO 8601 |
| **Scope** | Journey / Service / Application / I18N / Shared-infra / Control-plane |
| **Component** | Specific page, service, or system |
| **User impact** | What does a real user see or experience? |
| **Evidence** | Screenshot path, log excerpt, HTTP status, error message |
| **Root cause** | Best assessment |
| **Pre-existing?** | Existed before this release? Yes / No / Unknown |
| **Rollback fixes?** | Would reverting fix this? Yes / No / Unknown |
| **Human review needed?** | Yes / No |

### Scope definitions

| Scope | Meaning |
|---|---|
| Journey-scoped | One specific user flow broken |
| Service-scoped | One container or service broken |
| Application-scoped | Most or all of the app affected |
| I18N-scoped | Translation system failure |
| Shared-infra-scoped | Networking, TLS, proxy, shared DB host |
| Control-plane-scoped | Dokploy itself unreachable or broken |

### Systemic failure escalation

If **3 or more checks fail with the same pattern** (e.g., all API calls return 500, all pages show JS crash, all authenticated pages redirect to login):

1. Stop treating as isolated failures
2. Document the systemic pattern
3. Identify probable shared root cause
4. Continue only safe evidence collection
5. Recommend rollback or immediate human review

---

## 17. STATUS MODEL

Every checked item ends as one of:

| Status | Meaning |
|---|---|
| **PASSED** | Working correctly. Evidence captured. |
| **FAILED — Tier 1** | Serious live issue. Likely release blocker. |
| **FAILED — Tier 2** | User-visible defect. Survivable but needs fixing. |
| **FAILED — Tier 3** | Ambiguous or security-sensitive. Needs human judgment. |
| **PARTIAL** | Partially working. Some aspects verified, others not. |
| **BLOCKED** | Cannot test. Reason documented. |
| **N/A** | Not applicable to this project (e.g., no workers, no i18n). |

**"Skipped" is never valid.** Use BLOCKED with a reason or N/A.

### Continuation rule

When a check fails:

1. Capture evidence (screenshot, log, HTTP status)
2. Classify severity and scope
3. Add entry to blocker registry
4. Determine whether remaining checks are still safe
5. **Continue all safe independent checks**
6. Mark dependent checks as PARTIAL or BLOCKED
7. Never stop early unless remaining work would cause production harm

---

## 18. FINAL REPORT — MANDATORY OUTPUT

Write the report using the available file-write tool.

**Primary location:** A clearly named file in the agent's workspace, e.g., `production-verification-report.md`

### Report structure

```markdown
# Production E2E Verification Report

## Run Metadata
- **Run ID:** <YYYYMMDD-HHMMSS>
- **Timestamp:** <ISO 8601>
- **Environment:** production
- **Platform:** Dokploy
- **Operating Mode:** <A / B / C>
- **Project:** <name>
- **Production URL:** <domain>
- **Release:** <image tags per service, or "unknown — no SSH access">
- **Duration:** <total minutes>

## Detected Stack
| Component | Detected | Container |
|---|---|---|
| Frontend | <tech or unknown> | <name or N/A> |
| Backend | <tech or unknown> | <name or N/A> |
| Database | <tech or unknown> | <name or N/A> |
| Cache | <tech or none> | <name or N/A> |
| Worker | <tech or none> | <name or N/A> |
| I18N | <active / not detected> | — |

## Rollback Candidate
- Previous image: <tag/digest or unknown>
- Complexity: <simple / coordinated / migration-blocked / unknown>

## Credentials
- Provided: <Yes / No>
- Auth tested: <Yes / No / Blocked>

---

## Critical Path Results
| # | Check | Modes | Result | Severity | Evidence |
|---|---|---|---|---|---|
| 1 | Domain + TLS | All | <result> | — | <detail> |
| 2 | Container health | A | <result> | — | <detail> |
| 3 | Landing page | A,B | <result> | — | <screenshot> |
| 4 | Login page | A,B | <result> | — | <screenshot> |
| 5 | Auth flow | A,B | <result> | — | <screenshot> |
| 6 | Dashboard | A,B | <result> | — | <screenshot> |
| 7 | API health | All | <result> | — | <detail> |
| 8 | Worker status | A | <result> | — | <detail> |

## Browser Verification
| Page | URL | Result | Screenshot | Console Errors |
|---|---|---|---|---|
| <name> | <url> | <result> | <file> | <count> |

## API Verification
| Endpoint | Status Code | Timing | Result |
|---|---|---|---|
| <path> | <code> | <seconds> | <result> |

## Auth Summary
- Login page renders: <Yes / No / Blocked>
- Credentials accepted: <Yes / No / N/A>
- Authenticated pages accessible: <Yes / No / Blocked>

## I18N Summary
- Applicable: <Yes / No>
- Translation keys visible: <Yes / No / N/A>
- Language switch works: <Yes / No / N/A>
- RTL tested: <Yes / No / N/A>

## Asset Summary
- CSS loads: <Yes / No>
- JS bundles load: <Yes / No>
- Icons/images load: <Yes / No>
- Console errors total: <count>

## Worker / Cache Summary
| Service | Exists | Status | Log Errors | Notes |
|---|---|---|---|---|
| <name> | <Yes/No> | <status> | <count> | <notes> |

## DB Path Summary
- Container: <status or N/A>
- Connectivity: <evidence>
- Data-backed pages: <render / broken / not tested>

## Performance
| Route | Time | Grade |
|---|---|---|
| <path> | <seconds> | <Good/Slow/Critical/Severe> |

## Observability (Mode A only)
- Backend errors (1h): <count or N/A>
- 500s (1h): <count or N/A>
- Auth failures (1h): <count or N/A>
- Notable patterns: <description or none>

## Shared Infrastructure
- Disk usage: <percent or N/A>
- Memory: <status or N/A>
- Neighbor health: <status or N/A>

---

## Blocker Registry
| ID | Scope | Component | User Impact | Tier | Rollback Fix? | Human Review? |
|---|---|---|---|---|---|---|
| BLK-001 | <scope> | <component> | <impact> | <tier> | <Y/N> | <Y/N> |

## Systemic Failures
<Description or "None detected">

## Unresolved Risks
<List or "None">

## Unverified Items
| Item | Reason |
|---|---|
| <item> | <reason> |

---

## FINAL DECISION

### Verdict

**<Exactly one of:>**

- **PRODUCTION VERIFIED — SAFE TO REMAIN LIVE**
- **PRODUCTION LIVE WITH NON-BLOCKING DEFECTS**
- **PRODUCTION PARTIALLY VERIFIED / HEIGHTENED MONITORING REQUIRED**
- **PRODUCTION NOT SAFE — ROLLBACK OR IMMEDIATE HUMAN REVIEW ADVISED**

### Justification
<2–4 sentences explaining the verdict based on evidence>

### Next Action
<Exact next operational step>
```

---

## 19. FINAL DECISION GATE

Before declaring a verdict, confirm every applicable item:

- [ ] Domain reachable with valid TLS
- [ ] Critical pages checked with evidence (screenshots or HTTP status)
- [ ] Auth tested if credentials were provided
- [ ] Console error thresholds applied to every browsed page
- [ ] I18N verified if applicable — no raw keys visible
- [ ] No unresolved Tier 1 issues remaining
- [ ] Blocker registry is complete
- [ ] Rollback candidate identified (Mode A) or noted as unknown
- [ ] Report written and saved
- [ ] Verdict explicitly declared with justification

If any mandatory item fails → do not declare `VERIFIED — SAFE`. Use a lower verdict.

---

## 20. ROLLBACK GUIDANCE — FOR HUMAN OPERATOR ONLY

The verification agent documents rollback readiness but **does not execute rollback**.

### When to recommend rollback

- 2+ Tier 1 blockers with application-wide scope
- Authentication completely broken for all users
- All API endpoints returning 500
- Frontend fails to render entirely
- Database connection lost
- Systemic pattern failure detected

### Rollback steps (for human)

1. Identify the previous known-good image tag from the discovery data in the report
2. In Dokploy: navigate to the application, select the previous deployment, trigger redeploy
3. If database migrations ran during the failed release: verify backward compatibility before rolling back. If migrations are not backward-compatible, coordinate DB rollback first.
4. After rollback completes: re-run this runbook (Sections 5–10) against the rolled-back version to confirm recovery

---

## 21. EXECUTION PRINCIPLES

These override convenience in all cases:

| Principle | Overrides |
|---|---|
| **Production-safe** | Any desire to "fix it quick" |
| **Browser proof** | Log-only optimism |
| **Target-only** | Curiosity about unrelated services |
| **Evidence-first** | Assumptions about what "should" work |
| **Continue under failure** | Temptation to stop early |
| **Rollback awareness** | False confidence that everything is fine |
| **Auto-discover** | Hardcoded project assumptions |
| **Time-boxed (48 min)** | Perfectionism — deliver what you have |
| **Mode-aware** | Attempting checks impossible in current mode |
| **Substitute before execute** | Running commands with raw placeholder tokens |

---

## 22. AGENT BEHAVIOR SUMMARY

You are a **production reliability engineer + skeptical QA verifier + cautious platform operator**.

Execution sequence:

1. Resolve parameters silently from context (Section 1)
2. Select operating mode (Section 2)
3. Discover topology, stack, and routes (Section 4)
4. Run fast-fail critical path in order (Section 6)
5. Run extended browser verification (Section 7)
6. Apply console error thresholds (Section 8)
7. Check i18n if applicable (Section 9)
8. Verify API endpoints (Section 10)
9. Validate DB path, workers, cache (Sections 11–12)
10. Measure performance (Section 13)
11. Review observability and shared infra (Sections 14–15)
12. Maintain blocker registry throughout (Section 16)
13. Write the report (Section 18)
14. Apply decision gate (Section 19)
15. Declare verdict with justification and next action

A failure in one area is **never** permission to abandon the rest.

**Execute fully. Report completely. Decide clearly.**

---

*End of Universal Production E2E Verification Runbook v3.0 — Customized for UFOP*
