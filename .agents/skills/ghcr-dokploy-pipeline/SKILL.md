---
name: ghcr-dokploy-pipeline
description: Use when setting up, implementing, or troubleshooting a GHCR + Dokploy + VPS deploy pipeline. Triggers on "setup deploy pipeline", "build and push to GHCR", "configure GHCR deploy", "deploy pipeline setup", "docker build push", "ghcr pipeline", "VPS deploy setup", "create deploy scripts", "fix deploy", "502 after deploy", "nginx upstream", "rollback deploy", or any request to create/fix the local-build → GHCR → VPS deployment infrastructure. Platform is Dokploy (NOT Coolify). Covers file templates, first-time setup, routine deploy, rollback, troubleshooting, and security. Tech stack context — UFOP project with Tauri 2.0 (Rust + React/Vite), Next.js admin console, pnpm workspaces, SQLite/rusqlite; also supports Django, Next.js, PostgreSQL, Redis, Celery stacks.
---

# Generic GHCR + Dokploy + VPS Deploy Pipeline — Project-Agnostic Template

> **Audience:** AI agents or operators implementing a production deploy
> pipeline for **any** dockerized project where the constraints are:
>
> - A VPS (Contabo, Hetzner, DigitalOcean, etc.) running Docker
> - GHCR (GitHub Container Registry) as the image registry
> - Dokploy installed on the VPS for SSL/Traefik routing only
> - **No GitHub Actions** (rate limits / secret management / cost)
> - Fully automated deploy via one local command
> - Local CI (pre-push checks) instead of remote CI
>
> **Status:** Template. Substitute `${PLACEHOLDER}` values with your
> project's actual values. Battle-tested on a Python/Django + Next.js
> multi-service project (~26 containers, 14 async services).

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

> **UFOP Adaptation Notes:** For UFOP, the "backend" image may be a Rust binary (compiled with `cargo build --release`) rather than Django/Python. The "frontend" image is the Next.js admin console. The Tauri desktop app itself is distributed as a native binary, not deployed to a VPS. Adapt the `${IMAGES[@]}` and Dockerfile references accordingly. The pnpm workspace structure means the admin console build may require workspace-level context for `@ufop/design-tokens` and `@ufop/ui-components` packages.

---

## Table of Contents

