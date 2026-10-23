# Backup notifications: profiles, webhooks and HTML email

This page covers notification profiles (reusable channels), the webhook channel with its presets for Pushover, ntfy, Discord, PagerDuty and Gotify, and HTML email. The channels themselves, log outputs and support tools are described in [backup-logging-and-support.md](backup-logging-and-support.md).

## Channels

| Channel | Settings | Secret (secure store, `credential_id`) |
|---|---|---|
| Email (SMTP) | server, port, TLS mode, username, from, to, subject, **HTML** and HTML template | SMTP password |
| Telegram | chat id, Bot API URL | bot token |
| XMPP | sender JID, recipient, server, port, TLS mode | account password |
| Slack, Microsoft Teams | none | incoming-webhook URL |
| Webhook | URL, method, content type, headers, body template, preset | `{{secret}}`: a token, key or the URL itself |

Every channel has its own result filter (`send_on`: `success`, `warning`, `error`), message prefix and certificate policy. **Send test** sends a message straight away.

## Notification profiles

A profile is a channel set up once and chosen in any job: **Backup settings → Notification profiles** lists them, **New profile** picks the channel (or a webhook preset), **Test** sends a test message with the stored secrets, **Edit** and **Delete** change or remove one. In the job editor, **Hooks → Notification profiles** chooses the profiles the job also sends to. Both are available in Standard mode; the minimum severity, message and HTML templates, webhook headers and body template, and the preview are Advanced.

- A job sends to its own channels plus every chosen profile (`notification_profile_ids`). The same channel listed twice (same settings and credential) is sent once, with the result filters merged.
- **Minimum severity** (`min_severity`: `success`, `warning` or `error`) replaces the per-result filter: `warning` sends warnings and errors.
- **Policy templates** can set `notification_profile_ids` like any other section, so every job based on the template uses the same profiles.
- Deleting a profile that jobs or templates use is refused. **Delete anyway** (`force`) removes it from them and reports each one; its stored secrets are deleted too.
- Profiles are stored in the configuration table (`backup.notification_profiles`); their secrets stay in the secure store under the channel's credential ids. Job exports do not carry profiles (they hold secrets): on import, references to profiles that do not exist here are dropped.
- A secret sent with a save (`secret`) is stored and never returned or kept in the profile.

### Organization profiles

The management console can deliver profiles in the managed app settings (`notificationProfiles`, **Fleet → App settings → Notification profiles**). They are read-only on each desktop and server, listed with an *Organization* badge, and chosen as `org:<id>`. Secrets never travel in them: `credential_id` and every secret header must name a console credential (`central:<id>`), delivered through the console's credential channel. Plain-text secret header values are refused by both the console and the desktop.

```json
[{ "id": "on-call", "name": "On-call pager", "minSeverity": "error",
   "notification": { "type": "webhook", "preset": "pagerduty", "url": "https://events.pagerduty.com/v2/enqueue",
                     "body_template": "…", "send_on": ["success", "warning", "error"], "credential_id": "central:pagerduty-key" } }]
```

Paste the `notification` of a profile from the desktop app and change its credential ids.

## Webhooks

A webhook channel sends one HTTP request per run:

