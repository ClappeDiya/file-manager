# PRD V5 Addendum: AI-Agent Governance, Delivery, Review, and Maintenance Specification

**Version:** 1.0  
**Applies to:** All AI agents building, reviewing, testing, securing, and maintaining the Unified File Operations Platform  
**Companion to:** PRD V5 (Definitive)

---

## 1. Goals of the AI-Agent Operating Model

1. **Fast implementation** across all subsystems
2. **Strict separation of concerns** — no single agent owns implementation + testing + security + release approval
3. **Evidence-based completion** — code, tests, logs, artifacts required
4. **Cross-platform correctness** — Windows, macOS, Linux validation
5. **No unsafe automation** — no silent destructive behavior or governance bypass
6. **Auditability** — every action traceable with rollback guidance
7. **Maintainability** — codebase remains understandable after AI changes

## 2. Non-Goals

- Unrestricted autonomous pushes to production
- Replacing all human judgment in critical security workflows
- Agents redefining product scope without authorization
- Accepting "works on my machine" as proof of correctness

## 3. Core Governance Principles

- **Source of truth:** PRD V5 > ADRs > API contracts > Test specs > Production behavior > Agent proposals
- **No direct main-branch pushes** — all work through isolated branches + review + validation
- **Separation of duties** — writer != sole validator != sole release approver
- **Evidence over claims** — files, tests, results, risk class, rollback note required
- **No silent requirement drift** — scope/criteria/security changes must be explicit
- **Safe by default** — prefer non-destructive, reversible, logged behavior

## 4. Risk Classification

| Class | Description | Approval Minimum |
|-------|-------------|-----------------|
| R0 | Docs / non-executable | Documentation Agent + QA check |
| R1 | Low-risk UI / display | Implementing agent + QA |
| R2 | Functional behavior | Implementing + QA + peer specialist |
| R3 | Security / auth / persistence | Implementing + QA + Security + Release Validation |
| R4 | Destructive / data-integrity | All R3 reviewers + Orchestrator/human approval |

## 5. Required Workflow (All Tasks)

1. **Task intake** — ID, PRD refs, goal, risk class, acceptance criteria
2. **Plan** — approach, files affected, dependencies, risks, test plan, rollback
3. **Implement** — within declared scope only
4. **Self-check** — compile, lint, tests pass, no obvious regression
5. **Specialist review** — architecture, compat, UI, security as applicable
6. **QA validation** — execution, negative testing, regression, cross-platform
7. **Security validation** — all R3/R4 changes
8. **Documentation update** — all relevant docs
9. **Release readiness** — BLOCKED / READY FOR MERGE / READY FOR RELEASE

## 6. Mandatory Handoff Manifest

Every agent handoff must include: task_id, PRD references, risk class, owner agent, branch, files changed, commands run, tests added, tests passed, known limitations, security implications, rollback instructions, required next reviewer.

## 7. Merge Policy

- No self-approval
- Build + lint + tests + manifest + docs required before merge
- R3/R4: QA + Security + Release Validation signoff required
- Blocked by: missing tests, missing rollback, secret exposure, destructive path without review

## 8. Release Policy

Release requires: no P0 bugs, no data-loss defects, no destructive-sync defects, credential storage reviewed, installers validated, docs complete, rollback path exists, observability functional.

## 9. Launch Gates (Product-Specific)

1. No unresolved P0 or critical security issues
2. No unresolved data-loss defects
3. No unresolved destructive sync/delete defects
4. Compatibility engine passes defined corpus tests
5. Credential storage reviewed and approved
6. Installer/update flows pass on all supported OSes
7. Docs complete for general users and admins
8. Rollback path defined
9. Crash/error reporting functional
10. Known risks explicitly documented

## 10. Red-Line Prohibitions (All Agents)

- Direct push to production
- Bypassing QA/security for risky changes
- Logging secrets
- Weakening encryption or auth
- Removing compatibility mappings without migration
- Silently changing destructive behavior
- Silently changing monetization/tier boundaries
- Fabricating test results
- Claiming support for a feature not truly working
- Suppressing known blockers from release reports

## 11. Definition of Done

A feature is only "done" when ALL are true:
1. Requirement traced to PRD
2. Code implemented
3. Tests added/updated
4. Platform coverage executed
5. Security review completed if required
6. Documentation updated
7. Observability adequate
8. Rollback note present
9. QA evidence attached
10. Release Validation status recorded

## 12. Agent Roster

### Core Agents (11)
See `ai-agents/core/` for full YAML definitions:
- Orchestrator, Rust Core Architect, Connector Protocol, Sync Engine, Compatibility Engine, Desktop UI, Admin Console, Test & QA, Security Review, Documentation, Release Validation

### Support Agents (8)
See `ai-agents/support/` for full YAML definitions:
- CI/DevOps, Maintenance & Regression, Dependency/License, Observability, Product Consistency, API/CLI, Terminal/Shell, Migration Workflow

## 13. Maintenance Model

- All hotfixes require QA
- Security hotfixes require Security review
- Compatibility/sync hotfixes require regression tests
- Release channels: canary > beta > stable > emergency hotfix
- No maintenance agent deploys directly to stable without channel gates