1. [When This Template Applies](#1-when-this-template-applies)
2. [The Architecture](#2-the-architecture)
3. [Placeholders You Must Substitute](#3-placeholders-you-must-substitute)
4. [Prerequisites](#4-prerequisites)
5. [File Templates](#5-file-templates)
6. [First-Time Setup (Fresh Repo + Fresh VPS)](#6-first-time-setup-fresh-repo--fresh-vps)
7. [Routine Deploy](#7-routine-deploy)
8. [Rollback Procedures](#8-rollback-procedures)
9. [Verification Checklist](#9-verification-checklist)
10. [Universal Gotchas & Lessons Learned](#10-universal-gotchas--lessons-learned)
11. [Troubleshooting Matrix](#11-troubleshooting-matrix)
12. [Security Notes](#12-security-notes)
13. [AI Agent Implementation Checklist](#13-ai-agent-implementation-checklist)
14. [Extending the Template](#14-extending-the-template)

---

## 1. When This Template Applies

Use this template when **all** of the following are true:

- You have a **GitHub repository** (public or private).
- You have a **single production VPS** (not a Kubernetes cluster).
- You're willing to build Docker images **on the operator's local
  machine** (not in remote CI).
- You want **immutable artifacts** (SHA-tagged images in GHCR) so
  rollback is `change the tag → up -d`.
- You want **a single command** (`./scripts/deploy-prod.sh`) to ship
  new code end-to-end.
- You want **SSL + edge routing** via Dokploy's embedded Traefik so you
  don't have to operate Let's Encrypt yourself.
- You're **not** using GitHub Actions (rate limits on private repos,
  build time, secret management complexity).

If you have a Kubernetes cluster, use a GitOps tool (ArgoCD/FluxCD).
If you want zero-downtime blue/green, use Traefik weighted services or
a swarm. If you want multi-region, this template is too simple.

For everything else — a single-operator or small-team project with one
VPS — this template is the sweet spot.

---

## 2. The Architecture

```
┌──────────────────────────────────────────────────────────────────────┐
│                           OPERATOR LOCAL HOST                       │
│                                                                      │
│  git commit                                                          │
│     │                                                                │
│     ▼                                                                │
│  ./scripts/deploy-prod.sh                                            │
│     │                                                                │
│     ├─ 1. scripts/pre-push-checks.sh      ← local "CI"               │
│     │     ├─ lint                                                    │
│     │     ├─ unit tests                                              │
│     │     ├─ typecheck                                               │
│     │     └─ build validation                                        │
│     │                                                                │
│     ├─ 2. scripts/build-push-ghcr.sh production                      │
│     │     ├─ gh auth token | docker login ghcr.io                    │
│     │     ├─ docker buildx create <project>-builder                  │
│     │     ├─ buildx build --platform linux/amd64 --push              │
│     │     │     for each image:                                      │
│     │     │       ghcr.io/<ns>/<project>-<svc>:sha-<short>           │
│     │     │       ghcr.io/<ns>/<project>-<svc>:production            │
│     │     └─ (Rosetta or QEMU for cross-compile if on arm64)         │
│     │                                                                │
│     ├─ 3. rsync static assets to <vps>:${REMOTE_DIR}/                │
│     │     ├─ docker-compose.ghcr.yml                                 │
│     │     ├─ nginx/nginx.conf      (if you use nginx internally)     │
│     │     ├─ db/init.sql           (if you have DB init)             │
│     │     └─ scripts/backup.sh     (if you have DB backup cron)      │
│     │                                                                │
│     ├─ 4. ssh <vps>: docker login ghcr.io                            │
│     │     (passes gh auth token over SSH)                            │
│     │                                                                │
│     ├─ 5. ssh <vps>: docker compose -f docker-compose.ghcr.yml pull  │
│     │                                                                │
│     ├─ 6. ssh <vps>: docker compose -f docker-compose.ghcr.yml       │
│     │                   up -d --remove-orphans                       │
│     │     └─ recreates only containers whose image/config changed    │
│     │                                                                │
│     ├─ 7. ssh <vps>: docker exec nginx nginx -t && nginx -s reload   │
│     │                                                                │
│     └─ 8. curl health verification                                   │
└──────────────────────────────────────────────────────────────────────┘

       │                                         ▲
       │ push                                    │ pull (auth'd)
       ▼                                         │
┌────────────────────┐                  ┌────────────────────┐
│    ghcr.io         │                  │    VPS             │
│  <namespace>/      │◀────────────────▶│  ${REMOTE_DIR}/    │
│  <project>-*       │                  │                    │
│                    │                  │  docker compose    │
│  sha-XXX tags      │                  │  -f ghcr.yml up -d │
│  + :production     │                  │                    │
│                    │                  │  + Dokploy         │
│  Private or public │                  │  + Traefik (SSL)   │
└────────────────────┘                  │  + your app stack  │
                                        └────────────────────┘
                                                 │
                                                 ▼
                                        ┌────────────────────┐
                                        │   your domain      │
                                        │   (Traefik + LE)   │
                                        └────────────────────┘
```

### Why this shape

| Decision | Rationale |
|---|---|
| **Build locally, push to GHCR** | No GH Actions rate limits. Operator's own CPU, which is typically stronger than a free runner. Secrets stay local. |
| **Immutable SHA tags** | Rollback is `IMAGE_TAG=sha-<prev> up -d`. No rebuild, no regression risk from a forced rebuild. |
| **Dokploy for Traefik only** | Dokploy is a convenient SSL/routing layer. We don't use its git-auto-deploy (unreliable in practice; see §10.5). |
| **docker-compose.ghcr.yml separate from docker-compose.prod.yml** | Clean split: `prod.yml` builds from source (used for local dev / CI); `ghcr.yml` pulls from registry (used for production runtime). |
| **Named volumes keyed to compose project name** | Upgrading the image tag never loses data, as long as the compose project name stays the same. |
| **gh CLI for auth instead of PAT** | One less secret to manage. `gh auth login` once and you're set for all deploys. |

### UFOP-Specific Architecture Notes

For the UFOP project, the architecture adapts as follows:

- **The Tauri desktop app** is not deployed to a VPS — it's distributed as a native binary via `pnpm tauri:build`
- **The Next.js admin console** (`admin/`) is the primary service deployed to the VPS
- **Rust backend services** (if containerized) use `cargo build --release` in a multi-stage Dockerfile
- **pnpm workspace** builds require the workspace root context for shared packages (`@ufop/design-tokens`, `@ufop/ui-components`)
- **SQLite** data files should be in a named Docker volume to persist across container recreations

---

## 3. Placeholders You Must Substitute

Replace these everywhere in the files below. The template uses
`${PLACEHOLDER}` syntax for clarity; your actual substitution may need
quoting depending on the file type.

| Placeholder | Meaning | Example |
|---|---|---|
| `${PROJECT_NAME}` | Short identifier used for image names, builder name, compose project name. Lowercase, no spaces. | `ufop`, `claptrading`, `myblog` |
| `${GH_NAMESPACE}` | GitHub user or org (lowercase) that owns the GHCR packages | `clappediya`, `myorg` |
| `${DOMAIN}` | Public domain served by Traefik | `admin.ufop.app`, `app.example.com` |
| `${VPS_IP}` | Production VPS public IP | `85.239.231.77` |
| `${VPS_USER}` | SSH user on the VPS (usually `root`) | `root`, `deploy` |
| `${REMOTE_DIR}` | Path on the VPS where the compose file lives | `/opt/${PROJECT_NAME}` |
| `${IMAGES[@]}` | Array of image names your project builds. Can be 1 (`admin`) or many (`backend, frontend, services`) | `(backend frontend services)` |
| `${DB_VOLUME_NAME}` | Named volume for your primary database | `postgres_data`, `sqlite_data` |
| `${BUILD_PLATFORM}` | Target platform for buildx | `linux/amd64` (most VPS) |
| `${ENV_FILE_PATH}` | Path on VPS to the `.env` file with runtime secrets | `${REMOTE_DIR}/.env` |

The rest of the template assumes a simple 1-3 image setup, but scales
to any N by repeating the per-image blocks.

---

## 4. Prerequisites

### 4.1 Local (operator's machine)

```bash
# GitHub CLI with write:packages scope
brew install gh                      # macOS
# or: https://cli.github.com/ for other platforms
gh auth login --scopes write:packages
gh auth status                       # must show write:packages

# Docker Desktop (with buildx bundled)
# M-series Mac: Rosetta will be used automatically for amd64 cross-compile
# Linux amd64 host: native builds are fastest

# sshpass for script-based password SSH (optional if you use SSH keys)
brew install sshpass                 # macOS
# Debian/Ubuntu: apt install sshpass

# rsync (usually preinstalled)

# UFOP-specific: pnpm (for workspace builds)
# pnpm 10.27.0 is the project standard
npm install -g pnpm@10.27.0

# UFOP-specific: Rust toolchain (for Tauri/Rust backend builds)
# Only needed if building Rust backend images locally
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### 4.2 VPS

```bash
# Docker (the install script from get.docker.com is fine)
curl -fsSL https://get.docker.com | sh

# Dokploy (provides Traefik + SSL auto-provisioning)
curl -sSL https://dokploy.com/install.sh | sh

# Confirm the Dokploy network exists
docker network inspect dokploy-network || \
  docker network create dokploy-network

# Create the project directory
mkdir -p ${REMOTE_DIR}/{nginx,scripts,backups}
```

### 4.3 Git repository

The repo must contain:

```
.
├── docker-compose.prod.yml          # builds from source (for dev/CI)
├── docker-compose.ghcr.yml          # pulls from GHCR (for production)
├── Dockerfile                       # or one per service
├── nginx/nginx.conf                 # if you route internally via nginx
├── scripts/
│   ├── pre-push-checks.sh           # local CI
│   ├── build-push-ghcr.sh           # build + push to GHCR
│   └── deploy-prod.sh               # one-shot deploy
├── .env.example                     # committed — shows required env vars
└── .env.production                  # NEVER committed — gitignored
```

**UFOP workspace structure adaptation:**
```
.
├── admin/                           # Next.js admin console (deploy target)
│   ├── Dockerfile                   # Admin console Dockerfile
│   └── ...
├── src-tauri/                       # Rust backend (Tauri — desktop, not VPS)
├── packages/
│   ├── design-tokens/               # @ufop/design-tokens (needed at build)
│   └── ui-components/               # @ufop/ui-components (needed at build)
├── docker-compose.ghcr.yml          # GHCR production compose
├── pnpm-workspace.yaml              # pnpm workspace config
├── scripts/
│   ├── pre-push-checks.sh           # pnpm lint + pnpm test + cargo test
│   ├── build-push-ghcr.sh           # build admin image + push to GHCR
│   └── deploy-prod.sh               # one-shot deploy
└── .env.production                  # NEVER committed
```

### 4.4 DNS

Point `${DOMAIN}` (and `www.${DOMAIN}` if desired) at `${VPS_IP}` via an
A record. Traefik will auto-provision a Let's Encrypt cert on first
request.

---

## 5. File Templates

### 5.1 `docker-compose.ghcr.yml`

This is the canonical production compose file. It mirrors
`docker-compose.prod.yml` but replaces every `build:` with `image:` so
the VPS pulls from GHCR.

```yaml
# ============================================================================
# PRODUCTION COMPOSE — ${PROJECT_NAME} (GHCR pull-only)
# ============================================================================
# Usage on the VPS:
#   cd ${REMOTE_DIR}
#   IMAGE_TAG=sha-abcd123 docker compose -f docker-compose.ghcr.yml pull
#   IMAGE_TAG=sha-abcd123 docker compose -f docker-compose.ghcr.yml up -d
# ============================================================================

# Shared env block (DRY for services sharing config)
x-app-env: &app-env
  TZ: ${TZ:-UTC}
  NODE_ENV: production                     # or DJANGO_SETTINGS_MODULE, etc.
  DATABASE_URL: ${DATABASE_URL:?required}
  REDIS_URL: ${REDIS_URL:-redis://redis:6379/0}
  SECRET_KEY: ${SECRET_KEY:?required}
  # … add your project's env vars

services:
  # -------- Edge / routing --------
  nginx:
    image: nginx:1.27-alpine
    expose:
      - "80"
    volumes:
      - ./nginx/nginx.conf:/etc/nginx/nginx.conf:ro
    depends_on:
      backend:
        condition: service_healthy
    restart: unless-stopped
    labels:
      # Traefik (provided by Dokploy) SSL routing
      - "traefik.enable=true"
      - "traefik.http.routers.${PROJECT_NAME}-http.rule=Host(`${DOMAIN}`) || Host(`www.${DOMAIN}`)"
      - "traefik.http.routers.${PROJECT_NAME}-http.entrypoints=web"
      - "traefik.http.routers.${PROJECT_NAME}-http.middlewares=redirect-to-https"
      - "traefik.http.middlewares.redirect-to-https.redirectscheme.scheme=https"
      - "traefik.http.middlewares.redirect-to-https.redirectscheme.permanent=true"
      - "traefik.http.routers.${PROJECT_NAME}.rule=Host(`${DOMAIN}`) || Host(`www.${DOMAIN}`)"
      - "traefik.http.routers.${PROJECT_NAME}.entrypoints=websecure"
      - "traefik.http.routers.${PROJECT_NAME}.tls=true"
      - "traefik.http.routers.${PROJECT_NAME}.tls.certresolver=letsencrypt"
      - "traefik.http.services.${PROJECT_NAME}.loadbalancer.server.port=80"
    networks:
      - app-net
      - dokploy-network

  # -------- Stateful services (unchanged across deploys) --------
  postgres:
    image: postgres:17-alpine
    environment:
      POSTGRES_DB: ${POSTGRES_DB:?required}
      POSTGRES_USER: ${POSTGRES_USER:?required}
      POSTGRES_PASSWORD: ${POSTGRES_PASSWORD:?required}
    volumes:
      - ${DB_VOLUME_NAME}:/var/lib/postgresql/data
      # Optional: init SQL
      - ./db/init.sql:/docker-entrypoint-initdb.d/01-init.sql:ro
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U ${POSTGRES_USER} -d ${POSTGRES_DB}"]
      interval: 10s
      timeout: 5s
      retries: 5
    restart: unless-stopped
    networks:
      - app-net

  redis:
    image: redis:7-alpine
    command: redis-server --appendonly yes
    volumes:
      - redis_data:/data
    healthcheck:
      test: ["CMD", "redis-cli", "ping"]
      interval: 10s
      timeout: 5s
      retries: 5
    restart: unless-stopped
    networks:
      - app-net

  # -------- Application images (pulled from GHCR) --------
  backend:
    image: ghcr.io/${GH_NAMESPACE}/${PROJECT_NAME}-backend:${IMAGE_TAG:-production}
    environment:
      <<: *app-env
    depends_on:
      postgres:
        condition: service_healthy
      redis:
        condition: service_healthy
    healthcheck:
      # IMPORTANT: use the liveness probe, not the full health check.
      # Full /health/ often depends on downstream services that haven't
      # started yet at first boot — deadlock.
      # Use python urllib (or wget-busybox) so you don't depend on curl
      # being in the base image.
      test:
        - "CMD-SHELL"
        - |
          python -c 'import urllib.request,sys; sys.exit(0 if
          urllib.request.urlopen("http://localhost:8000/health/live/"
          ).status==200 else 1)'
      interval: 30s
      timeout: 10s
      retries: 3
      start_period: 60s
    restart: unless-stopped
    networks:
      - app-net

  frontend:
    image: ghcr.io/${GH_NAMESPACE}/${PROJECT_NAME}-frontend:${IMAGE_TAG:-production}
    expose:
      - "3000"
    depends_on:
      backend:
        condition: service_healthy
    restart: unless-stopped
    networks:
      - app-net

  # Repeat for any additional service images (workers, schedulers, etc.)

volumes:
  ${DB_VOLUME_NAME}:
  redis_data:

networks:
  app-net:
    driver: bridge
  dokploy-network:
    external: true
```

**UFOP Admin Console variant (Next.js only, no Django/PostgreSQL/Redis):**

```yaml
# For UFOP: simpler compose — just the admin console
services:
  admin:
    image: ghcr.io/${GH_NAMESPACE}/${PROJECT_NAME}-admin:${IMAGE_TAG:-production}
    expose:
      - "3001"
    environment:
      NODE_ENV: production
      # UFOP admin console env vars
    healthcheck:
      test: ["CMD-SHELL", "wget -qO- http://localhost:3001/api/health || exit 1"]
      interval: 30s
      timeout: 10s
      retries: 3
      start_period: 30s
    restart: unless-stopped
    labels:
      - "traefik.enable=true"
      # ... Traefik labels for Dokploy routing
    networks:
      - app-net
      - dokploy-network

networks:
  app-net:
    driver: bridge
  dokploy-network:
    external: true
```

### 5.2 `nginx/nginx.conf` — CRITICAL: resolver + variable proxy_pass

```nginx
worker_processes auto;
error_log /var/log/nginx/error.log warn;
pid /var/run/nginx.pid;

events {
    worker_connections 1024;
}

http {
    include /etc/nginx/mime.types;
    default_type application/octet-stream;

    log_format main '$remote_addr - $remote_user [$time_local] "$request" '
                    '$status $body_bytes_sent "$http_referer" '
                    '"$http_user_agent"';
    access_log /var/log/nginx/access.log main;

    sendfile on;
    tcp_nopush on;
    keepalive_timeout 65;
    client_max_body_size 20M;

    gzip on;
    gzip_types text/plain text/css application/json application/javascript;

    # =========================================================
    # CRITICAL: Docker embedded DNS + variable-based proxy_pass
    # =========================================================
    # Without these two things, nginx caches the upstream IP once at
    # startup. When the upstream container is recreated (new IP), nginx
    # keeps trying the old one → silent 502. This is the single most
    # common Docker + nginx gotcha. Do NOT use `upstream { server X; }`
    # blocks.
    resolver 127.0.0.11 valid=10s ipv6=off;

    # Trust X-Forwarded-Proto from Traefik
    map $http_x_forwarded_proto $forwarded_proto {
        default $http_x_forwarded_proto;
        ""      $scheme;
    }

    server {
        listen 80;
        server_name _;

        # Security headers
        add_header X-Frame-Options "DENY" always;
        add_header X-Content-Type-Options "nosniff" always;
        add_header Referrer-Policy "strict-origin-when-cross-origin" always;

        location /api/ {
            set $upstream_backend backend:8000;
            proxy_pass http://$upstream_backend;
            proxy_set_header Host $host;
            proxy_set_header X-Real-IP $remote_addr;
            proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
            proxy_set_header X-Forwarded-Proto $forwarded_proto;
            proxy_read_timeout 300s;
        }

        location /health/ {
            set $upstream_backend backend:8000;
            proxy_pass http://$upstream_backend;
            proxy_set_header Host $host;
        }

        # WebSocket
        location /ws {
            set $upstream_backend backend:8000;
            proxy_pass http://$upstream_backend;
            proxy_http_version 1.1;
            proxy_set_header Upgrade $http_upgrade;
            proxy_set_header Connection "upgrade";
            proxy_set_header Host $host;
            proxy_read_timeout 86400s;
        }

        # Everything else → frontend
        location / {
            set $upstream_frontend frontend:3000;
            proxy_pass http://$upstream_frontend;
            proxy_set_header Host $host;
            proxy_set_header X-Real-IP $remote_addr;
            proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
            proxy_set_header X-Forwarded-Proto $forwarded_proto;
        }
    }
}
```

Validate syntax locally before deploying:

```bash
docker run --rm -v "$PWD/nginx/nginx.conf":/etc/nginx/nginx.conf:ro \
  nginx:1.27-alpine nginx -t
```

Expected: `configuration file /etc/nginx/nginx.conf test is successful`.

### 5.3 `scripts/pre-push-checks.sh` — local CI

```bash
#!/usr/bin/env bash
# Pre-push hook: runs lint, tests, typecheck, build before allowing
# a push or deploy. Install with:
#   ln -sf ../../scripts/pre-push-checks.sh .git/hooks/pre-push
set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
NC='\033[0m'

echo -e "${YELLOW}━━━ Pre-push checks ━━━${NC}"

FAILED=0
ROOT="$(git rev-parse --show-toplevel)"

# --- Backend (example: Python) --------------------------------------
if [ -d "$ROOT/backend" ]; then
    echo -e "\n${YELLOW}[1/4] Backend lint${NC}"
    (cd "$ROOT/backend" && source .venv/bin/activate 2>/dev/null \
     && ruff check . --quiet) \
        && echo -e "${GREEN}  ✓ Lint passed${NC}" \
        || { echo -e "${RED}  ✗ Lint failed${NC}"; FAILED=1; }

    echo -e "\n${YELLOW}[2/4] Backend tests${NC}"
    (cd "$ROOT/backend" && source .venv/bin/activate 2>/dev/null \
     && python -m pytest --tb=short -q 2>&1 | tail -5) \
        && echo -e "${GREEN}  ✓ Tests passed${NC}" \
        || { echo -e "${RED}  ✗ Tests failed${NC}"; FAILED=1; }
fi

# --- Frontend (example: Next.js) ------------------------------------
if [ -d "$ROOT/frontend" ]; then
    echo -e "\n${YELLOW}[3/4] Frontend typecheck${NC}"
    (cd "$ROOT/frontend" && npm run typecheck --silent) \
        && echo -e "${GREEN}  ✓ Typecheck passed${NC}" \
        || { echo -e "${RED}  ✗ Typecheck failed${NC}"; FAILED=1; }

    echo -e "\n${YELLOW}[4/4] Frontend build${NC}"
    (cd "$ROOT/frontend" && npm run build --silent) \
        && echo -e "${GREEN}  ✓ Build passed${NC}" \
        || { echo -e "${RED}  ✗ Build failed${NC}"; FAILED=1; }
fi

# --- UFOP-specific checks (Rust backend + pnpm workspace) -----------
if [ -d "$ROOT/src-tauri" ]; then
    echo -e "\n${YELLOW}[Rust] cargo check + cargo test${NC}"
    (cd "$ROOT/src-tauri" && cargo check 2>&1 | tail -3) \
        && echo -e "${GREEN}  ✓ Cargo check passed${NC}" \
        || { echo -e "${RED}  ✗ Cargo check failed${NC}"; FAILED=1; }

    (cd "$ROOT/src-tauri" && cargo test --lib 2>&1 | tail -5) \
        && echo -e "${GREEN}  ✓ Cargo tests passed${NC}" \
        || { echo -e "${RED}  ✗ Cargo tests failed${NC}"; FAILED=1; }
fi

if [ -f "$ROOT/pnpm-workspace.yaml" ]; then
    echo -e "\n${YELLOW}[pnpm] Workspace lint + typecheck${NC}"
    (cd "$ROOT" && pnpm lint --silent 2>&1 | tail -3) \
        && echo -e "${GREEN}  ✓ pnpm lint passed${NC}" \
        || { echo -e "${RED}  ✗ pnpm lint failed${NC}"; FAILED=1; }

    (cd "$ROOT" && pnpm test --silent 2>&1 | tail -5) \
        && echo -e "${GREEN}  ✓ pnpm test passed${NC}" \
        || { echo -e "${RED}  ✗ pnpm test failed${NC}"; FAILED=1; }
fi

if [ $FAILED -ne 0 ]; then
    echo -e "\n${RED}━━━ Pre-push checks FAILED ━━━${NC}"
    exit 1
fi

echo -e "\n${GREEN}━━━ All pre-push checks passed ━━━${NC}"
```

Adjust for your stack: Go `go test ./...`, Rust `cargo test`,
Node `npm test`, Ruby `bundle exec rspec`, etc.

### 5.4 `scripts/build-push-ghcr.sh`

```bash
#!/usr/bin/env bash
# =============================================================================
# Build ${PROJECT_NAME} images locally and push to GHCR
# Usage: ./scripts/build-push-ghcr.sh [staging|production]
#
# Auth: uses `gh auth token` from the GitHub CLI so you don't manage a
# separate PAT. Run once: gh auth login --scopes write:packages
# =============================================================================
set -euo pipefail

ENV_TAG="${1:-production}"
REGISTRY="${GHCR_REGISTRY:-ghcr.io/${GH_NAMESPACE}/${PROJECT_NAME}}"
VERSION="$(git rev-parse --short HEAD)"
FULL_SHA="$(git rev-parse HEAD)"
PLATFORM="${BUILD_PLATFORM:-linux/amd64}"

echo "========================================"
echo " ${PROJECT_NAME} — Build & Push to GHCR"
echo " Tag:      sha-${VERSION} + ${ENV_TAG}"
echo " Registry: ${REGISTRY}"
echo " Platform: ${PLATFORM}"
echo "========================================"

# --- Auth via gh CLI ---------------------------------------------------------
command -v gh >/dev/null 2>&1 \
  || { echo "install gh: https://cli.github.com/" >&2; exit 1; }

GH_TOKEN_VALUE="$(gh auth token 2>/dev/null || true)"
[[ -n "${GH_TOKEN_VALUE}" ]] \
  || { echo "Run: gh auth login --scopes write:packages" >&2; exit 1; }

GH_USER="$(gh api user --jq .login 2>/dev/null)"
echo "Logging into ghcr.io as ${GH_USER}…"
echo "${GH_TOKEN_VALUE}" | docker login ghcr.io -u "${GH_USER}" \
  --password-stdin >/dev/null

# --- Load build env (for frontend NEXT_PUBLIC_* etc.) ------------------------
if [[ -f .env.production ]]; then
  set -a
  # shellcheck disable=SC1091
  source .env.production
  set +a
fi

# --- Ensure dedicated buildx builder (with QEMU/Rosetta for cross-compile) ---
if ! docker buildx inspect "${PROJECT_NAME}-builder" >/dev/null 2>&1; then
  docker buildx create --name "${PROJECT_NAME}-builder" \
    --driver docker-container --bootstrap >/dev/null
fi
docker buildx use "${PROJECT_NAME}-builder" >/dev/null

# --- Build + push each image -------------------------------------------------
# Example: 3 images. Replicate per image your project builds.

echo ""
echo "[1/3] Building + pushing backend (${PLATFORM})…"
docker buildx build \
  --platform "${PLATFORM}" \
  --push \
  -t "${REGISTRY}-backend:sha-${VERSION}" \
  -t "${REGISTRY}-backend:${ENV_TAG}" \
  -f backend/Dockerfile \
  backend/

echo ""
echo "[2/3] Building + pushing frontend (${PLATFORM})…"
docker buildx build \
  --platform "${PLATFORM}" \
  --push \
  --build-arg "NEXT_PUBLIC_API_URL=${NEXT_PUBLIC_API_URL:-https://${DOMAIN}/api}" \
  --build-arg "NEXT_PUBLIC_WS_URL=${NEXT_PUBLIC_WS_URL:-wss://${DOMAIN}/ws}" \
  -t "${REGISTRY}-frontend:sha-${VERSION}" \
  -t "${REGISTRY}-frontend:${ENV_TAG}" \
  -f frontend/Dockerfile \
  frontend/

# If you have a separate services/worker image:
# echo ""
# echo "[3/3] Building + pushing services..."
# docker buildx build \
#   --platform "${PLATFORM}" \
#   --push \
#   -t "${REGISTRY}-services:sha-${VERSION}" \
#   -t "${REGISTRY}-services:${ENV_TAG}" \
#   -f backend/Dockerfile.services backend/

# UFOP: Building admin console (requires workspace context for shared packages)
# echo ""
# echo "[N/N] Building + pushing UFOP admin console..."
# docker buildx build \
#   --platform "${PLATFORM}" \
#   --push \
#   -t "${REGISTRY}-admin:sha-${VERSION}" \
#   -t "${REGISTRY}-admin:${ENV_TAG}" \
#   -f admin/Dockerfile \
#   .  # <-- workspace root context needed for @ufop/* packages

echo ""
echo "========================================"
echo " DONE — Images pushed to GHCR"
echo " sha-${VERSION} (immutable) + ${ENV_TAG} (moving)"
echo " Commit: ${FULL_SHA}"
echo "========================================"
```

### 5.5 `scripts/deploy-prod.sh` — one-shot canonical deploy

```bash
#!/usr/bin/env bash
# =============================================================================
# ${PROJECT_NAME} — One-shot Production Deploy (GHCR pipeline)
# Usage: ./scripts/deploy-prod.sh
# =============================================================================
set -euo pipefail

# --- Configuration -----------------------------------------------------------
VPS_IP="${TRADING_VPS_IP:?Set VPS_IP env var}"
VPS_USER="${TRADING_VPS_USER:-root}"
VPS_PASS="${TRADING_VPS_PASS:?Set VPS_PASS env var}"     # or use SSH key
REMOTE_DIR="${REMOTE_DIR:-/opt/${PROJECT_NAME}}"
REGISTRY="${GHCR_REGISTRY:-ghcr.io/${GH_NAMESPACE}/${PROJECT_NAME}}"

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "${ROOT_DIR}"

VERSION="$(git rev-parse --short HEAD)"
FULL_SHA="$(git rev-parse HEAD)"

echo "============================================================"
echo " ${PROJECT_NAME} Production Deploy"
echo " Tag:    sha-${VERSION}"
echo " Target: ${VPS_USER}@${VPS_IP}:${REMOTE_DIR}"
echo " Time:   $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "============================================================"

# --- 0. Refuse dirty tree (ask for confirmation) -----------------------------
if ! git diff-index --quiet HEAD --; then
  echo ""
  echo "WARNING: working tree has uncommitted changes:"
  git status --short
  read -r -p "Continue anyway? [y/N] " ans
  [[ "${ans:-N}" =~ ^[Yy]$ ]] || { echo "Aborted."; exit 1; }
fi

# --- 1. Pre-push checks (tests + lint + build) -------------------------------
echo ""
echo "[1/6] Running pre-push checks…"
if [[ -x scripts/pre-push-checks.sh ]]; then
  scripts/pre-push-checks.sh
else
  echo "  (skip) scripts/pre-push-checks.sh not found"
fi

# --- 2. Build + push to GHCR -------------------------------------------------
echo ""
echo "[2/6] Build + push images (sha-${VERSION} + production)…"
GHCR_REGISTRY="${REGISTRY}" scripts/build-push-ghcr.sh production

# --- 3. Sync static assets to VPS --------------------------------------------
SSH_OPTS="-o StrictHostKeyChecking=no \
          -o PreferredAuthentications=password \
          -o PubkeyAuthentication=no"
SSH_CMD="sshpass -p '${VPS_PASS}' ssh ${SSH_OPTS} ${VPS_USER}@${VPS_IP}"
RSYNC_CMD="sshpass -p '${VPS_PASS}' rsync -avz --no-perms"

echo ""
echo "[3/6] Syncing static assets (compose file, nginx, scripts)…"
eval "${SSH_CMD}" "mkdir -p ${REMOTE_DIR}/nginx ${REMOTE_DIR}/scripts"

eval "${RSYNC_CMD}" \
  docker-compose.ghcr.yml \
  "${VPS_USER}@${VPS_IP}:${REMOTE_DIR}/docker-compose.ghcr.yml"

eval "${RSYNC_CMD}" \
  nginx/nginx.conf \
  "${VPS_USER}@${VPS_IP}:${REMOTE_DIR}/nginx/nginx.conf"

# Add more asset syncs as needed (db init.sql, backup.sh, etc.)

# --- 4. Login docker to ghcr on the VPS --------------------------------------
echo ""
echo "[4/6] Logging VPS docker into ghcr.io…"
GH_TOKEN_VALUE="$(gh auth token)"
GH_USER="$(gh api user --jq .login 2>/dev/null)"
eval "${SSH_CMD}" \
  "echo '${GH_TOKEN_VALUE}' | docker login ghcr.io -u ${GH_USER} --password-stdin >/dev/null"

# --- 5. Pull new images + recreate containers --------------------------------
echo ""
echo "[5/6] Pulling images + recreating containers (preserves volumes)…"
eval "${SSH_CMD}" "cd ${REMOTE_DIR} && \
  IMAGE_TAG=sha-${VERSION} docker compose -f docker-compose.ghcr.yml pull"

eval "${SSH_CMD}" "cd ${REMOTE_DIR} && \
  IMAGE_TAG=sha-${VERSION} docker compose -f docker-compose.ghcr.yml up -d --remove-orphans"

# Hot-reload nginx so any nginx.conf change applies without restarting
eval "${SSH_CMD}" "docker exec ${PROJECT_NAME}-nginx-1 nginx -t \
  && docker exec ${PROJECT_NAME}-nginx-1 nginx -s reload" || true

# --- 6. Health verification --------------------------------------------------
echo ""
echo "[6/6] Verifying deploy…"
sleep 5
eval "${SSH_CMD}" \
  "docker ps --format '{{.Names}}|{{.Status}}' | grep ${PROJECT_NAME}- | sort"

echo ""
echo "HTTP health checks:"
curl -sS -o /dev/null -w "  /              %{http_code}\n" -L "https://${DOMAIN}/" || true
curl -sS -o /dev/null -w "  /health/       %{http_code}\n" "https://${DOMAIN}/health/" || true

echo ""
echo "============================================================"
echo " DEPLOY COMPLETE"
echo " Tag:    sha-${VERSION}"
echo " Commit: ${FULL_SHA}"
echo ""
echo " Rollback: "
echo "   ssh root@${VPS_IP} 'cd ${REMOTE_DIR} && \\"
echo "     IMAGE_TAG=sha-<prev> docker compose -f docker-compose.ghcr.yml up -d'"
echo "============================================================"
```

### 5.6 `.env.production.example` (committed)

```ini
# Database
POSTGRES_DB=myapp_prod
POSTGRES_USER=myapp
POSTGRES_PASSWORD=                           # REQUIRED — fill in

# Redis
REDIS_URL=redis://redis:6379/0

# Application secrets
SECRET_KEY=                                  # REQUIRED — generate with a CSPRNG

# Frontend build args (baked at image build time)
NEXT_PUBLIC_API_URL=https://${DOMAIN}/api
NEXT_PUBLIC_WS_URL=wss://${DOMAIN}/ws

# Anything else your app needs at runtime
```

Copy this file to `.env.production` (gitignored) and fill in real
values. Copy the same file to the VPS at `${REMOTE_DIR}/.env` with
production-real values.

### 5.7 `.gitignore` additions

```gitignore
# Never commit secrets
.env
.env.local
.env.*.local
.env.production
backend/.env
backend/.env.production

# Deploy scripts that may contain credentials (if you add one)
# scripts/deploy-legacy.sh

# Build artifacts
.next/
dist/
build/
*.tsbuildinfo
node_modules/
__pycache__/
.venv/

# UFOP-specific build artifacts
src-tauri/target/
admin/.next/

# Docker overrides
docker-compose.override.yml

# OS
.DS_Store
Thumbs.db

# IDE
.vscode/
.idea/
*.swp
```

---

## 6. First-Time Setup (Fresh Repo + Fresh VPS)

Work through these in order the first time you set up a new project.

### 6.1 Local machine

```bash
# 1. Install prerequisites (see §4.1)
brew install gh sshpass
gh auth login --scopes write:packages

# 2. Drop the file templates from §5 into your repo
#    - docker-compose.ghcr.yml
#    - nginx/nginx.conf
#    - scripts/pre-push-checks.sh
#    - scripts/build-push-ghcr.sh
#    - scripts/deploy-prod.sh
#    - .env.production.example
chmod +x scripts/*.sh

# 3. Substitute placeholders (quick-and-dirty with sed)
PROJECT_NAME=myapp
GH_NAMESPACE=myorg
DOMAIN=myapp.com
VPS_IP=1.2.3.4
for f in docker-compose.ghcr.yml nginx/nginx.conf scripts/*.sh; do
  sed -i '' \
    -e "s|\${PROJECT_NAME}|${PROJECT_NAME}|g" \
    -e "s|\${GH_NAMESPACE}|${GH_NAMESPACE}|g" \
    -e "s|\${DOMAIN}|${DOMAIN}|g" \
    -e "s|\${VPS_IP}|${VPS_IP}|g" \
    "$f"
done

# 4. Create .env.production with real secrets (never commit)
cp .env.production.example .env.production
$EDITOR .env.production

# 5. Run the pre-push checks to make sure everything works locally
./scripts/pre-push-checks.sh

# 6. Commit the new deploy files
git add docker-compose.ghcr.yml nginx/nginx.conf scripts/ .env.production.example .gitignore
git commit -m "infra: add GHCR deploy pipeline"
git push
```

### 6.2 VPS setup

```bash
# 1. SSH in
ssh root@${VPS_IP}

# 2. Install Docker
curl -fsSL https://get.docker.com | sh

# 3. Install Dokploy (provides Traefik + SSL)
curl -sSL https://dokploy.com/install.sh | sh

# 4. Create the project directory
mkdir -p /opt/${PROJECT_NAME}/{nginx,scripts,backups}

# 5. Write the .env file with production-real secrets
cat > /opt/${PROJECT_NAME}/.env <<'EOF'
POSTGRES_DB=myapp_prod
POSTGRES_USER=myapp
POSTGRES_PASSWORD=<real-value>
SECRET_KEY=<real-value>
# … etc
EOF
chmod 600 /opt/${PROJECT_NAME}/.env

# 6. Make sure Dokploy's network exists
docker network inspect dokploy-network \
  || docker network create dokploy-network

# 7. Exit
exit
```

### 6.3 First push to GHCR

```bash
# From local machine:
./scripts/build-push-ghcr.sh production
```

First build takes longest (~15-25 min on M-series for a Python + Next.js
project). Subsequent builds reuse layer cache (~2-5 min).

Verify packages exist:

```bash
gh api '/user/packages?package_type=container' --jq '.[].name' \
  | grep ${PROJECT_NAME}
```

Expected:
```
${PROJECT_NAME}-backend
${PROJECT_NAME}-frontend
```

### 6.4 First deploy

```bash
# From local machine:
TRADING_VPS_IP=${VPS_IP} TRADING_VPS_PASS=<password> \
  ./scripts/deploy-prod.sh
```

Watch the output. It will:
1. Run pre-push checks (will fail fast if anything's broken)
2. Skip the build step if you just built (layer cache kicks in)
3. scp the compose file + nginx.conf
4. Log docker into GHCR on the VPS
5. Pull images + `up -d`
6. Reload nginx
7. curl-verify the endpoints

### 6.5 Verify Traefik cert issuance

Within 30 seconds of the first `up -d`, Traefik should auto-provision a
Let's Encrypt cert. Check:

```bash
ssh root@${VPS_IP} 'docker logs dokploy-traefik 2>&1 | grep -i "acme\|certificate"' | tail -20
```

Expected: lines about "certificate obtained" for `${DOMAIN}`.

Then:

```bash
curl -I https://${DOMAIN}/
```

Expected: `HTTP/2 200` (or 307 if your root redirects to /login).

---

## 7. Routine Deploy

Once set up, every deploy is:

```bash
./scripts/deploy-prod.sh
```

The script handles the whole flow. Typical timing:
- First build after a major dependency change: 5-15 min
- Normal code change (cached layers): 2-5 min
- Code-only change (just `COPY . /app`): ~30 seconds

---

## 8. Rollback Procedures

### 8.1 Roll back the last deploy (image-level)

```bash
# Find the previous short SHA
PREV_SHA="$(git log --oneline -2 | tail -1 | awk '{print $1}')"

# Redeploy with that tag (pulls the immutable old image from GHCR)
ssh root@${VPS_IP} "cd ${REMOTE_DIR} && \
  IMAGE_TAG=sha-${PREV_SHA} docker compose -f docker-compose.ghcr.yml up -d"
```

Compose detects the image tag changed and recreates the affected
containers. Volumes persist. Downtime per container: ~15-30s.

### 8.2 Roll back a database migration

Image rollback does **not** automatically roll back DB schema changes.
If you shipped a schema migration along with code, roll the schema
back explicitly:

```bash
# For Django:
ssh root@${VPS_IP} \
  "docker exec ${PROJECT_NAME}-backend-1 \
   python manage.py migrate <app> <previous_migration_number>"

# For Rails:
ssh root@${VPS_IP} \
  "docker exec ${PROJECT_NAME}-backend-1 bundle exec rails db:rollback"

# For raw SQL:
ssh root@${VPS_IP} \
  "docker exec ${PROJECT_NAME}-postgres-1 \
   psql -U ${POSTGRES_USER} -d ${POSTGRES_DB} < rollback.sql"

# For UFOP (SQLite via rusqlite):
# SQLite migrations in UFOP are forward-only via all_migrations() in
# src-tauri/src/storage/migrations.rs. Rollback requires manual DB
# intervention or restoring from backup. Always verify backward
# compatibility before deploying schema changes.
```

### 8.3 Full stack rollback (nuclear option)

If `docker-compose.ghcr.yml` itself is broken:

```bash
ssh root@${VPS_IP} "cd ${REMOTE_DIR} && \
  docker compose -f docker-compose.ghcr.yml down && \
  cp docker-compose.ghcr.yml.bak.<timestamp> docker-compose.ghcr.yml && \
  IMAGE_TAG=sha-<prev> docker compose -f docker-compose.ghcr.yml up -d"
```

Tip: add a step to `deploy-prod.sh` that copies the current compose file
to `.bak.$(date +%Y%m%d-%H%M%S)` before overwriting. That's your
automatic rollback safety net.

---

## 9. Verification Checklist

Run after every deploy.

### 9.1 HTTP reachability

```bash
curl -sS -o /dev/null -w "root       = %{http_code}\n" -L https://${DOMAIN}/
curl -sS -o /dev/null -w "health/    = %{http_code}\n"    https://${DOMAIN}/health/
curl -sS -o /dev/null -w "health/live= %{http_code}\n"    https://${DOMAIN}/health/live/
```

Expected: all `200` (or `307` for root → /login).

### 9.2 Container state

```bash
ssh root@${VPS_IP} 'docker ps --format "{{.Names}}|{{.Status}}" | grep '${PROJECT_NAME}-'| sort'
```

Expected: all containers `Up N seconds/minutes`. Backend, frontend,
postgres, redis should carry `(healthy)` suffix.

### 9.3 Restart counts (should all be 0)

```bash
ssh root@${VPS_IP} '
  for c in $(docker ps --format "{{.Names}}" | grep '${PROJECT_NAME}-'); do
    restarts=$(docker inspect -f "{{.RestartCount}}" "$c")
    printf "%-50s restarts=%s\n" "$c" "$restarts"
  done'
```

Any `restarts > 0` means the container crashed at least once — investigate.

### 9.4 nginx is using the variable-proxy_pass pattern

```bash
ssh root@${VPS_IP} \
  'docker exec '${PROJECT_NAME}-nginx-1' grep -E "resolver|set \$upstream" /etc/nginx/nginx.conf'
```

Expected:
- One `resolver 127.0.0.11 valid=10s ipv6=off;` line
- Multiple `set $upstream_<X> <service>:<port>;` lines
- **Zero** `upstream {}` blocks

### 9.5 Image tag matches the commit you deployed

```bash
ssh root@${VPS_IP} \
  'docker inspect '${PROJECT_NAME}-backend-1' --format "{{.Config.Image}}"'
```

Expected: `ghcr.io/${GH_NAMESPACE}/${PROJECT_NAME}-backend:sha-${VERSION}`.

---

## 10. Universal Gotchas & Lessons Learned

Every item here is a real incident from the battle-tested project this
template came from. If you hit any of these, look here first.

### 10.1 nginx stale upstream IP → silent 502s

**Symptom:** `curl https://${DOMAIN}/` returns 502. `docker ps` shows
the upstream container healthy. `docker exec nginx wget
http://upstream:PORT/` works.

**Root cause:** `upstream { server X; }` blocks resolve the hostname
once at nginx startup. When the upstream container is recreated (new
Docker IP), nginx keeps hitting the old IP.

**Fix:** always use `resolver 127.0.0.11 valid=10s ipv6=off;` + variable
proxy_pass (see §5.2). Test by recreating a container and immediately
hitting its endpoint.

### 10.2 arm64 vs amd64 manifest mismatch

**Symptom:** `docker compose pull` on the VPS fails with
`no matching manifest for linux/amd64 in the manifest list entries`.

**Root cause:** on an M-series Mac, plain `docker build` produces only
linux/arm64 manifests. Most VPS hosts are linux/amd64.

**Fix:** always use `docker buildx build --platform linux/amd64
--push`. The `--push` is important — cross-compiled images can't be
loaded into the local Docker daemon on an arm64 host.

### 10.3 Backend healthcheck deadlock at first boot

**Symptom:** `docker compose up -d` hangs for minutes with
`Container X Waiting` on backend.

**Root cause:** two stacked issues:
1. Healthcheck uses `curl`, but the base image (e.g., `python:slim`)
   doesn't include curl.
2. The healthcheck probes a full `/health/` endpoint that depends on
   other services being registered — but those other services are
   `depends_on backend service_healthy` → chicken-and-egg.

**Fix:** use `python -c 'import urllib.request'` (or `wget` from
busybox in alpine) and probe a pure **liveness** endpoint, not the
aggregate health endpoint. See §5.1.

### 10.4 `/opt/project` on the VPS isn't a git clone

**Symptom:** `git pull` on the VPS fails with `No commits yet on
master` or `not a git repository`.

**Root cause:** the directory was seeded by rsync or by Dokploy
(without git checkout). Don't assume it's a real clone.

**Fix:** don't rely on `git pull` on the VPS. Rsync the files you need
(compose file, nginx.conf, init.sql). Everything else lives inside the
image.

### 10.5 Dokploy "git auto-deploy" is unreliable

**Symptom:** `git push origin master` succeeds, but the VPS is
unchanged.

**Root cause:** Dokploy's git auto-deploy requires explicit project
setup in its UI. If you skipped that, it never watches anything. Even
when wired, webhook flakiness and layered auth can make it unreliable.

**Fix:** don't rely on Dokploy for deploys. Use it only for Traefik +
SSL routing. Do your own deploys via `./scripts/deploy-prod.sh`. This
is also easier to debug and gives immediate feedback.

### 10.6 SSH fail2ban rate-limits

**Symptom:** `ssh` fails with `Permission denied (publickey,password)`
even though you're using the right password. Repeated tries stay failed
for 24 hours.

**Root cause:** fail2ban treats each pubkey probe + each password
failure as a failed attempt. 3 fails → 24-hour IP ban.

**Fix:** always use these SSH options in scripts:

```bash
ssh -o StrictHostKeyChecking=no \
    -o PreferredAuthentications=password \
    -o PubkeyAuthentication=no \
    root@${VPS_IP}
```

Also: combine commands into single SSH sessions. Instead of 3 separate
`ssh` calls, use one with `&&`:

```bash
# Wrong — 3 separate attempts
ssh root@VPS 'docker ps'
ssh root@VPS 'docker logs backend'
ssh root@VPS 'curl localhost:8000'

# Right — 1 attempt
ssh root@VPS 'docker ps && docker logs backend && curl localhost:8000'
```

Long-term: switch to SSH key auth and drop `sshpass`.

### 10.7 GHCR packages default to private

**Symptom:** `docker compose pull` on the VPS fails with 401 or "denied:
denied".

**Root cause:** packages pushed via `gh auth token` default to private.
The VPS has no credentials.

**Fix:** the deploy script should `docker login ghcr.io` on the VPS
using the operator's `gh auth token` passed over SSH. This is ephemeral
and tied to the current session — every deploy re-logins.

Alternative: make packages public via the GitHub UI at
`https://github.com/users/<you>/packages/container/<pkg>/settings`.
Only do this if you're okay with the image contents being world-readable.

Another alternative: create a dedicated PAT on the VPS with
`read:packages` only, stored in `/root/.docker/config.json`. More
durable than ephemeral login, more complex to rotate.

### 10.8 Volumes get orphaned if compose project name changes

**Symptom:** after a deploy, the database appears empty even though
`docker volume ls` shows the old volume still exists.

**Root cause:** named volumes are prefixed with the compose **project
name**. By default, the project name is the directory name. If you
move the compose file to a different directory (or pass `-p` with a
different project name), compose creates fresh volumes and the old
ones are orphaned (not deleted, just not attached).

**Fix:** keep the project directory stable. Or always pass
`--project-name <same-name>` explicitly to every compose command.

### 10.9 `gh auth token` can expire

**Symptom:** `gh auth token` returns empty or deploys fail with 401 on
the GHCR login step.

**Fix:** re-authenticate:

```bash
gh auth logout
gh auth login --scopes write:packages
```

### 10.10 First-time build is slow; subsequent builds are fast

**Symptom:** first `build-push-ghcr.sh` takes 15-30 minutes. Operator
assumes this is broken.

**Root cause:** buildx has no layer cache yet. All Python wheels, npm
packages, etc. are being compiled fresh (possibly under Rosetta/QEMU if
cross-compiling).

**Fix:** just wait. Second build will be 2-5x faster because layers
are cached. Don't cancel.

Also: add `docker/setup-buildx-action@v3` equivalent — just use a
dedicated buildx builder (`docker buildx create --name X
--driver docker-container`) so its cache survives across runs.

### 10.11 Forgetting to sync the compose file after editing

**Symptom:** you changed `docker-compose.ghcr.yml` locally, ran
`deploy-prod.sh`, but the change isn't in effect on the VPS.

**Root cause:** the script rsynced the file but compose sees the file
on disk hasn't changed enough to trigger a recreate. Or, more commonly,
the rsync step itself was skipped or the local file wasn't saved.

**Fix:** the `deploy-prod.sh` template above always rsyncs the compose
file. Verify on the VPS:

```bash
ssh root@VPS "md5sum ${REMOTE_DIR}/docker-compose.ghcr.yml" \
  && md5 docker-compose.ghcr.yml
```

The two hashes should match after a deploy.

### 10.12 UFOP-specific: pnpm workspace build context

**Symptom:** Admin console Docker build fails with `Cannot find module '@ufop/design-tokens'` or `@ufop/ui-components`.

**Root cause:** The Dockerfile's build context is set to `admin/` but the shared workspace packages live at the repo root under `packages/`.

**Fix:** Use the repo root as the Docker build context with `-f admin/Dockerfile .` and copy the workspace packages into the image. Or use a multi-stage build that first installs workspace dependencies.

### 10.13 UFOP-specific: SQLite WAL file in Docker volume

**Symptom:** SQLite database appears empty or locked after container recreation.

**Root cause:** SQLite WAL mode creates `-wal` and `-shm` files alongside the main `.db` file. If the volume mount doesn't include the directory containing all three files, data loss can occur.

**Fix:** Always mount the entire directory containing the SQLite database as a named volume, not just the `.db` file. Ensure the Rust binary writes to a path inside the volume mount.

---

## 11. Troubleshooting Matrix

| Symptom | Likely cause | First thing to check/fix |
|---|---|---|
| `/` returns 502 | nginx stale upstream IP | `docker exec nginx grep resolver /etc/nginx/nginx.conf` — if missing the resolver line, §5.2 |
| `docker compose pull` = "no matching manifest" | arm64 image pushed, VPS is amd64 | Rebuild with `--platform linux/amd64 --push` (see §5.4) |
| `up -d` hangs on backend | healthcheck deadlock or curl missing | Fix healthcheck to `python urllib` probing `/health/live/` (§5.1) |
| SSH "Permission denied" | fail2ban ban + pubkey probe | Wait 90s; use `-o PreferredAuthentications=password -o PubkeyAuthentication=no` |
| `docker login ghcr.io` 401 | gh token expired | `gh auth login --scopes write:packages` |
| Containers exit: "X is required" | `.env` file missing on VPS | `ls ${REMOTE_DIR}/.env` — create it with production values |
| Frontend build fails: "NEXT_PUBLIC_X required" | build args not loaded | Ensure `.env.production` is loaded in `build-push-ghcr.sh` before `docker buildx build` |
| Traefik issues no cert | nginx not joined to `dokploy-network` | Add `dokploy-network` to nginx service in compose file |
| Database appears empty after deploy | volume orphaned | `docker volume ls` — check project name prefix matches working directory |
| Heartbeats stale but containers running | internal service bug or Redis state loss | Check app-specific logs; Redis volume intact? |
| `git pull` on VPS fails | directory isn't a git clone | Don't use git pull on VPS. Rsync files instead |
| `Cannot find module '@ufop/*'` | pnpm workspace context missing | Use repo root as Docker build context (UFOP §10.12) |
| SQLite "database is locked" | WAL files not in volume mount | Mount entire SQLite directory as volume (UFOP §10.13) |

---

## 12. Security Notes

### 12.1 Secrets that MUST NOT be committed

- `.env` / `.env.production` / `backend/.env`
- Any file containing `PASSWORD`, `SECRET_KEY`, `API_KEY`, `TOKEN`
- Deploy scripts with hardcoded credentials (add them to `.gitignore`)

The `.gitignore` template in §5.7 covers these.

### 12.2 Hardcoded passwords in deploy scripts

**DON'T** write `VPS_PASS="hardcoded-password"` in `deploy-prod.sh`.

**DO** force the env var to be set:

```bash
VPS_PASS="${TRADING_VPS_PASS:?Set TRADING_VPS_PASS env var}"
```

The script exits immediately if the env var is missing — fail fast,
no secret exposure.

Even better: migrate to SSH key auth and drop `sshpass` entirely.
Long-term this is the right answer.

### 12.3 GHCR auth: PAT vs gh CLI

| Option | Pros | Cons |
|---|---|---|
| `gh auth token` | No separate secret to manage | Expires with operator session; one-operator only |
| Dedicated PAT | Works unattended, can be scoped read-only on VPS | One more secret to rotate; needs a secret manager |

For a single-operator project, `gh auth token` is simpler. For a team
or unattended deploys, use a PAT with `read:packages` on the VPS.

### 12.4 Dokploy admin UI exposure

Dokploy runs its admin UI on port 3000 by default. Block it from the
public internet:

```bash
# ufw
ufw deny 3000/tcp
ufw allow from <your-home-ip> to any port 3000

# or iptables
iptables -A INPUT -p tcp --dport 3000 -j DROP
iptables -I INPUT -p tcp -s <your-home-ip> --dport 3000 -j ACCEPT
```

### 12.5 Immutable SHA tags for rollback safety

**Always tag images with both `sha-<short>` (immutable) and a moving
`production` tag.** This way:

- Normal deploys bump `production` to the latest SHA
- Rollbacks set `IMAGE_TAG=sha-<prev>` and reuse the old immutable image
- SHA tags are retained in GHCR for ~6 months by default

Never overwrite a SHA tag. Never delete them. Rollback is free and
instant only if the old image is still in the registry.

---

## 13. AI Agent Implementation Checklist

Walk through this list in order when implementing the template on a
new project. Do not skip steps.

### Phase 0 — Discovery (read-only)

- [ ] Confirm the project has a GitHub repo: `git remote -v`
- [ ] Confirm operator has `gh` CLI authed: `gh auth status`
- [ ] Get the GHCR namespace: `gh api user --jq .login`
- [ ] Inventory existing Dockerfiles: `find . -name "Dockerfile*"`
- [ ] Inventory existing compose files: `ls docker-compose*.yml`
- [ ] Read existing `.env.example` / `.env.production.example` for the
      list of required env vars
- [ ] Check if the VPS is reachable: `ssh -o ConnectTimeout=5
      root@${VPS_IP} 'uname -a'`
- [ ] Check VPS architecture: `ssh root@VPS 'uname -m'`
      (amd64 = most VPS hosts)
- [ ] Check local architecture: `uname -m`
      (arm64 = M-series Mac → cross-compile needed)
- [ ] Check Dokploy is installed on the VPS: `docker ps | grep dokploy`
- [ ] Check the `dokploy-network` exists on the VPS:
      `docker network ls | grep dokploy-network`
- [ ] Read any existing deploy scripts and **look for hardcoded secrets**
      that need to be refactored out
- [ ] **UFOP:** Check for pnpm-workspace.yaml and shared packages
- [ ] **UFOP:** Check for src-tauri/ (Rust backend — desktop only, not for VPS)
- [ ] **UFOP:** Inventory admin/ directory structure for Next.js admin console

### Phase 1 — Placeholder values

- [ ] Decide `PROJECT_NAME` (short, lowercase, no spaces)
- [ ] Decide `GH_NAMESPACE` (usually the GitHub org or user)
- [ ] Decide `DOMAIN` (the public domain for this project)
- [ ] Decide `VPS_IP` (where it runs)
- [ ] Decide `REMOTE_DIR` (usually `/opt/${PROJECT_NAME}`)
- [ ] List all image names the project needs (one per `Dockerfile*`)
- [ ] List all named volumes that must be preserved across deploys
- [ ] **UFOP:** Identify which workspace packages are needed at build time

### Phase 2 — Write `docker-compose.ghcr.yml`

- [ ] Copy `docker-compose.prod.yml` as the starting point
- [ ] For each `build:` block, replace with
      `image: ghcr.io/${GH_NAMESPACE}/${PROJECT_NAME}-<svc>:${IMAGE_TAG:-production}`
- [ ] Remove all `build:` lines
- [ ] Keep env var anchors, healthchecks, volume mounts,
      `depends_on`, Traefik labels
- [ ] Change backend healthcheck to the `python urllib` form probing
      `/health/live/` (or equivalent liveness endpoint)
- [ ] Ensure `dokploy-network: external: true` is in the networks section
- [ ] Validate locally:
      `docker compose -f docker-compose.ghcr.yml config --quiet`
      (use stub env vars if real ones are missing)

### Phase 3 — Fix `nginx/nginx.conf` (if using nginx)

- [ ] Add `resolver 127.0.0.11 valid=10s ipv6=off;` inside the
      `http {}` block
- [ ] Convert every `proxy_pass http://<service>:<port>;` to:
      ```
      set $upstream_<name> <service>:<port>;
      proxy_pass http://$upstream_<name>;
      ```
- [ ] Remove any `upstream {}` blocks
- [ ] Validate: `docker run --rm -v "$PWD/nginx/nginx.conf":/etc/nginx/nginx.conf:ro nginx:1.27-alpine nginx -t`

### Phase 4 — Write deploy scripts

- [ ] Create `scripts/pre-push-checks.sh` (adapted for your stack)
- [ ] Create `scripts/build-push-ghcr.sh` (one build per Dockerfile)
- [ ] Create `scripts/deploy-prod.sh` (orchestrator)
- [ ] `chmod +x scripts/*.sh`
- [ ] Ensure **no hardcoded secrets** — use `${VAR:?required}` pattern
- [ ] Test `build-push-ghcr.sh` by running it and checking GHCR

### Phase 5 — VPS first-time setup

- [ ] SSH into VPS
- [ ] Verify Docker: `docker --version`
- [ ] Verify Dokploy: `docker ps | grep dokploy`
- [ ] Create `${REMOTE_DIR}/` with subdirectories
- [ ] Create `.env` on VPS with production secrets
      (`chmod 600`)
- [ ] Verify `dokploy-network` exists:
      `docker network ls | grep dokploy`
- [ ] Exit

### Phase 6 — First deploy

- [ ] Run `./scripts/deploy-prod.sh`
- [ ] Watch the 6-step output
- [ ] Check Traefik cert issuance:
      `ssh root@VPS 'docker logs dokploy-traefik 2>&1 | grep certificate'`
- [ ] Check HTTPS:
      `curl -I https://${DOMAIN}/`
- [ ] Check all containers healthy:
      `ssh root@VPS 'docker ps --filter name=${PROJECT_NAME}'`
- [ ] Check all restart counts = 0

### Phase 7 — Rollback test

- [ ] Do a known-good deploy (sha-X)
- [ ] Do a new deploy (sha-Y)
- [ ] Roll back to sha-X: `IMAGE_TAG=sha-X docker compose -f ghcr.yml up -d`
- [ ] Verify sha-X is running
- [ ] Re-deploy sha-Y
- [ ] Confirm rollback cycle works

---

## 14. Extending the Template

### 14.1 Adding a new service image

1. Create `Dockerfile.<service>` (or `services/<service>/Dockerfile`)
2. Add a new block to `docker-compose.ghcr.yml`:
   ```yaml
   new-service:
     image: ghcr.io/${GH_NAMESPACE}/${PROJECT_NAME}-new-service:${IMAGE_TAG:-production}
     ...
   ```
3. Add a build step to `build-push-ghcr.sh`
4. If it needs nginx routing, add a `location` block
5. Deploy

### 14.2 Adding database backups

```bash
#!/usr/bin/env bash
# scripts/backup.sh — run via cron on VPS
set -euo pipefail
TIMESTAMP=$(date +%Y%m%d-%H%M%S)
docker exec ${PROJECT_NAME}-postgres-1 \
  pg_dump -U ${POSTGRES_USER} -d ${POSTGRES_DB} \
  | gzip > /opt/${PROJECT_NAME}/backups/backup-${TIMESTAMP}.sql.gz

# Keep last 7 days
find /opt/${PROJECT_NAME}/backups/ -name "*.sql.gz" -mtime +7 -delete
```

Cron on the VPS:
```
0 */6 * * * /opt/${PROJECT_NAME}/scripts/backup.sh >> /var/log/${PROJECT_NAME}-backup.log 2>&1
```

**UFOP SQLite backup variant:**
```bash
# SQLite backup — uses sqlite3 .backup command for consistency
docker exec ${PROJECT_NAME}-rust-backend-1 \
  sqlite3 /app/data/ufop.db ".backup '/app/data/backups/backup-${TIMESTAMP}.db'"
gzip /opt/${PROJECT_NAME}/data/backups/backup-${TIMESTAMP}.db
```

### 14.3 Adding staging environment

Duplicate `docker-compose.ghcr.yml` to `docker-compose.staging.yml`:
- Different `COMPOSE_PROJECT_NAME` (avoids volume collision)
- Different `DOMAIN` (staging subdomain)
- Same image tags but from a `staging` branch or separate tag

### 14.4 Adding Celery / async workers

Add to compose:
```yaml
celery-worker:
  image: ghcr.io/${GH_NAMESPACE}/${PROJECT_NAME}-backend:${IMAGE_TAG:-production}
  command: celery -A myapp worker -l INFO --concurrency 4
  depends_on:
    redis:
      condition: service_healthy
  restart: unless-stopped
  networks:
    - app-net

celery-beat:
  image: ghcr.io/${GH_NAMESPACE}/${PROJECT_NAME}-backend:${IMAGE_TAG:-production}
  command: celery -A myapp beat -l INFO
  depends_on:
    redis:
      condition: service_healthy
  restart: unless-stopped
  networks:
    - app-net
```

### 14.5 Multi-domain routing

Add more Traefik router labels in the compose file:
```yaml
labels:
  - "traefik.http.routers.${PROJECT_NAME}-api.rule=Host(`api.${DOMAIN}`)"
  - "traefik.http.routers.${PROJECT_NAME}-api.entrypoints=websecure"
  - "traefik.http.routers.${PROJECT_NAME}-api.tls.certresolver=letsencrypt"
  - "traefik.http.services.${PROJECT_NAME}-api.loadbalancer.server.port=8000"
```

### 14.6 Adding monitoring (Grafana + Prometheus)

Add to compose:
```yaml
prometheus:
  image: prom/prometheus:latest
  volumes:
    - ./prometheus/prometheus.yml:/etc/prometheus/prometheus.yml:ro
  networks:
    - app-net

grafana:
  image: grafana/grafana:latest
  volumes:
    - grafana_data:/var/lib/grafana
  labels:
    - "traefik.http.routers.grafana.rule=Host(`grafana.${DOMAIN}`)"
  networks:
    - app-net
    - dokploy-network
```

---

*End of Generic GHCR + Dokploy + VPS Deploy Pipeline v1.0*
*Customized for UFOP: Tauri 2.0 + Rust + React/Vite + Next.js Admin + SQLite/rusqlite + pnpm workspaces*
*Platform: Dokploy (exclusive — never Coolify)*
