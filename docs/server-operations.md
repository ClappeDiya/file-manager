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