- **URL** with placeholders (URL-encoded), **method** `POST`, `PUT`, `PATCH` or `GET` (no body), **content type** (default `application/json`).
- **Headers**: name and value (placeholders allowed). Names must be RFC 9110 tokens; `Host`, `Content-Length`, `Transfer-Encoding`, `Connection` and other hop-by-hop headers are refused, and values cannot contain line breaks (rendered values lose CR/LF too, so placeholders cannot inject headers). Non-ASCII values are sent RFC 2047-encoded. A **secret** header's value is moved to the secure store when the job or profile is saved; exports, bug reports and remote configuration copies never contain it.
- **Body template** (empty = a JSON object of the run's values). Placeholders are escaped for the body format: JSON string content for JSON bodies (so `"text": "{{message}}"` stays valid JSON whatever the message holds), form encoding for `application/x-www-form-urlencoded`, HTML escaping for HTML. `{{json var}}` inserts a quoted JSON string and `{{raw var}}` the value unchanged.

Variables: `name`, `operation`, `result`, `machine`, `backup_id`, `job_id`, `duration`, `files`, `bytes`, `errors`, `title` (the subject line), `message` (the report text), `secret`, `previous_result`, `recovered`, and for presets `pd_severity`, `event_action`, `pushover_priority`, `ntfy_priority`, `ntfy_tags`, `gotify_priority`, `discord_color`.

HTTP reports (**Hooks → HTTP reports**) take the same headers, including secret ones, and `PATCH`.

### Presets

A preset fills in a webhook channel; the secret goes in the secure store as usual.

| Preset | Request | Secret | Inputs |
|---|---|---|---|
| Pushover | `POST https://api.pushover.net/1/messages.json`, form: `token`, `user`, `title`, `message` (1,000 characters), `priority` (error 1, warning 0, success −1) | application API token | user or group key |
| ntfy | `POST {server}/{topic}`, the message as text, headers `Title`, `Priority` (error 4, warning 3, success 2), `Tags`; optional `Authorization: Bearer` | access token (optional) | server (default `https://ntfy.sh`), topic, bearer |
| Discord | `POST` to the webhook URL, JSON `{username, content, embeds: [{title, description, color}]}` | the Discord webhook URL | none |
| PagerDuty Events v2 | `POST https://events.pagerduty.com/v2/enqueue`, JSON `{routing_key, event_action, dedup_key: "ufop-<backup id>", payload: {summary, source, severity, component, group, custom_details}}` | integration (routing) key | none |
| Gotify | `POST {server}/message?token=…`, JSON `{title, message, priority}` (error 8, warning 5, success 2) | application token | server |
| Webhook | `POST` JSON of the run's values | anything referenced as `{{secret}}` | URL |

PagerDuty triggers an alert for warnings and errors (`critical` for errors, `warning` for warnings) and **resolves** it on the next successful run; successes after successes send nothing. The deduplication key is per job, so repeated failures update one incident. **Send test** triggers a test incident and resolves it at once.

The engine builds presets (`backup_webhook_preset`, `POST /api/v1/notifications/webhook-preset`), so the desktop, the server web UI and the CLI create the same requests.

## HTML email

Email channels can send `multipart/alternative` messages: a plain-text part and an HTML part, both UTF-8 and base64 (lines of 76 characters). New channels send HTML; channels created before this release keep text only until **Send as HTML** is turned on (`html`). Subjects with non-ASCII characters are RFC 2047 encoded words split to stay within line limits.

The default HTML is a table of the run (job, operation, result in its colour, machine, duration, files, bytes, errors) above the report text. In Advanced mode, **HTML template** replaces it: every `{{var}}` is HTML-escaped (`{{raw var}}` is not), and `{{result_color}}` gives the result's colour. **Preview** renders the message for a sample successful, warning or failed run in a sandboxed frame (no scripts, no network), and shows webhook requests with secrets masked (`backup_preview_notification`, `POST /api/v1/notifications/preview`).

## Commands and API

| Desktop IPC | REST (`/api/v1`) | CLI (`ufop backup notify-profile …`, server-backed) |
|---|---|---|
| `backup_list_notification_profiles` | `GET /notification-profiles` | `list` |
| `backup_get_notification_profile` | `GET /notification-profiles/{id}` | `show <profile>` |
| `backup_save_notification_profile` | `POST /notification-profiles`, `PUT /notification-profiles/{id}` | `create`, `update <profile>` |
| `backup_delete_notification_profile` (`force`) | `DELETE /notification-profiles/{id}?force=true` | `delete <profile> [--force]` |
| `backup_test_notification_profile` | `POST /notification-profiles/{id}/test` | `test <profile>` |
| `backup_preview_notification` | `POST /notifications/preview` | |
| `backup_webhook_preset` | `POST /notifications/webhook-preset` | `create --preset …` |

```bash
ufop server login https://backup.example.org:8200
ufop backup notify-profile create --name "On call" --preset pagerduty --min-severity warning --secret-env PD_ROUTING_KEY
ufop backup notify-profile create --name Phone --preset ntfy --topic backups --bearer --secret-env NTFY_TOKEN
ufop backup notify-profile create --name Ops --notification '{"type":"email","host":"smtp.example.org","from":"ufop@example.org","to":["ops@example.org"],"html":true}' --secret-env SMTP_PASSWORD
ufop backup notify-profile test "On call"
ufop backup notify-profile update Ops --min-severity error
ufop backup notify-profile delete Phone --force      # also removes it from jobs and templates
```
