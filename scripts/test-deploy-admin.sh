#!/usr/bin/env bash
# Reproduce a first deploy with a portable rsync interface and real fixture files.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
mkdir -p "$T/bin" "$T/remote"
printf 'ADMIN_EMAIL=fixture@example.invalid\nADMIN_PASSWORD=fixture-only\n' > "$T/bootstrap.env"
chmod 644 "$T/bootstrap.env"
cat > "$T/bin/ssh" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
case "$*" in
  "fixture-vps test -f '/opt/filemanager/admin/.env'") exit 1 ;;
  *"docker login"*|*"sh -s --"*) cat >/dev/null ;;
esac
MOCK
cat > "$T/bin/rsync" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
[[ "$1" == -a && "$#" == 3 ]] || { echo "unsupported rsync arguments" >&2; exit 2; }
case "$3" in
  *:*/.env) cp -p "$2" "$UFOP_DEPLOY_FIXTURE/remote/.env" ;;
  *:*/compose.yml) cp -p "$2" "$UFOP_DEPLOY_FIXTURE/remote/compose.yml" ;;
  *) exit 2 ;;
esac
MOCK
cat > "$T/bin/gh" <<'MOCK'
#!/usr/bin/env bash
case "$*" in
  "api user --jq .login") echo fixture-user ;;
  "auth token") echo fixture-registry-token ;;
  *) exit 2 ;;
esac
MOCK
cat > "$T/bin/docker" <<'MOCK'
#!/usr/bin/env bash
[[ "$*" == "buildx imagetools inspect ghcr.io/clappediya/filemanager-admin:sha-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" ]]
MOCK
chmod +x "$T/bin/ssh" "$T/bin/rsync" "$T/bin/gh" "$T/bin/docker"
PATH="$T/bin:$PATH" UFOP_DEPLOY_FIXTURE="$T" DEPLOY_SSH_ALIAS=fixture-vps ADMIN_REMOTE_DIR=/opt/filemanager/admin \
  bash scripts/deploy-admin-ghcr.sh aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa "$T/bootstrap.env" >/dev/null
cmp "$T/bootstrap.env" "$T/remote/.env"
cmp docker-compose.admin.ghcr.yml "$T/remote/compose.yml"
node -e 'const fs=require("node:fs");for(const p of process.argv.slice(1))if((fs.statSync(p).mode&0o777)!==0o600)process.exit(1)' "$T/bootstrap.env" "$T/remote/.env"
echo "OK: first admin deploy copies exact runtime inputs with portable rsync and mode 0600"
