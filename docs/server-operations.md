# Operating ufop-server

How to run the headless backup server (`ufop-server`) in production: stopping it safely, protecting
browser sessions, accounts and access rules, API tokens, request logs, metrics and traces. For the
REST API itself see [api-reference.md](api-reference.md#ufop-server-rest-api-apiv1); for the CLI see
[cli-reference.md](cli-reference.md).

## Graceful shutdown

The server stops cleanly when it receives **SIGTERM** or **SIGINT** (Ctrl-C, also on Windows), when the
**Windows service** is stopped (or the machine shuts down), or when an administrator calls
`POST /api/v1/server/shutdown` (`ufop server shutdown`). It then:

1. answers new state-changing requests (`POST`, `PUT`, `PATCH`, `DELETE`) with **`503 Service Unavailable`**
   and a `Retry-After` header; reads keep working until the listener closes;
2. stops the scheduler from starting anything and refuses new backups and operations;
3. asks every running operation to stop at its next safe point. A backup stopped this way writes a
   **partial version** with everything uploaded so far (the next run continues from it) and releases its
   repository lock;
4. waits up to the **grace period** — 60 seconds by default, `--shutdown-grace <secs>`
   (`UFOP_SERVER_SHUTDOWN_GRACE`), or `grace_secs` in the API request;
5. closes open connections (up to 10 more seconds), sends the last metrics push and the remaining
   trace spans, and exits with status 0.

A **second** signal or shutdown request while draining stops at once: running operations are cancelled
without waiting. An operation that did not reach a safe point leaves its lock behind; locks expire on their
own, or can be removed with `DELETE /api/v1/jobs/{id}/lock?confirm=true`.

```bash
ufop server shutdown --grace 120      # wait up to two minutes for running backups
systemctl stop ufop-server            # SIGTERM; set TimeoutStopSec= above the grace period
```

The packaged systemd unit and launchd plist send SIGTERM. With systemd, set `TimeoutStopSec` to more than
`--shutdown-grace` plus 15 seconds, or systemd kills the process first. The Windows service reports
*Stop pending* with a wait hint of the grace period plus 15 seconds.

The desktop app does the same when it quits (tray *Quit* or closing the window with *Keep running in the
tray* off): running backups get up to 60 seconds to write their partial version before backup drives are
closed and the app exits.

## Browser sessions and CSRF

Signing in (`POST /api/v1/auth/login`) returns a short-lived access token and sets two cookies:

| Cookie | Flags | Purpose |
|---|---|---|
| `ufop_refresh` | `HttpOnly; SameSite=Strict; Path=/api/v1/auth` (+ `Secure` with HTTPS) | Rotating refresh token: `POST /auth/refresh` exchanges it for a new access token |
| `ufop_csrf` | `SameSite=Strict; Path=/` (+ `Secure`), readable by scripts | Double-submit CSRF token bound to the refresh token |

Because the browser attaches `ufop_refresh` on its own, every state-changing request (`POST`, `PUT`,
`PATCH`, `DELETE`) that carries it and is **not** authenticated with `Authorization: Bearer` (or the
reverse-proxy pre-authentication header) must send the `ufop_csrf` value in an **`X-CSRF-Token`** header.
The server compares it in constant time with the cookie *and* with an HMAC of the refresh cookie under the
server's signing key, so a cookie planted by another site does not help. A missing or wrong token gets
`403` with `error_type: "csrf"`. The token rotates with every refresh, is also returned as `csrf_token` in
the sign-in and refresh responses, and changes when tokens are reset.

Defense in depth: those requests, and the sign-in routes (`/auth/login`, `/auth/signin`), are refused with
`403` when `Origin` (or, without it, `Referer`) names a site other than the server's own `Host`,
`--public-url` or a `--cors-origins` entry. Requests without either header (scripts, the CLI) pass this check.

Bearer-token requests are exempt: they cannot be forged cross-site. The web UI and `ufop server` send the
header for you; a script that keeps the refresh cookie must send it too
(`curl -b jar -c jar -H "X-CSRF-Token: $(awk '/ufop_csrf/{print $7}' jar)" -X POST …/api/v1/auth/refresh`).
After upgrading, run `ufop server login` again once so the CLI stores the token. Share-link pages (`/s/…`)
carry their capability in the URL and use `SameSite=Lax` unlock cookies; they are not covered by this check.

## Request tracing and logs

Every request gets a **request id**. A client may send its own in `X-Request-Id` (1–128 characters of
letters, digits and `. _ : / + = -`); anything else is replaced by a new UUIDv4. The id is

- echoed in the `X-Request-Id` response header (also exposed to CORS clients),
- added as `request_id` to every JSON error body (`{error_type, message, advice, request_id}`),
- a field of the request's tracing span `http.request` (tower-http `TraceLayer`), next to `http.method`,
  `http.route` (the **route template**, e.g. `/api/v1/jobs/{id}/run`), and `http.status_code`.

With `--request-log on` (the default; `UFOP_SERVER_REQUEST_LOG`) the server writes one line per request:

```text
INFO http.request{otel.kind="server" http.method=POST http.route=/api/v1/jobs/{id}/run http.status_code=200 request_id=3f1c…}: ufop_server::http: request status=200 latency_ms=4.2
```

`--request-log off` drops these lines; spans (and trace export) stay. Raw paths are never logged, because
share links and gateway paths carry tokens in them. At `RUST_LOG=debug` a "request received" line adds the
query string with sensitive parameters (`token`, `access_token`, `authid`, `code`, `password`, `secret`,
`key`, `signature`, `X-Amz-*`, SAS `sig`/`se`/`sp`, anything ending in `token`, `secret` or `password`)
replaced by `REDACTED`, and the headers with `Authorization`, `Cookie`, `Set-Cookie`, `X-UFOP-PreAuth` and
`X-CSRF-Token` replaced. Quote the request id when reporting a problem; search the log for it.
