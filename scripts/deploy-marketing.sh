#!/usr/bin/env bash
# =============================================================================
# FileManager marketing — one-shot deploy to singularity (184.70.179.66).
#
# Usage: ./scripts/deploy-marketing.sh
#
# Flow:
#   1. Verify git tree (warn on dirty).
#   2. Build + push marketing image to GHCR (sha-<short> + production tags).
#   3. Rsync compose file + Caddyfile to /opt/filemanager on the VPS.
#   4. SSH: docker login ghcr.io + compose pull + compose up -d.
#   5. Health-check the four production URLs over HTTPS.
# =============================================================================
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

# Load credentials from .env.local (gitignored)
if [[ -f .env.local ]]; then
    set -a
    # shellcheck disable=SC1091
    source .env.local
    set +a
fi

VPS_HOST="${DEPLOY_HOST:-184.70.179.66}"
VPS_USER="${DEPLOY_SSH_USER:-mddiya}"
VPS_PORT="${DEPLOY_SSH_PORT:-22022}"
VPS_KEY="${DEPLOY_SSH_KEY:-$HOME/.ssh/id_ed25519}"
VPS_SUDO_PASSWORD="${DEPLOY_SUDO_PASSWORD:-}"
REMOTE_DIR="${REMOTE_DIR:-/opt/filemanager}"

SSH_BASE=(ssh -i "$VPS_KEY" -p "$VPS_PORT" -o StrictHostKeyChecking=no "${VPS_USER}@${VPS_HOST}")
RSYNC_RSH="ssh -i $VPS_KEY -p $VPS_PORT -o StrictHostKeyChecking=no"

VERSION="$(git rev-parse --short HEAD)"
FULL_SHA="$(git rev-parse HEAD)"

echo "============================================================"
echo " FileManager marketing deploy"
echo " Tag:    sha-${VERSION}"
echo " Target: ${VPS_USER}@${VPS_HOST}:${REMOTE_DIR}"
echo " Time:   $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "============================================================"

# --- 0. Dirty tree warning ---------------------------------------------------
if ! git diff-index --quiet HEAD --; then
    echo ""
    echo "WARNING: working tree has uncommitted changes:"
    git status --short
    read -r -p "Continue anyway? [y/N] " ans
    [[ "${ans:-N}" =~ ^[Yy]$ ]] || { echo "Aborted."; exit 1; }
fi

# --- 1. Build + push image ---------------------------------------------------
echo ""
echo "[1/5] Building + pushing image..."
./scripts/build-push-marketing.sh production

# --- 2. Ensure remote directory exists ---------------------------------------
echo ""
echo "[2/5] Preparing remote directory ${REMOTE_DIR}..."
if [[ -n "$VPS_SUDO_PASSWORD" ]]; then
    "${SSH_BASE[@]}" "echo '$VPS_SUDO_PASSWORD' | sudo -S mkdir -p ${REMOTE_DIR}/releases-proxy && \
        echo '$VPS_SUDO_PASSWORD' | sudo -S chown -R ${VPS_USER}:${VPS_USER} ${REMOTE_DIR}"
else
    "${SSH_BASE[@]}" "mkdir -p ${REMOTE_DIR}/releases-proxy"
fi

# --- 3. Sync compose + Caddyfile ---------------------------------------------
echo ""
echo "[3/5] Syncing compose file and Caddyfile..."
rsync -avz -e "$RSYNC_RSH" \
    docker-compose.ghcr.yml \
    "${VPS_USER}@${VPS_HOST}:${REMOTE_DIR}/docker-compose.ghcr.yml"

rsync -avz -e "$RSYNC_RSH" \
    releases-proxy/Caddyfile \
    "${VPS_USER}@${VPS_HOST}:${REMOTE_DIR}/releases-proxy/Caddyfile"

# Optional .env.production if present (gitignored)
if [[ -f .env.production ]]; then
    rsync -avz -e "$RSYNC_RSH" \
        .env.production \
        "${VPS_USER}@${VPS_HOST}:${REMOTE_DIR}/.env"
fi

# --- 4. GHCR auth + compose pull/up on the VPS -------------------------------
echo ""
echo "[4/5] Logging VPS docker into ghcr.io + pulling + restarting..."

GH_TOKEN_VALUE="$(gh auth token)"
GH_USER="$(gh api user --jq .login 2>/dev/null)"

# Pipe token through SSH stdin so it never appears in process listings.
echo "${GH_TOKEN_VALUE}" | "${SSH_BASE[@]}" "docker login ghcr.io -u ${GH_USER} --password-stdin >/dev/null"

"${SSH_BASE[@]}" "cd ${REMOTE_DIR} && \
    IMAGE_TAG=sha-${VERSION} docker compose -f docker-compose.ghcr.yml pull"

"${SSH_BASE[@]}" "cd ${REMOTE_DIR} && \
    IMAGE_TAG=sha-${VERSION} docker compose -f docker-compose.ghcr.yml up -d --remove-orphans"

# --- 5. Verify --------------------------------------------------------------
echo ""
echo "[5/5] Verifying deploy..."
sleep 5

"${SSH_BASE[@]}" "docker ps --format '{{.Names}}|{{.Status}}' | grep filemanager- | sort"

echo ""
echo "HTTP health checks (allow ~30s after first deploy for Traefik cert issuance):"
for url in https://filemanager.clappe.com/ \
           https://www.filemanager.clappe.com/ \
           https://docs.filemanager.clappe.com/ \
           https://releases.filemanager.clappe.com/healthz; do
    code=$(curl -sS -o /dev/null -w "%{http_code}" -L "$url" --max-time 10 || echo "fail")
    printf "  %-55s %s\n" "$url" "$code"
done

echo ""
echo "============================================================"
echo " DEPLOY COMPLETE"
echo " Tag:    sha-${VERSION}"
echo " Commit: ${FULL_SHA}"
echo ""
echo " Rollback to a previous SHA:"
echo "   ssh -p ${VPS_PORT} ${VPS_USER}@${VPS_HOST} 'cd ${REMOTE_DIR} && \\"
echo "     IMAGE_TAG=sha-<prev> docker compose -f docker-compose.ghcr.yml up -d'"
echo "============================================================"
