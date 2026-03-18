# AI Agent Pack — Unified File Operations Platform

## Loading Instructions

### For any task:
1. Load `global-policy.yaml` first
2. Load the specific agent YAML for the task
3. For R3/R4 risk tasks, also load `core/test-and-qa.yaml` + `core/security-review.yaml`

### Example: Build OneDrive Connector
```
global-policy.yaml
core/connector-protocol.yaml
core/test-and-qa.yaml
core/security-review.yaml
core/documentation.yaml
```

### Example: Build Conflict Resolution UI
```
global-policy.yaml
core/desktop-ui.yaml
core/sync-engine.yaml
core/compatibility-engine.yaml
core/test-and-qa.yaml
```

### Example: Prepare Release Candidate
```
global-policy.yaml
core/release-validation.yaml
support/ci-devops.yaml
core/test-and-qa.yaml
core/security-review.yaml
core/documentation.yaml
```

## Agent Roster

### Core Agents (11)
| Agent | File | Mission |
|-------|------|---------|
| Orchestrator | `core/orchestrator.yaml` | Route work, enforce boundaries, track blockers |
| Rust Core Architect | `core/rust-core-architect.yaml` | Crate architecture, IPC, persistence, privilege separation |
| Connector Protocol | `core/connector-protocol.yaml` | All 14+ connectors: auth, retry, resume |
| Sync Engine | `core/sync-engine.yaml` | Sync modes, conflict handling, dry-run, rollback |
| Compatibility Engine | `core/compatibility-engine.yaml` | Naming/path normalization, mapping, restoration |
| Desktop UI | `core/desktop-ui.yaml` | Tauri + React/Vite, Simple/Advanced modes |
| Admin Console | `core/admin-console.yaml` | Next.js admin: RBAC, policies, audit, billing |
| Test & QA | `core/test-and-qa.yaml` | Independent validation, certification, regression |
| Security Review | `core/security-review.yaml` | Credentials, auth, encryption, privilege boundaries |
| Documentation | `core/documentation.yaml` | User/admin/API/CLI docs, release notes |
| Release Validation | `core/release-validation.yaml` | Final gatekeeper: merge/beta/GA readiness |

### Support Agents (8)
| Agent | File | Mission |
|-------|------|---------|
| CI/DevOps | `support/ci-devops.yaml` | Pipelines, builds, signing, installers |
| Maintenance | `support/maintenance-regression.yaml` | Post-release stability, regression triage |
| Dependency/License | `support/dependency-license-compliance.yaml` | SBOM, CVEs, license audit |
| Observability | `support/observability-telemetry.yaml` | Logging, metrics, crash reporting |
| Product Consistency | `support/product-consistency.yaml` | Simple mode, tier, monetization alignment |
| API/CLI | `support/api-cli.yaml` | CLI commands, API contracts, automation |
| Terminal | `support/terminal-shell-integration.yaml` | Embedded terminal, PTY, shell sessions |
| Migration | `support/migration-workflow.yaml` | Computer/drive migration flows |

## Risk Classes
- **R0:** Docs only
- **R1:** Low-risk UI/display
- **R2:** Normal functional behavior
- **R3:** Security/auth/connectors/persistence
- **R4:** Destructive/data-integrity-critical
