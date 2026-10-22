# Admin console: fleet, tenancy and enterprise

The admin console (`admin/`, Next.js) manages backup clients centrally. It is the UFOP counterpart of the Duplicati Console. This page covers how to run it, its main concepts and the agent protocol clients speak.

## Running

```bash
pnpm admin:dev                    # development, port 3001
cd admin && npx next build && npx next start -p 3001
```

| Variable | Purpose |
|---|---|
| `NEXTAUTH_SECRET` | **Required.** Signs session cookies. Also the fallback key for sealing secrets. |
| `ADMIN_ENCRYPTION_KEY` | Key for sealing stored secrets (AES-256-GCM): channel passwords, SSO client secrets, central credentials, the AI API key, TOTP seeds. Defaults to HKDF(`NEXTAUTH_SECRET`). Set it explicitly in production. |
| `ADMIN_DB_PATH` | SQLite file (default `./admin-data.db`). Migrations run on start (`PRAGMA user_version`). |
| `ADMIN_PUBLIC_URL` | External URL behind a proxy. Used for SSO callbacks, invitation links, registration links and preload files. |
| `ADMIN_EMAIL`, `ADMIN_PASSWORD` | Bootstrap owner of the default organization, created on first sign-in to an empty database. |
| `ADMIN_DISABLE_JOBS` | Turns off the 5-minute background jobs: missed-backup alerts, scheduled AI summaries, offboarding purges. |

## Concepts

- **Organizations** form a tree. Owners and admins of a parent also manage its children (delegation). A **detached** child is invisible to and unmanaged by its ancestors.
  - **Protected** organizations cannot be offboarded.
  - **Offboarding** revokes all access and agent tokens immediately. Data is purged after a grace period (0–90 days).
  - Licenses (plan, machine seats) are set by the parent and inherited by children that don't set their own. Registration beyond the seats fails with 402.
- **Roles** per organization: `org_owner`, `org_admin`, `operator`, `user`, `read_only_auditor`. A role comes from a direct membership, a parent organization (delegated) or a group. The server recomputes it on every request, so changes apply at once.
- **Groups** grant an organization role and, optionally, access to specific machines, tags or backups.
  - Members with the `user` role see only what their groups grant.
  - Group owners add and remove members and send invitations.
- **Invitations** are single-use links that expire after 7 days. They are sent through the email channel marked *use for invitations*, or copied by hand.
- **Single sign-on:**
  - **OIDC:** authorization code + PKCE, with the ID token verified against the issuer's JWKS, and claims also taken from userinfo.
  - **SAML 2.0:** signed assertions required, InResponseTo checked, SP metadata at `/api/auth/sso/<id>/metadata`.
  - Providers claim email domains. The sign-in page shows them for matching addresses, and existing accounts in those domains are linked on first sign-in.
  - Accounts can be created on first sign-in with a default role (never owner), a default group, and groups mapped from the IdP's group claim.
  - People link and unlink providers on **My account**.
- **Two-factor sign-in:** TOTP (RFC 6238) with replay protection and 10 single-use recovery codes. A provider can require it on top of SSO. Turning it off needs a current code.
- **Audit log:** append-only (enforced by SQLite triggers) and hash-chained (SHA-256), with HMAC-signed checkpoints. **Audit → Verify integrity** recomputes the whole chain.

## Fleet

- **Enrollment** has three paths:
  1. A **registration link** token for unattended installs: `ufop agent register --url … --token ufop_reg_…`, or a downloadable `preload.json`.
  2. A **claim code**: the machine runs `ufop agent claim` and shows `XXXX-XXXX`, which an admin enters on **Enrollment**.
  3. A **report URL** for clients that only send reports (`/api/ingest/<token>`, JSON or form-encoded).
- **Machines** carry tags (with organization-wide colors), notes, connectivity (online / stale / offline from heartbeats), backups, reports, alerts and policy compliance. Machines can be suspended, revoked, deleted, or have their token rotated (the agent rotates itself on its next check-in).
- **Remote session:** queue allow-listed client calls (`backup_*`), runs, policy applies and token rotation. Agents pick commands up on their next heartbeat and post the result back. Calls that read secrets back or touch arbitrary files are not allowed.
- **Fleet policies** push a backup job template to every machine with the targeted tags. Machines create, update and remove the managed job (`policy-<id>`) and acknowledge the version they applied.
- **Credentials** (passphrases, cloud keys) are stored sealed and can be replaced but never read back.
  - Policies reference them as `central:<id>`.
  - They are delivered only to authenticated agents that a policy targets, with `Cache-Control: no-store`.
  - Agents keep them in the OS keychain.
- **Alerts:**
  - Rules: backup failed (optionally on warnings), missed backup (schedule from the client, or an assumed interval, plus grace), unusual changed or deleted file counts, unusual duration (median and standard deviation of earlier runs).
  - Rules are scoped by tags, use a cooldown and can notify on resolve.
  - Channels: SMTP email (STARTTLS/TLS, custom CA), Slack, and signed webhooks (`X-UFOP-Signature: sha256=<HMAC of body>`).
- **AI insights:** organization-level Anthropic API key (sealed), on-demand analysis of a machine or the whole fleet, and daily or weekly summaries sent to channels. Only report metadata is sent; file names, paths and secrets are not.

## Agent protocol (summary)

All agent calls carry `Authorization: Bearer ufop_agent_…`, except registration and claiming.

| Call | Purpose |
|---|---|
| `POST /api/agent/register` `{registrationToken, info, tags}` | Returns `{machineId, agentToken, heartbeatSeconds}`. |
| `POST /api/agent/claim` `{info}`, then `POST /api/agent/claim/poll` `{claimId, secret}` | Returns the token once the claim is approved. |
| `POST /api/agent/heartbeat` `{info, jobs, policySetVersion}` | Returns pending `commands`, the heartbeat interval and whether policies changed. |
| `GET /api/agent/policies`, `POST /api/agent/policies/applied` | Fetch managed jobs and credentials; acknowledge versions. |
| `POST /api/agent/report` | Run result, plus `stats {version_count, backend_size, source_size, schedule_interval_minutes}`. |
| `POST /api/agent/commands/<id>` `{ok, result}` | Complete a remote command. |
| `POST /api/agent/rotate` | Rotate the token. |
| `POST /api/agent/unregister` | Unregister the machine. |

## Tests

`cd admin && npx vitest run` covers:

- migrations, sessions, audit;
- tenancy and isolation;
- the agent protocol and fleet;
- alerts over a real SMTP server and local Slack/webhook endpoints;
- AI against a local Messages-API stream;
- SSO against a live `oidc-provider` and SAML responses signed by a samlify IdP.

The SMTP cases expect the local aiosmtpd server on `127.0.0.1:2525` (`UFOP_TEST_SMTP_DIR`, `UFOP_TEST_SMTP_CERT`).
