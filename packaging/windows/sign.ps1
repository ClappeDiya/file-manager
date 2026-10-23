# sign.ps1 — Authenticode-sign one file with signtool (Windows release builds).
# Used as Tauri's bundle.windows.signCommand by scripts/release-local.sh:
#   powershell -NoProfile -ExecutionPolicy Bypass -File packaging\windows\sign.ps1 <file>
# Certificate (first set wins):
#   WINDOWS_SIGN_CERT_THUMBPRINT                         a certificate in the store (e.g. a token / HSM)
#   WINDOWS_SIGN_CERT_FILE + WINDOWS_SIGN_CERT_PASSWORD  a .pfx file
# WINDOWS_SIGN_TIMESTAMP_URL (default http://timestamp.digicert.com), RFC 3161.
param([Parameter(Mandatory = $true)][string]$File)
$ErrorActionPreference = 'Stop'
$ts = if ($env:WINDOWS_SIGN_TIMESTAMP_URL) { $env:WINDOWS_SIGN_TIMESTAMP_URL } else { 'http://timestamp.digicert.com' }
$signtool = (Get-Command signtool.exe -ErrorAction SilentlyContinue).Source
if (-not $signtool) {
    $kits = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" -ErrorAction SilentlyContinue | Sort-Object FullName -Descending | Select-Object -First 1
    if ($kits) { $signtool = $kits.FullName } else { throw 'signtool.exe not found (install the Windows SDK)' }
}
$args = @('sign', '/fd', 'SHA256', '/td', 'SHA256', '/tr', $ts)
if ($env:WINDOWS_SIGN_CERT_THUMBPRINT) {
    $args += @('/sha1', $env:WINDOWS_SIGN_CERT_THUMBPRINT)
} elseif ($env:WINDOWS_SIGN_CERT_FILE) {
    $args += @('/f', $env:WINDOWS_SIGN_CERT_FILE)
    if ($env:WINDOWS_SIGN_CERT_PASSWORD) { $args += @('/p', $env:WINDOWS_SIGN_CERT_PASSWORD) }
} else {
    throw 'Set WINDOWS_SIGN_CERT_THUMBPRINT, or WINDOWS_SIGN_CERT_FILE and WINDOWS_SIGN_CERT_PASSWORD'
}
& $signtool @args $File
if ($LASTEXITCODE -ne 0) { throw "signtool failed for $File" }
& $signtool verify /pa $File | Out-Null
if ($LASTEXITCODE -ne 0) { throw "signature check failed for $File" }
