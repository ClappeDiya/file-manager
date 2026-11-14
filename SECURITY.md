# Security Policy

We take the security of FileManager (UFOP) seriously, especially the parts that
handle your backups, credentials and encryption keys. Thank you for helping keep
our users safe.

## Reporting a vulnerability

**Please do not report security vulnerabilities through public GitHub issues,
discussions or pull requests.**

Report them privately through GitHub's private vulnerability reporting:

1. Go to <https://github.com/ClappeDiya/file-manager/security/advisories/new>
   (or the repository's **Security** tab → **Report a vulnerability**).
2. Fill in the advisory form. Only you and the maintainers can see it.

This creates a draft GitHub Security Advisory where we can discuss the issue,
collaborate on a fix in a private fork and request a CVE when appropriate.

### What to include

The more of the following you can share, the faster we can help:

- The affected component (see [Scope](#scope)) and version, plus your OS and
  architecture.
- The type of issue (e.g. path traversal, authentication bypass, key or
  passphrase disclosure, remote code execution, repository corruption).
- Step-by-step instructions to reproduce, including any configuration,
  sample files or repository layout required.
- A proof of concept or exploit code, if you have one.
- The impact as you understand it: what an attacker can do and under which
  preconditions (local user, network position, authenticated or not).
- Any suggested fix or mitigation.

Please never include real secrets, passphrases or other people's data in a
report. Redact logs where needed.

## Supported versions

The current release line is **0.1.x**. Security fixes are made for the
**latest minor release** only; older releases do not receive patches, so
please upgrade before reporting if you can.

| Version             | Supported |
| ------------------- | --------- |
| Latest minor (0.1.x) | Yes      |
| Anything older      | No        |

## Response targets

| Step                         | Target                                   |
| ---------------------------- | ---------------------------------------- |
| Acknowledge your report      | Within 3 business days                   |
| Triage (confirm and rate severity) | Within 7 days of the report        |
| Fix released — Critical      | As fast as possible, aiming for 7 days   |
| Fix released — High          | Within 30 days                           |
| Fix released — Medium        | Within 90 days                           |
| Fix released — Low           | In a regular release, as scheduled       |

Severity follows CVSS v3.1. We will keep you informed of progress at least at
each step above and tell you if a target is going to slip.

## Scope

In scope:

- **Desktop app** — the Tauri application (`src/`, `src-tauri/`), including its
  IPC commands, backup/sync/transfer engines and protocol connectors.
- **ufop-server** — the headless server and its REST API (`src-tauri/src/server/`,
  `ufop-server` binary).
- **CLI** — the `ufop` command-line tool (`cli/`).
- **Admin console** — the Next.js admin console and customer portal (`admin/`).
- **ufop-core** — the shared engine crate (`crates/ufop-core/`), including the
  backup repository format, encryption and restore code.

Out of scope:

- Vulnerabilities in third-party services, cloud providers or storage backends
  themselves (report those to the vendor), and in upstream dependencies unless
  UFOP uses them in an exploitable way.
- Attacks that require an already compromised machine, root/administrator
  access, or physical access to an unlocked device.
- Social engineering, phishing and attacks on project maintainers' accounts.
- Denial of service through resource exhaustion on a machine you control, or
  volumetric attacks against hosted infrastructure.
- The marketing website and other content that does not process user data.
- Findings from automated scanners without a demonstrated, practical impact,
  missing best-practice headers on static pages, and self-XSS.
- Issues in unsupported versions or in modified builds.

## Coordinated disclosure

- Please give us a reasonable chance to fix the issue before disclosing it
  publicly. We ask for up to **90 days** from your report, or until a fix is
  released, whichever comes first; we may agree a different date with you for
  complex issues.
- We will publish a GitHub Security Advisory when the fix ships, describing the
  issue, affected versions and how to upgrade, and request a CVE where it applies.
- We credit reporters in the advisory and the CHANGELOG unless you prefer to
  stay anonymous.
- We will not take legal action against good-faith research that follows this
  policy: avoid privacy violations, data destruction and service disruption,
  only test against accounts and data you own, and stop and report as soon as
  you find a problem.
