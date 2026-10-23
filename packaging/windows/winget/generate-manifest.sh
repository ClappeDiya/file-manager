#!/usr/bin/env bash
# generate-manifest.sh — winget manifests (schema 1.6, multi-file) for a
# FileManager Windows release.
#
# Writes <Id>.yaml (version), <Id>.installer.yaml and <Id>.locale.en-US.yaml
# into --out. Submit them to microsoft/winget-pkgs under
# manifests/<first letter>/<Publisher>/<Name>/<version>/ (e.g. with
# `wingetcreate submit <dir>`), or validate with `winget validate <dir>`.
#
# Usage:
#   packaging/windows/winget/generate-manifest.sh --version 1.2.0 \
#       --installer-url https://github.com/ClappeDiya/file-manager/releases/download/v1.2.0/FileManager-1.2.0-x64-setup.exe \
#       [--sha256 <hex> | --installer-file FileManager-1.2.0-x64-setup.exe] \
#       [--installer-type nsis|wix] [--msi-url URL --msi-sha256 HEX] \
#       [--id ClappeDiya.FileManager] [--out DIR]

set -euo pipefail

VERSION=""
URL=""
SHA=""
FILE=""
TYPE="nsis"
MSI_URL=""
MSI_SHA=""
ID="ClappeDiya.FileManager"
OUT="."

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version) VERSION="$2"; shift 2 ;;
    --installer-url) URL="$2"; shift 2 ;;
    --sha256) SHA="$2"; shift 2 ;;
    --installer-file) FILE="$2"; shift 2 ;;
    --installer-type) TYPE="$2"; shift 2 ;;
    --msi-url) MSI_URL="$2"; shift 2 ;;
    --msi-sha256) MSI_SHA="$2"; shift 2 ;;
    --id) ID="$2"; shift 2 ;;
    --out) OUT="$2"; shift 2 ;;
    -h|--help) sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "generate-manifest: unknown argument: $1" >&2; exit 2 ;;
  esac
done

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}
[[ -z "$SHA" && -n "$FILE" ]] && SHA="$(sha256_of "$FILE")"
[[ -n "$VERSION" && -n "$URL" && -n "$SHA" ]] || { echo "generate-manifest: --version, --installer-url and --sha256 (or --installer-file) are required" >&2; exit 2; }
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.]+)?$ ]] || { echo "generate-manifest: invalid version '$VERSION'" >&2; exit 2; }
case "$TYPE" in nsis|wix) ;; *) echo "generate-manifest: --installer-type must be nsis or wix" >&2; exit 2 ;; esac
SHA="$(printf '%s' "$SHA" | tr 'a-f' 'A-F')"
[[ "$SHA" =~ ^[0-9A-F]{64}$ ]] || { echo "generate-manifest: the SHA-256 must be 64 hex digits" >&2; exit 2; }
if [[ -n "$MSI_URL" ]]; then
  MSI_SHA="$(printf '%s' "$MSI_SHA" | tr 'a-f' 'A-F')"
  [[ "$MSI_SHA" =~ ^[0-9A-F]{64}$ ]] || { echo "generate-manifest: --msi-sha256 must be 64 hex digits" >&2; exit 2; }
fi

PUBLISHER="${ID%%.*}"
mkdir -p "$OUT"
SCHEMA="https://aka.ms/winget-manifest"
MV="1.6.0"

cat > "$OUT/$ID.yaml" <<YAML
# yaml-language-server: \$schema=${SCHEMA}.version.${MV}.schema.json
PackageIdentifier: ${ID}
PackageVersion: ${VERSION}
DefaultLocale: en-US
ManifestType: version
ManifestVersion: ${MV}
YAML

{
  cat <<YAML
# yaml-language-server: \$schema=${SCHEMA}.installer.${MV}.schema.json
PackageIdentifier: ${ID}
PackageVersion: ${VERSION}
MinimumOSVersion: 10.0.17763.0
Scope: user
UpgradeBehavior: install
ReleaseDate: $(date -u +%Y-%m-%d)
Installers:
- Architecture: x64
  InstallerType: ${TYPE}
  InstallerUrl: ${URL}
  InstallerSha256: ${SHA}
YAML
  if [[ "$TYPE" == nsis ]]; then
    cat <<'YAML'
  InstallerSwitches:
    Silent: /S
    SilentWithProgress: /S
YAML
  fi
  if [[ -n "$MSI_URL" ]]; then
    cat <<YAML
- Architecture: x64
  InstallerType: wix
  Scope: machine
  InstallerUrl: ${MSI_URL}
  InstallerSha256: ${MSI_SHA}
YAML
  fi
  cat <<YAML
ManifestType: installer
ManifestVersion: ${MV}
YAML
} > "$OUT/$ID.installer.yaml"

cat > "$OUT/$ID.locale.en-US.yaml" <<YAML
# yaml-language-server: \$schema=${SCHEMA}.defaultLocale.${MV}.schema.json
PackageIdentifier: ${ID}
PackageVersion: ${VERSION}
PackageLocale: en-US
Publisher: ${PUBLISHER}
PublisherUrl: https://github.com/ClappeDiya
PackageName: FileManager
PackageUrl: https://filemanager.clappe.com
License: Proprietary
LicenseUrl: https://github.com/ClappeDiya/file-manager/blob/main/LICENSE
ShortDescription: Cross-platform file manager with transfers, sync, versioned encrypted backups and governance.
Moniker: filemanager
Tags:
- backup
- file-manager
- sync
- sftp
- s3
ReleaseNotesUrl: https://github.com/ClappeDiya/file-manager/releases/tag/v${VERSION}
ManifestType: defaultLocale
ManifestVersion: ${MV}
YAML

echo "  + winget manifests for ${ID} ${VERSION} in ${OUT}"
