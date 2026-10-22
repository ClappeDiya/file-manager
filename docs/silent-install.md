# Silent installation and fleet enrollment

This guide covers deploying the desktop app to many machines without user
interaction, enrolling each one with the management console, and controlling
app settings centrally.

## 1. Install silently

The release produces one installer per platform (Tauri bundle targets).

| Platform | Installer | Silent command |
|---|---|---|
| Windows | `.msi` | `msiexec /i "Unified File Operations Platform_<ver>_x64_en-US.msi" /qn /norestart` |
| Windows | NSIS `-setup.exe` | `"Unified File Operations Platform_<ver>_x64-setup.exe" /S` |
| macOS | `.dmg` (app bundle) | `hdiutil attach UFOP.dmg -nobrowse -quiet && cp -R "/Volumes/UFOP/Unified File Operations Platform.app" /Applications/ && hdiutil detach "/Volumes/UFOP" -quiet` |
| Debian/Ubuntu | `.deb` | `sudo DEBIAN_FRONTEND=noninteractive apt-get install -y ./ufop_<ver>_amd64.deb` |
| Fedora/RHEL | `.rpm` | `sudo dnf install -y ./ufop-<ver>.x86_64.rpm` |
| Linux (other) | `.AppImage` | copy it to `/opt/ufop/` and `chmod 755` it |

Use your software-distribution tool (Intune, SCCM, Jamf, Munki, Ansible…) to
run these commands. They need administrator rights because the preload file
below lives in a machine-wide location.

## 2. Enroll with the console (preload file)

In the console, open **Enrollment**, create a registration link (set an
expiry, a use limit and default tags) and download its **preload file**. It
looks like this:

```json
{
  "env": {
    "*": {
      "UFOP_CONSOLE_URL": "https://console.example.com",
      "UFOP_CONSOLE_REGISTRATION_TOKEN": "ufop_reg_…",
      "UFOP_CONSOLE_TAGS": "laptop,finance"
    }
  }
}
```

Copy it to the machine-wide location:

| Platform | Path |
|---|---|
| Windows | `%PROGRAMDATA%\ufop\preload.json` |
| macOS | `/Library/Application Support/ufop/preload.json` |
| Linux | `/etc/ufop/preload.json` |

`$UFOP_PRELOAD_SETTINGS` overrides the path. On macOS and Linux the file is
ignored when group or other users can write it: install it as `root:root`
(or `root:wheel`) with mode `0644`. The token is a registration secret, so
restrict the file further (`0640`, readable by the users who run the app)
where your policy requires.

On first start the app registers itself with the link, stores its agent
token in the OS keychain and never reads the registration token again. If a
user explicitly leaves the console (Settings → Managed by Console → Unregister), automatic registration stays off on that machine.

The same values can come from environment variables instead of the file:

| Variable | Meaning |
|---|---|
| `UFOP_CONSOLE_URL` | Console address (`https://…`) |
| `UFOP_CONSOLE_REGISTRATION_TOKEN` | Registration link token (`ufop_reg_…`) |
| `UFOP_CONSOLE_TAGS` | Comma-separated tags added to the link's default tags |
| `UFOP_CONSOLE_HEARTBEAT_SECONDS` | Heartbeat interval override (15–3600) |
| `UFOP_CONSOLE_CA_FILE` | PEM file with the CA of a console behind a private CA |

Variables already set in the environment win over the preload file.

### Interactive alternatives

- **Sign in with browser** (Settings → Managed by Console): the app opens the
  console, the user signs in (password, SSO or MFA) and approves. Needs the
  *enroll machines* permission. The redirect goes to `127.0.0.1` only and the
  code is bound to the app with PKCE.
- **Claim code**: the app shows a code, an administrator enters it under
  Enrollment → Claim.

## 3. Manage app settings

Under **App settings** in the console an organization can set, for all of its
desktops:

- **Updates**: the channel (stable or beta), whether the app checks
  automatically, and a weekly **blackout window** (machine local time, e.g.
  Mon–Fri 08:00–18:00) during which no update is downloaded or installed.
- **Proxy**: system, none, manual or a PAC file, with bypass hosts. The proxy
  is then read-only in the app's Settings and any saved proxy password is
  removed.
- **AI**: turn AI features off, or keep AI on local models only.
- **Transfers**: total upload and download rate for all backups and restores
  on a machine together, and how many operations may run at the same time
  (1–8; the same job never runs twice at once). Each one replaces the
  machine's own setting (Backup → Settings → Machine-wide limits).

Every setting left on *User's choice* stays with the user. Desktops pick up a
change on their next heartbeat and keep the last settings while offline. The
page shows how many desktops applied the current version; failures appear in
the audit log as `policy.apply_failed`. *Stop managing* gives every setting
back to the users, as does unregistering a machine.

## 4. Limits the machine sets on the console

The person (or IT image) controlling a machine can limit what the console may
do there, under Settings → Managed by Console → *What the console may do on
this machine*. These limits are local: the console cannot see or change them.

- **Allowed folders**: job sources, local destinations and restore targets
  sent by the console must be inside one of them (empty = anywhere).
- **Symbolic links**: by default a path must really point inside an allowed
  folder (links are resolved); *Judge the path as written* skips that.
- **Read-only**: the console can view jobs and run backups and checks, but
  cannot save or delete jobs, change defaults, restore files or delete backup
  data. Fleet policies are reported as failed on such machines.
- **Scripts**: untick to refuse jobs from the console that carry pre/post
  scripts (commands that would run on this machine).

Refused calls fail with a clear reason in the console's command history and
policy status; nothing is changed on the machine. If the saved limits cannot
be read, the agent refuses all changes until they are saved again.

## 5. Verify

- The machine appears under **Machines** with the link's tags.
- Settings → Managed by Console on the machine shows the console, the
  organization and a box listing what the organization manages.
- Settings → Logging → *Send to console* uploads a diagnostics bundle the
  administrator can download from the machine's **Diagnostics** tab, or the
  administrator can request one from there.
