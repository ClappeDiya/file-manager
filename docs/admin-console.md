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
- **Credentials** (passphrases, cloud keys) are stored sealed and can be replaced but never read back. Each replacement is a new **version** (history: who and when, never the secret). Machines whose policies use a credential fetch the new version on their next heartbeat without their jobs being rewritten, and report the version they hold; the Credentials page shows how many machines have the current version and which are still waiting.
- **App settings** set update channel, update blackout window, proxy, AI limits and machine-wide transfer caps for every desktop (see `silent-install.md`).
- **Diagnostics**: machines upload scrubbed log bundles on request (machine page → Diagnostics) or from the app.
- **Webhooks** (Integrations → Webhooks): subscribe URLs to console events — backup results (`backup.success|warning|error|cancelled`), alerts (`alert.opened|resolved`) and every audit event type (`machine.register`, `credential.rotate`, …), by exact name, `prefix.*` or `*`. Each delivery is a JSON POST with `X-UFOP-Event`, `X-UFOP-Delivery`, `X-UFOP-Timestamp` and `X-UFOP-Signature: sha256=HMAC-SHA256(secret, "<timestamp>.<body>")`. Failed deliveries are retried after 1 min, 5 min, 30 min, 2 h and 6 h, then marked failed; after 15 failures in a row the webhook is paused until resumed. The delivery log shows status, attempts and responses, and any delivery can be sent again. Internal addresses (loopback, private, link-local) are refused unless `WEBHOOKS_ALLOW_PRIVATE=1`; redirects are not followed.
- **Passwords** (Settings → Security; My account → Password): new passwords need 12+ characters, three character classes or 16+ characters, no repeats/sequences, no common passwords, and not the user's name or address (a strength meter shows why). Organizations can require a new password after N days; accounts in several organizations follow the strictest. An expired password still has to be correct: the sign-in then leads to a 15-minute page to choose a new one. *Forgot your password?* e-mails a one-hour, single-use link through the organization's account-mail channel; the answer is the same whether or not the account exists, and requests are rate-limited. MFA still applies after a reset.
- **Passkeys** (My account → Passkeys; *Sign in with a passkey* on the sign-in page): WebAuthn credentials (Face ID/Touch ID, Windows Hello, security keys; ES256, EdDSA, RS256). The relying party is the console's public origin (`ADMIN_PUBLIC_URL` behind a proxy). A passkey used with a PIN or biometric counts as two-factor sign-in and satisfies a two-factor requirement; the last second factor cannot be removed while one is required. Wrong origins or sites, replayed challenges, bad signatures and signature counters that go backwards (copied keys) are refused and audited.
- **Sessions**: every sign-in creates a server-side session (at most 8 hours). Signing out ends it; My account → Sessions lists where the account is signed in and can end any of them; changing or resetting the password ends the others; administrators can sign members out everywhere (Users → bulk *sign out*), and deactivation does too. Settings → Security can sign out sessions idle for 5–480 minutes (strictest organization wins).
- **Step-up confirmation**: sensitive changes (security settings, single sign-on, SCIM tokens, central credentials, webhooks, removing members and changing roles, adding or removing passkeys) need a sign-in or confirmation within the last `STEP_UP_MINUTES` (default 10). Otherwise the console asks you to confirm — with your authenticator code or a passkey (with PIN/biometric) when the account has them, else your password; single sign-on accounts sign in again — and then repeats the change. Confirmations are audited as `auth.reauth` / `auth.reauth_failed` and rate-limited.
- **API tokens** (My account → API tokens): personal tokens for scripts, sent as `Authorization: Bearer ufop_cat_…` (no session cookie). Each has a name, an expiry (1–365 days) and a chosen subset of your permissions (read-only, everything your role allows, or a custom list); on every request it is further narrowed to what your role allows *now*, so demoting or removing you narrows or ends it, and deactivation revokes it. Tokens cannot change account or sign-in settings, mint tokens, switch organization or make step-up changes; the organization's allowed networks apply. Only a hash is stored; last use (time, address) is shown; administrators with user management can list (`?all=1`) and revoke members' tokens. Created/revoked tokens and every audited change made with one (metadata `apiToken`) are in the audit log.
- **Inactive access** (Settings → Security): with *Retire after N days* (30–3650; 0 = never), the background jobs remove members whose last activity — sign-in, session use or API token use, else when they joined — is older than N days (the account is disabled, its sessions and tokens revoked, when it belongs to no other organization) and revoke API tokens unused that long. Owners are never retired; access granted only through groups is managed on the group. Audited as `user.retired_inactive` / `api_token.retired_inactive`.
- **Allowed networks** (Settings → Security): restrict console access to IP addresses / CIDR ranges (IPv4 and IPv6). Enforced on every request and at sign-in (an account in several organizations lands in one that admits the address; otherwise the sign-in is refused and audited as `auth.login_failed`); signing out always works, and agent and SCIM endpoints are not affected. A list that excludes the address you save from is refused, so you cannot lock yourself out. The client address is the X-Forwarded-For entry added by your proxy: set `TRUSTED_PROXY_HOPS` to the number of proxies in front of the console (default 1) and never expose the console directly. `ufop-server` has the same control for its own clients: `--allowed-cidrs` / `UFOP_SERVER_ALLOWED_CIDRS` (matched against the TCP peer, so forwarded headers cannot bypass it).
- **Required two-factor sign-in** (Settings → Security): when on, every member must use an authenticator app. Until they enroll, their session (including sessions started before the rule) can only reach enrollment, session info and sign-out; the console sends them to My account. Optionally exempt single sign-on when the identity provider enforces MFA. Members of several organizations follow the strictest rule, and MFA cannot be turned off while required.
- **CSV user import** (Users → Import CSV): columns `email` (required), `role`, `group`, `group_role`. The server previews every row (invite, skip — already a member — or the reason it cannot be imported: invalid address, duplicate, unknown role or group, a role above yours), then invites the valid rows through the normal invitation flow (e-mail via the outbox when an invitation channel exists). Up to 1,000 rows / 1 MB; audited.
- **SCIM 2.0 provisioning** (Users → SSO & provisioning): create a token (shown once, with a default role for provisioned users — never an admin role) and give the identity provider the tenant URL `https://<console>/api/scim/v2`. Supported: `Users` and `Groups` (GET with `eq` filters on userName/externalId/emails/displayName/active/id and paging, POST, PUT, PATCH with the operation shapes Entra ID and Okta send, DELETE), `ServiceProviderConfig`, `ResourceTypes`, `Schemas`. `active: false` or DELETE removes the user from this organization (and disables the account when it belongs to no other); reactivation restores it with the default role. An existing account from another organization is linked rather than duplicated. SCIM never changes users who are owners/admins/security admins of the organization, nor groups created in the console (which may grant roles). Every change is audited as `scim:<token name>`.
- **Notification outbox** (Alerts → Deliveries): every alert, summary and invitation sent through a channel (email/SMTP, Slack, Teams, webhook) is recorded. The first attempt is immediate; failures are retried on the same schedule as webhooks and then marked failed. Messages can be retried now or discarded; sent and discarded ones are kept 30 days. Channel *Test* sends directly.
  - Policies reference them as `central:<id>`.
  - They are delivered only to authenticated agents that a policy targets, with `Cache-Control: no-store`.
  - Agents keep them in the OS keychain.
- **Alerts:**
  - Rules: backup failed (optionally on warnings), missed backup (schedule from the client, or an assumed interval, plus grace), backup not arrived by a set time (local time in an IANA time zone, chosen weekdays, grace; a successful backup must arrive between the previous deadline and this one), unusual changed or deleted file counts, unusual duration (median and standard deviation of earlier runs).
  - Open alerts can be **acknowledged** (they stay open, no further notifications) or **snoozed** for up to 30 days; both are audited and visible on the alert.
  - Rules are scoped by tags, use a cooldown and can notify on resolve.
  - Channels: SMTP email (STARTTLS/TLS, custom CA), Slack, Microsoft Teams (paste the URL of a "Post to a channel when a webhook request is received" workflow; messages arrive as Adaptive Cards), and signed webhooks (`X-UFOP-Signature: sha256=<HMAC of body>`).
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
