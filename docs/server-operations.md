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

## Prometheus metrics

Metrics are served in the Prometheus text format (0.0.4):

- on the main port at **`GET /api/v1/metrics`**, for any signed-in user or API token with the `read` scope
  (`ufop server tokens create prometheus --scope read --days 365`);
- optionally on a **separate listener**, `--metrics-listen 127.0.0.1:9464` (`UFOP_SERVER_METRICS_LISTEN`),
  at `/metrics` without sign-in, or with HTTP Basic when `--metrics-basic-auth prom:<password>`
  (`UFOP_SERVER_METRICS_BASIC_AUTH`) is set. It is off by default; keep it on loopback or a monitoring
  network, since the job names in `ufop_backup_job_info` are visible to anyone who can reach it.

```yaml
scrape_configs:
  - job_name: ufop
    metrics_path: /api/v1/metrics
    scheme: https
    authorization: { credentials_file: /etc/prometheus/ufop-token }   # ufop_pat_… (read scope)
    static_configs: [{ targets: ["backup.example.com:8200"] }]
```

| Metric | Type | Labels | |
|---|---|---|---|
| `ufop_build_info` | gauge | `version`, `os`, `arch` | Always 1 |
| `ufop_uptime_seconds`, `ufop_shutting_down` | gauge | | Seconds since start; 1 while draining |
| `ufop_http_requests_total` | counter | `method`, `route`, `status` | `route` is the route template (`/api/v1/jobs/{id}/run`, `static`, `unmatched`) |
| `ufop_http_request_duration_seconds` | histogram | `method`, `route`, `status` | Buckets 5 ms … 60 s |
| `ufop_backup_job_info` | gauge | `job_id`, `job_name`, `read_only` | One per job (join on `job_id` for names) |
| `ufop_backup_runs_total` | counter | `job_id`, `operation`, `result` | From the run history (survives restarts; pruning the history resets it) |
| `ufop_backup_last_success_timestamp_seconds` | gauge | `job_id` | Newest successful (or warning) backup |
| `ufop_backup_last_duration_seconds` | gauge | `job_id`, `result` | Newest backup run |
| `ufop_backup_last_uploaded_bytes`, `ufop_backup_last_files` | gauge | `job_id` | Newest backup run: bytes uploaded, files examined |
| `ufop_backup_versions`, `ufop_backup_source_bytes`, `ufop_backup_destination_bytes` | gauge | `job_id` | As of the job's last run |
| `ufop_operations_running` | gauge | `operation` | Running now |
| `ufop_operations_queued` | gauge | | Queue length |
| `ufop_repository_lock_held` | gauge | `job_id` | 1 while an operation of the job runs (it holds the repository lock) |
| `ufop_scheduler_running`, `ufop_scheduler_paused` | gauge | | |
| `ufop_scheduler_next_run_timestamp_seconds` | gauge | `job_id`, `operation` | Next backup and maintenance runs |
| `ufop_volume_cache_bytes`, `_max_bytes`, `_entries` | gauge | | Local volume cache |
| `ufop_volume_cache_hits_total`, `_misses_total`, `_evictions_total` | counter | | Since start |

Example alert: `time() - ufop_backup_last_success_timestamp_seconds > 2 * 86400` (no good backup in two days).
At most 2000 HTTP series are kept; further ones are counted under `route="other"`.

## Trace export (OpenTelemetry)

With `--otlp-endpoint http://otel-collector:4318` (`OTEL_EXPORTER_OTLP_ENDPOINT`) the server exports
trace spans over **OTLP/HTTP with JSON** to `{endpoint}/v1/traces` (an endpoint that already ends in
`/v1/traces` is used as is). Any OpenTelemetry Collector, Jaeger, Grafana Tempo, Honeycomb or another
OTLP/HTTP receiver that accepts JSON works.

| Option | Env | Default | |
|---|---|---|---|
| `--otlp-endpoint` | `OTEL_EXPORTER_OTLP_ENDPOINT` | off | Collector URL |
| `--otlp-headers` | `OTEL_EXPORTER_OTLP_HEADERS` | none | `name=value,name2=value2` (values may be %-encoded), e.g. `x-honeycomb-team=…` or `authorization=Bearer%20…` |
| `--otlp-service-name` | `OTEL_SERVICE_NAME` | `ufop-server` | `service.name` resource attribute |
| `--otlp-sample-ratio` | `UFOP_SERVER_OTLP_SAMPLE_RATIO` | `1.0` | Share of new traces kept |

Spans:

- **`http.request`** (kind *server*), named `METHOD /route/template`, with `http.method`, `http.route`,
  `http.status_code` and `request_id`; status *error* for 5xx.
- **`backup.operation`** (kind *internal*) for every backup, restore, verification, compaction and other
  operation, with `job.id`, `job.name`, `backup.operation`, `task.id`, `backup.result`
  (`success`, `warning`, `error`, `cancelled`) and `backup.error`; status *error* when it failed. An
  operation started by a request is a child of that request's span, so one trace shows the API call and the
  backup it started.

**Propagation**: a W3C `traceparent` header on an incoming request makes the request span a child of the
caller's span, in the caller's trace, and the caller's *sampled* flag decides whether it is exported.
Requests without one start a new trace that is kept with probability `--otlp-sample-ratio`; child spans
follow their parent's decision.

Spans are queued (up to 4096; when the collector is slow or down, further spans are dropped instead of
slowing requests) and sent in batches of up to 512 at least every 2 seconds. The queue is flushed on
shutdown. Export errors are logged at debug level.

## Push gateway

For servers that Prometheus cannot reach (behind NAT, short-lived), push the same metrics to a
**Prometheus Pushgateway**:

```bash
ufop-server --pushgateway-url http://pushgateway:9091 --pushgateway-interval 60 \
  --pushgateway-job ufop-server --pushgateway-instance nas-1 --pushgateway-basic-auth pg:secret
```

Every interval (default 60 s; first push at start) the server sends `PUT
{url}/metrics/job/{job}/instance/{instance}` with the `/metrics` text, so each push replaces the previous
group. `--pushgateway-instance` defaults to the host name; values with a `/` use the `@base64` grouping-key
form. Authenticate with `--pushgateway-basic-auth user:password` or `--pushgateway-token` (bearer). When the
server stops, it waits for running backups to finish draining and pushes once more, so the last values
(including `ufop_shutting_down 1`) are kept. Delete stale groups in the Pushgateway when you retire a server.
