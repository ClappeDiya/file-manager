# Documentation Index

## Product Specifications

| Document | Location | Description |
|----------|----------|-------------|
| PRD V5 (Markdown) | `../PRD-V5-Definitive.md` | Master PRD - all 32 sections |
| PRD V5 (Word) | `../PRD-V5-Definitive-Unified-File-Operations-Platform.docx` | Formatted Word document |
| PRD V5 (PDF) | `../PRD-V5-Definitive-Unified-File-Operations-Platform.pdf` | PDF version |
| AI-Agent Governance | `AI-Agent-Governance-Addendum.md` | Agent governance spec |
| Tech Stack Detail | `tech-stack-v4.1.md` | V4.1 revised stack rationale |

## Planning Documents

| Document | Location | Description |
|----------|----------|-------------|
| Sprint Plan | `../planning/sprint-plan-summary.md` | 42 sprints across 8 phases |
| Epics Summary | `../planning/epics-summary.md` | 47 epics mapped to sprints |

## Operations

| Document | Location | Description |
|----------|----------|-------------|
| Admin console | `admin-console.md` | Console setup, fleet, policies |
| Silent install & enrollment | `silent-install.md` | Unattended installs, preload enrollment, managed app settings |

## Using the app

| Document | Location | Description |
|----------|----------|-------------|
| User guide | `user-guide.md` | Desktop app, portable mode, settings |
| CLI reference | `cli-reference.md` | Every `ufop` command and flag |
| REST API reference | `api-reference.md` | `ufop-server` REST API (`/api/v1`) |
| IPC commands | `ipc-commands.md` | Tauri commands of the desktop app (generated) |

## Backups

| Document | Location | Description |
|----------|----------|-------------|
| Repository format | `backup-repository-format.md` | Format 1 and 2, upgrade, password change, resharding, shared repositories |
| Repository safety | `backup-safety.md` | Locks, clock check, checkpoints, read-only connections, encrypted local database |
| Sources and runs | `backup-sources-and-runs.md` | What a backup reads and how a run behaves |
| Versions | `backup-versions.md` | Notes, tags, pins, retention preview, copies |
| Restore | `backup-restore.md` | Restore as of a time, into archives, mirror restore, preview, open as a drive |
| Policies and schedules | `backup-policies-and-schedules.md` | Templates, effective settings, cron, time zones, maintenance |
| Notifications | `backup-notifications.md` | Profiles, webhooks and presets, HTML email |
| Ransomware hardening | `backup-ransomware-hardening.md` | Least privilege, object lock, append-only SFTP, soft delete, legal hold |
| Destinations | `backup-destinations.md` | Local, cloud and replica destinations |
| Encryption and secrets | `backup-encryption-and-secrets.md` | Encryption methods, certificate pinning, secret providers |
| Logging and support | `backup-logging-and-support.md` | Logs, diagnostics and support tools |
| Photos libraries | `backup-photos.md` | Exporting a macOS Photos library |

## Server

| Document | Location | Description |
|----------|----------|-------------|
| Server operations | `server-operations.md` | Running `ufop-server`: shutdown, security, metrics, tracing, branding |

## Agent Configurations

See `../ai-agents/README.md` for the full agent pack documentation.

## Document Hierarchy

```
PRD V5 (Definitive)            - What to build and why
AI-Agent Governance Addendum   - How agents build it safely
Engineering Epics              - Detailed implementation spec
Sprint Plan                    - When to build it
Agent Pack (YAML files)        - Machine-readable agent configs
```
