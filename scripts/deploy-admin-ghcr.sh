#!/usr/bin/env bash
# Deploy a locally verified immutable GHCR admin image to Singularity.
# Usage: scripts/deploy-admin-ghcr.sh FULL_COMMIT_SHA /protected/bootstrap.env
# The env file must contain the real ADMIN_* / NEXTAUTH_SECRET values, never placeholders.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
SHA="${1:?Pass the full tested commit SHA}"
ENV_FILE="${2:?Pass a protected runtime env file}"
[[ "$SHA" =~ ^[a-f0-9]{40}$ ]] || { echo "A full 40-character commit SHA is required" >&2; exit 2; }
[[ -f "$ENV_FILE" ]] || { echo "Runtime env file is missing" >&2; exit 2; }
TARGET="${DEPLOY_SSH_ALIAS:-singularity}"
REMOTE="${ADMIN_REMOTE_DIR:-/opt/filemanager/admin}"
[[ "$TARGET" =~ ^[A-Za-z0-9._-]+$ && "$REMOTE" =~ ^/[A-Za-z0-9/_-]+$ ]] || { echo "Unsafe target or directory" >&2; exit 2; }
IMAGE="ghcr.io/clappediya/filemanager-admin:sha-${SHA}"
docker buildx imagetools inspect "$IMAGE" >/dev/null
# Existing SSH trust is required; never disable host-key validation.
ssh -o BatchMode=yes -o ConnectTimeout=10 "$TARGET" "set -eu; mkdir -p '$REMOTE'; chmod 700 '$REMOTE'; docker network inspect dokploy-network >/dev/null"
# Refuse a directory that belongs to a different compose project.
ssh "$TARGET" "set -eu; test ! -f '$REMOTE/compose.yml' || grep -q '^name: filemanager-admin$' '$REMOTE/compose.yml'"
# A changed encryption key would make existing stored credentials unreadable.
if ssh "$TARGET" "test -f '$REMOTE/.env'"; then
    LOCAL_ENCRYPTION_KEY="$(sed -n 's/^ADMIN_ENCRYPTION_KEY=//p' "$ENV_FILE")"
    REMOTE_ENCRYPTION_KEY="$(ssh "$TARGET" "sed -n 's/^ADMIN_ENCRYPTION_KEY=//p' '$REMOTE/.env'")"
    if [[ -z "$LOCAL_ENCRYPTION_KEY" || "$LOCAL_ENCRYPTION_KEY" != "$REMOTE_ENCRYPTION_KEY" ]]; then
        echo "Refusing to replace the existing credential encryption key; preserve the runtime environment." >&2
        exit 1
    fi
    unset LOCAL_ENCRYPTION_KEY REMOTE_ENCRYPTION_KEY
fi
# Capture the rollback identity before replacing the compose/environment files.
ssh "$TARGET" "set -eu; docker ps --filter label=com.docker.compose.project=filemanager-admin --format '{{.Image}}' > '$REMOTE/previous-image.txt'; if test -f '$REMOTE/compose.yml'; then cp '$REMOTE/compose.yml' '$REMOTE/compose.previous.yml'; fi; if test -f '$REMOTE/.env'; then cp '$REMOTE/.env' '$REMOTE/.env.previous'; chmod 600 '$REMOTE/.env.previous'; fi"
chmod 600 "$ENV_FILE"
rsync -a "$ENV_FILE" "${TARGET}:${REMOTE}/.env"
ssh "$TARGET" "chmod 600 '$REMOTE/.env'"
rsync -a docker-compose.admin.ghcr.yml "${TARGET}:${REMOTE}/compose.yml"
GHCR_USER="$(gh api user --jq .login)"
[[ "$GHCR_USER" =~ ^[A-Za-z0-9-]+$ ]] || { echo "Invalid registry user" >&2; exit 2; }
gh auth token | ssh "$TARGET" "docker login ghcr.io -u '$GHCR_USER' --password-stdin >/dev/null"
ssh "$TARGET" "set -eu; cd '$REMOTE'; IMAGE_TAG='sha-$SHA' docker compose --env-file .env -f compose.yml config --quiet; IMAGE_TAG='sha-$SHA' docker compose --env-file .env -f compose.yml pull"
ssh "$TARGET" "cd '$REMOTE'; sh -s -- 'sha-$SHA'" <<'REMOTE_SH'
set -eu
candidate="$1"
if IMAGE_TAG="$candidate" docker compose --env-file .env -f compose.yml up -d --wait --wait-timeout 120; then
    exit 0
fi
previous="$(head -n 1 previous-image.txt)"
case "$previous" in
  ghcr.io/clappediya/filemanager-admin:sha-*)
    if test -f compose.previous.yml && test -f .env.previous; then
      IMAGE_TAG="${previous##*:}" docker compose --env-file .env.previous -f compose.previous.yml up -d --wait --wait-timeout 120
      echo "Candidate failed; restored the preceding admin image." >&2
    fi
    ;;
  *) IMAGE_TAG="$candidate" docker compose --env-file .env -f compose.yml stop admin ;;
esac
exit 1
REMOTE_SH
echo "Admin container started from $IMAGE; HTTPS/browser verification is still required."
