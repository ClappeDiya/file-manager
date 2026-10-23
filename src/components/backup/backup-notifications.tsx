/**
 * Job editor pieces for result notifications (email, Telegram, XMPP, Slack,
 * Teams and webhooks with presets for Pushover, ntfy, Discord, PagerDuty and
 * Gotify), report privacy and TLS settings, and per-job log outputs (file,
 * system log). Channel passwords, bot tokens, webhook URLs and secret header
 * values go to the secure store under credential ids, never into the job.
 */
import { useEffect, useState } from "react";
import { tauriInvoke } from "@/hooks/use-tauri";
import { inputCls } from "@/lib/backup-helpers";
import {
  errorText,
  newLogFilter,
  type BackupNotification,
  type LogFilter,
  type LogLevel,
  type LogOutputs,
  type NotificationChannel,
  type ReportConfig,
  type TlsMode,
  type WebhookHeader,
  type WebhookPreset,
} from "@/stores/backup-store";
import { PRESETS, channelLabel, previewNotification, validHeaderName, webhookPreset, type NotificationPreview, type PresetParams, type Severity } from "@/stores/backup-notify";
import { Banner, Btn, Check, Field, Section } from "./backup-ui";
import { AdvancedOnly } from "./advanced-only";

const RESULTS = ["success", "warning", "error"] as const;
const LEVELS: LogLevel[] = ["info", "warning", "error"];

function secretLabel(n: BackupNotification): string {
  switch (n.type) {
    case "email":
      return "SMTP password";
    case "telegram":
      return "Bot token";
    case "xmpp":
      return "Account password";
    case "slack":
    case "teams":
      return "Webhook URL";
    case "webhook":
      return PRESETS.find((p) => p.id === n.preset)?.secret ?? "Secret";
  }
}

type SimpleType = Exclude<NotificationChannel["type"], "webhook">;

export function newNotification(type: SimpleType): BackupNotification {
  const base = { send_on: ["warning", "error"], credential_id: crypto.randomUUID(), template: "", cert: { accept_invalid_certs: false, ca_pem: null, pinned_sha256: [] } };
  if (type === "telegram") return { type, chat_id: "", api_base: "", ...base };
  if (type === "xmpp") return { type, jid: "", to: "", host: "", port: null, tls: "start_tls", ...base };
  if (type === "slack" || type === "teams") return { type, ...base };
  // New mail channels send HTML (with a text part); older ones keep text only.
  return { type: "email", host: "", port: null, tls: "start_tls", username: "", from: "", to: [], subject: "UFOP {{operation}} {{result}}: {{name}}", html: true, html_template: "", ...base };
}

const splitList = (s: string) => s.split(/[\n,]/).map((x) => x.trim()).filter(Boolean);
const splitPins = (s: string) => s.split(/[,\s]+/).filter(Boolean);
const portValue = (v: string) => (v.trim() === "" ? null : Math.min(65535, Math.max(1, Number(v) || 1)));

function TlsSelect({ value, onChange }: { value: TlsMode; onChange: (t: TlsMode) => void }) {
  return (
    <select className={inputCls} value={value} onChange={(e) => onChange(e.target.value as TlsMode)}>
      <option value="start_tls">STARTTLS</option>
      <option value="implicit">TLS (implicit)</option>
      <option value="none">None (local relays only)</option>
    </select>
  );
}

/** Request headers; a secret one's value is sent to the secure store on save. */
export function HeadersEditor({ value, onChange }: { value: WebhookHeader[]; onChange: (h: WebhookHeader[]) => void }) {
  const set = (i: number, p: Partial<WebhookHeader>) => onChange(value.map((h, j) => (j === i ? { ...h, ...p } : h)));
  return (
    <div className="space-y-1" data-testid="webhook-headers">
      <span className="text-xs font-medium">Headers</span>
      {value.map((h, i) => (
        <div key={i} className="flex gap-1 items-center">
          <input className={`${inputCls} font-mono ${h.name && !validHeaderName(h.name) ? "border-red-500" : ""}`} placeholder="X-Api-Key" value={h.name} onChange={(e) => set(i, { name: e.target.value })} aria-label="Header name" />
          <input
            className={`${inputCls} font-mono`}
            type={h.secret ? "password" : "text"}
            autoComplete="off"
            placeholder={h.secret && h.credential_id ? "(saved; type to replace)" : "value or {{placeholder}}"}
            value={h.value}
            onChange={(e) => set(i, { value: e.target.value.replace(/[\r\n]/g, "") })}
            aria-label="Header value"
          />
          <Check label="Secret" checked={h.secret} onChange={(v) => set(i, { secret: v, credential_id: v ? h.credential_id || crypto.randomUUID() : h.credential_id })} />
          <Btn tone="danger" onClick={() => onChange(value.filter((_, j) => j !== i))}>
            ×
          </Btn>
        </div>
      ))}
      <Btn testId="webhook-header-add" onClick={() => onChange([...value, { name: "", value: "", secret: false, credential_id: "" }])}>
        + Header
      </Btn>
    </div>
  );
}

function WebhookFields({ n, upd }: { n: Extract<BackupNotification, { type: "webhook" }>; upd: (p: Record<string, unknown>) => void }) {
  const urlFromSecret = n.url.trim().startsWith("{{raw secret}}");
  return (
    <div className="space-y-2">
      {urlFromSecret ? (
        <p className="text-xs text-zinc-500">The webhook URL is the secret below (kept in the secure store).</p>
      ) : (
        <Field label="URL" hint="Placeholders such as {{secret}} or {{name}} are URL-encoded.">
          <input className={`${inputCls} font-mono`} value={n.url} placeholder="https://hooks.example.com/ufop" onChange={(e) => upd({ url: e.target.value })} data-testid="webhook-url" />
        </Field>
      )}
      <AdvancedOnly>
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
          <Field label="Method">
            <select className={inputCls} value={n.method} onChange={(e) => upd({ method: e.target.value })} data-testid="webhook-method">
              {["POST", "PUT", "PATCH", "GET"].map((m) => (
                <option key={m}>{m}</option>
              ))}
            </select>
          </Field>
          <Field label="Content type (empty = application/json)">
            <input className={inputCls} value={n.content_type} onChange={(e) => upd({ content_type: e.target.value })} />
          </Field>
        </div>
        <HeadersEditor value={n.headers} onChange={(headers) => upd({ headers })} />
        <Field
          label="Body template (empty = the run's values as JSON)"
          hint="{{var}} is escaped for the body (JSON string content, form encoding); {{json var}} inserts a quoted JSON string, {{raw var}} the value as is. Variables: name, operation, result, machine, duration, files, bytes, errors, title, message, secret, pd_severity, event_action, previous_result."
        >
          <textarea className={`${inputCls} font-mono`} rows={4} value={n.body_template} onChange={(e) => upd({ body_template: e.target.value })} data-testid="webhook-body" />
        </Field>
      </AdvancedOnly>
    </div>
  );
}

function ChannelFields({ n, set }: { n: BackupNotification; set: (p: Partial<BackupNotification>) => void }) {
  const upd = set as (p: Record<string, unknown>) => void;
  if (n.type === "email")
    return (
      <div className="space-y-2">
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
          <Field label="SMTP server">
            <input className={inputCls} value={n.host} placeholder="smtp.example.com" onChange={(e) => upd({ host: e.target.value })} />
          </Field>
          <div className="grid grid-cols-2 gap-1">
            <Field label="Port">
              <input className={inputCls} type="number" value={n.port ?? ""} placeholder={n.tls === "implicit" ? "465" : "587"} onChange={(e) => upd({ port: portValue(e.target.value) })} />
            </Field>
            <Field label="Security">
              <TlsSelect value={n.tls} onChange={(tls) => upd({ tls })} />
            </Field>
          </div>
          <Field label="Username (empty = no login)">
            <input className={inputCls} value={n.username} onChange={(e) => upd({ username: e.target.value })} />
          </Field>
          <Field label="From">
            <input className={inputCls} value={n.from} placeholder="backups@example.com" onChange={(e) => upd({ from: e.target.value })} />
          </Field>
          <Field label="To (comma or one per line)">
            <textarea className={inputCls} rows={1} value={n.to.join("\n")} onChange={(e) => upd({ to: splitList(e.target.value) })} />
          </Field>
          <Field label="Subject" hint="Same placeholders as the message template.">
            <input className={inputCls} value={n.subject} onChange={(e) => upd({ subject: e.target.value })} />
          </Field>
        </div>
        <Check label="Send as HTML (with a plain-text part)" checked={n.html ?? false} onChange={(html) => upd({ html })} />
        {n.html && (
          <AdvancedOnly>
            <Field label="HTML template (empty = the built-in report table)" hint="Values are HTML-escaped; {{raw var}} inserts one unescaped. Extra variable: result_color.">
              <textarea className={`${inputCls} font-mono`} rows={4} value={n.html_template ?? ""} onChange={(e) => upd({ html_template: e.target.value })} data-testid="email-html-template" />
            </Field>
          </AdvancedOnly>
        )}
      </div>
    );
  if (n.type === "telegram")
    return (
      <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
        <Field label="Chat id" hint="A user, group or channel id (e.g. -1001234567890).">
          <input className={inputCls} value={n.chat_id} onChange={(e) => upd({ chat_id: e.target.value })} />
        </Field>
        <Field label="Bot API URL (empty = api.telegram.org)">
          <input className={inputCls} value={n.api_base} onChange={(e) => upd({ api_base: e.target.value })} />
        </Field>
      </div>
    );
  if (n.type === "slack" || n.type === "teams") return <p className="text-xs text-zinc-500">Paste the incoming-webhook URL as the secret below.</p>;
  if (n.type === "webhook") return <WebhookFields n={n} upd={upd} />;
  return (
    <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
      <Field label="Sender account (JID)">
        <input className={inputCls} value={n.jid} placeholder="ufop@example.org" onChange={(e) => upd({ jid: e.target.value })} />
      </Field>
      <Field label="Recipient JID">
        <input className={inputCls} value={n.to} placeholder="me@example.org" onChange={(e) => upd({ to: e.target.value })} />
      </Field>
      <Field label="Server (empty = JID domain)">
        <input className={inputCls} value={n.host} onChange={(e) => upd({ host: e.target.value })} />
      </Field>
      <div className="grid grid-cols-2 gap-1">
        <Field label="Port">
          <input className={inputCls} type="number" value={n.port ?? ""} placeholder="5222" onChange={(e) => upd({ port: portValue(e.target.value) })} />
        </Field>
        <Field label="Security">
          <TlsSelect value={n.tls} onChange={(tls) => upd({ tls })} />
        </Field>
      </div>
    </div>
  );
}

/** What a channel would send for a sample run; HTML renders in a sandboxed frame. */
export function NotificationPreviewPanel({ n }: { n: BackupNotification }) {
  const [result, setResult] = useState<Severity>("error");
  const [p, setP] = useState<NotificationPreview | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const load = async () => {
    setErr(null);
    try {
      setP(await previewNotification(n, result));
    } catch (e) {
      setErr(errorText(e));
    }
  };
  return (
    <div className="space-y-1" data-testid="notification-preview">
      <div className="flex gap-1 items-center">
        <select className={`${inputCls} w-auto`} value={result} onChange={(e) => setResult(e.target.value as Severity)} aria-label="Sample result">
          {RESULTS.map((r) => (
            <option key={r}>{r}</option>
          ))}
        </select>
        <Btn testId="notification-preview-btn" onClick={load}>
          Preview
        </Btn>
      </div>
      {err && <Banner tone="error">{err}</Banner>}
      {p && (
        <div className="space-y-1 text-xs">
          <div>
            <span className="font-medium">Subject:</span> {p.subject}
          </div>
          {p.html !== null ? (
            <iframe title="HTML preview" sandbox="" srcDoc={p.html} className="w-full h-72 border border-zinc-200 dark:border-zinc-700 rounded bg-white" data-testid="notification-preview-html" />
          ) : p.request !== null ? (
            <pre className="whitespace-pre-wrap break-all font-mono bg-zinc-50 dark:bg-zinc-900 p-2 rounded" data-testid="notification-preview-request">
              {p.request}
            </pre>
          ) : (
            <pre className="whitespace-pre-wrap font-mono bg-zinc-50 dark:bg-zinc-900 p-2 rounded">{p.text}</pre>
          )}
        </div>
      )}
    </div>
  );
}

/**
 * One channel's settings, secret and test button. `onTest` replaces the
 * default test (for saved profiles); `children` adds fields (profiles).
 */
export function NotificationCard({
  n,
  onChange,
  onRemove,
  onTest,
  children,
}: {
  n: BackupNotification;
  onChange: (n: BackupNotification) => void;
  onRemove?: () => void;
  onTest?: () => Promise<void>;
  children?: React.ReactNode;
}) {
  const [secret, setSecret] = useState("");
  const [hasSecret, setHasSecret] = useState(false);
  const [msg, setMsg] = useState<{ tone: "success" | "error"; text: string } | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    tauriInvoke<boolean>("backup_has_destination_secret", { credentialId: n.credential_id }, false).then(setHasSecret).catch(() => setHasSecret(false));
  }, [n.credential_id]);
  const set = (p: Partial<BackupNotification>) => onChange({ ...n, ...p } as BackupNotification);
  const saveSecret = async () => {
    try {
      await tauriInvoke("backup_set_destination_secret", { credentialId: n.credential_id, secret });
      setHasSecret(secret.length > 0);
      setSecret("");
      setMsg({ tone: "success", text: "Saved to the secure store." });
    } catch (e) {
      setMsg({ tone: "error", text: errorText(e) });
    }
  };
  const test = async () => {
    setBusy(true);
    setMsg(null);
    try {
      if (onTest) await onTest();
      else await tauriInvoke("backup_test_notification", { notification: n });
      setMsg({ tone: "success", text: "Test message sent." });
    } catch (e) {
      setMsg({ tone: "error", text: errorText(e) });
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="border border-zinc-200 dark:border-zinc-700 rounded p-2 space-y-2" data-testid={`notification-${n.type}`}>
      <div className="flex justify-between items-center">
        <span className="text-xs font-medium">{channelLabel(n)}</span>
        <div className="flex gap-1">
          <Btn testId="notification-test" disabled={busy} onClick={test}>
            {busy ? "Sending…" : "Send test"}
          </Btn>
          {onRemove && (
            <Btn tone="danger" onClick={onRemove}>
              Remove
            </Btn>
          )}
        </div>
      </div>
      {children}
      <ChannelFields n={n} set={set} />
      <div className="flex gap-1 items-end">
        <Field label={`${secretLabel(n)} ${hasSecret ? "(saved)" : ""}`}>
          <input className={inputCls} type="password" autoComplete="off" value={secret} onChange={(e) => setSecret(e.target.value)} data-testid="notification-secret" />
        </Field>
        <Btn onClick={saveSecret} testId="notification-secret-save">
          {secret ? "Save" : hasSecret ? "Clear" : "Save"}
        </Btn>
      </div>
      <div className="flex gap-3 flex-wrap">
        {RESULTS.map((r) => (
          <Check key={r} label={`On ${r}`} checked={n.send_on.includes(r)} onChange={(v) => set({ send_on: v ? [...n.send_on, r] : n.send_on.filter((x) => x !== r) })} />
        ))}
      </div>
      <Field label="Message prefix (optional)" hint="Placed before the job's report text.">
        <input className={inputCls} value={n.template} onChange={(e) => set({ template: e.target.value })} />
      </Field>
      <Check label="Accept invalid TLS certificates" checked={n.cert.accept_invalid_certs} onChange={(v) => set({ cert: { ...n.cert, accept_invalid_certs: v } })} />
      <Field label="Trusted CA or server certificate (PEM, optional)">
        <textarea className={`${inputCls} font-mono`} rows={2} value={n.cert.ca_pem ?? ""} onChange={(e) => set({ cert: { ...n.cert, ca_pem: e.target.value.trim() ? e.target.value : null } })} />
      </Field>
      <Field label="Pinned SHA-256 fingerprints (optional)" hint="When set, only these certificates are accepted.">
        <input className={`${inputCls} font-mono`} value={(n.cert.pinned_sha256 ?? []).join(", ")} onChange={(e) => set({ cert: { ...n.cert, pinned_sha256: splitPins(e.target.value) } })} />
      </Field>
      {(n.type === "webhook" || (n.type === "email" && n.html)) && (
        <AdvancedOnly>
          <NotificationPreviewPanel n={n} />
        </AdvancedOnly>
      )}
      {msg && (
        <Banner tone={msg.tone} onClose={() => setMsg(null)}>
          {msg.text}
        </Banner>
      )}
    </div>
  );
}

const PRESET_INPUTS: Record<WebhookPreset, (keyof PresetParams)[]> = {
  pushover: ["user"],
  ntfy: ["server", "topic", "bearer"],
  discord: [],
  pagerduty: [],
  gotify: ["server"],
  generic: ["server"],
};
const PARAM_LABEL: Record<keyof PresetParams, string> = { server: "Server URL", topic: "Topic", user: "User or group key", bearer: "Send the token as a bearer token" };

/** Add a channel: the plain ones directly, webhooks from a preset (built by the engine). */
export function AddChannel({ onAdd }: { onAdd: (n: BackupNotification) => void }) {
  const [preset, setPreset] = useState<WebhookPreset | null>(null);
  const [params, setParams] = useState<PresetParams>({ server: "", topic: "", user: "", bearer: false });
  const [err, setErr] = useState<string | null>(null);
  const add = async () => {
    if (!preset) return;
    setErr(null);
    try {
      onAdd(await webhookPreset(preset, params));
      setPreset(null);
      setParams({ server: "", topic: "", user: "", bearer: false });
    } catch (e) {
      setErr(errorText(e));
    }
  };
  return (
    <div className="space-y-2">
      <div className="flex gap-1 flex-wrap">
        {(["email", "telegram", "xmpp", "slack", "teams"] as const).map((t) => (
          <Btn key={t} testId={`notification-add-${t}`} onClick={() => onAdd(newNotification(t))}>
            + {t === "xmpp" ? "XMPP" : t.charAt(0).toUpperCase() + t.slice(1)}
          </Btn>
        ))}
        <select className={`${inputCls} w-auto`} value={preset ?? ""} onChange={(e) => setPreset((e.target.value || null) as WebhookPreset | null)} data-testid="notification-add-preset" aria-label="Add a webhook">
          <option value="">+ Webhook…</option>
          {PRESETS.map((p) => (
            <option key={p.id} value={p.id}>
              {p.label}
            </option>
          ))}
        </select>
      </div>
      {preset && (
        <div className="flex gap-1 items-end flex-wrap" data-testid="preset-form">
          {PRESET_INPUTS[preset].map((k) =>
            k === "bearer" ? (
              <Check key={k} label={PARAM_LABEL[k]} checked={params.bearer} onChange={(bearer) => setParams({ ...params, bearer })} />
            ) : (
              <Field key={k} label={PARAM_LABEL[k]}>
                <input className={inputCls} value={params[k]} placeholder={k === "server" && preset === "ntfy" ? "https://ntfy.sh" : ""} onChange={(e) => setParams({ ...params, [k]: e.target.value })} data-testid={`preset-${k}`} />
              </Field>
            ),
          )}
          <Btn tone="primary" testId="preset-add" onClick={add}>
            Add {PRESETS.find((p) => p.id === preset)?.label}
          </Btn>
        </div>
      )}
      {err && <Banner tone="error">{err}</Banner>}
    </div>
  );
}

export function NotificationsEditor({ value, onChange }: { value: BackupNotification[]; onChange: (v: BackupNotification[]) => void }) {
  return (
    <Section title="Email, chat and webhooks" hint="Send the run report to people, chats or services. Each channel has its own result filter.">
      {value.map((n, i) => (
        <NotificationCard key={n.credential_id || i} n={n} onChange={(x) => onChange(value.map((y, j) => (j === i ? x : y)))} onRemove={() => onChange(value.filter((_, j) => j !== i))} />
      ))}
      <AddChannel onAdd={(n) => onChange([...value, n])} />
    </Section>
  );
}

/** Privacy, filtering and TLS options shared by HTTP reports and notifications. */
export function ReportExtras({ reports, onChange }: { reports: ReportConfig; onChange: (r: ReportConfig) => void }) {
  return (
    <div className="space-y-2">
      <Check label="Hide file paths in reports and notifications" hint="Source folders become [source:…] tags; other paths keep only the file name." checked={reports.redact_paths} onChange={(v) => onChange({ ...reports, redact_paths: v })} />
      <div className="grid grid-cols-1 sm:grid-cols-3 gap-2">
        <Field label="Lowest log level included">
          <select className={inputCls} value={reports.min_log_level} onChange={(e) => onChange({ ...reports, min_log_level: e.target.value as LogLevel })}>
            {LEVELS.map((l) => (
              <option key={l} value={l}>
                {l}
              </option>
            ))}
          </select>
        </Field>
        <Field label="Log lines included">
          <input className={inputCls} type="number" min={0} max={10000} value={reports.max_log_lines} onChange={(e) => onChange({ ...reports, max_log_lines: Math.max(0, Number(e.target.value) || 0) })} />
        </Field>
        <Field label="First retry delay (ms)" hint="Doubles on each retry.">
          <input className={inputCls} type="number" min={0} value={reports.retry_delay_ms} onChange={(e) => onChange({ ...reports, retry_delay_ms: Math.max(0, Number(e.target.value) || 0) })} />
        </Field>
      </div>
      <Field label="Trusted CA or server certificate for report URLs (PEM, optional)">
        <textarea className={`${inputCls} font-mono`} rows={2} value={reports.ca_pem ?? ""} onChange={(e) => onChange({ ...reports, ca_pem: e.target.value.trim() ? e.target.value : null })} />
      </Field>
      <Field label="Pinned SHA-256 fingerprints for report URLs (optional)" hint="When set, only these certificates are accepted.">
        <input className={`${inputCls} font-mono`} value={(reports.pinned_sha256 ?? []).join(", ")} onChange={(e) => onChange({ ...reports, pinned_sha256: splitPins(e.target.value) })} />
      </Field>
    </div>
  );
}

function FilterFields({ value, onChange, label }: { value: LogFilter; onChange: (f: LogFilter) => void; label: string }) {
  return (
    <div className="grid grid-cols-1 sm:grid-cols-4 gap-2" data-testid={`log-filter-${label}`}>
      <Field label="Lowest level">
        <select className={inputCls} value={value.min_level} onChange={(e) => onChange({ ...value, min_level: e.target.value as LogLevel })}>
          {LEVELS.map((l) => (
            <option key={l} value={l}>
              {l}
            </option>
          ))}
        </select>
      </Field>
      <Field label="Only tags" hint="engine, script, replica, report, network…">
        <input className={inputCls} value={value.include_tags.join(", ")} onChange={(e) => onChange({ ...value, include_tags: splitList(e.target.value) })} />
      </Field>
      <Field label="Never tags">
        <input className={inputCls} value={value.exclude_tags.join(", ")} onChange={(e) => onChange({ ...value, exclude_tags: splitList(e.target.value) })} />
      </Field>
      <Field label="Matching (regex)">
        <input className={`${inputCls} font-mono`} value={value.pattern} onChange={(e) => onChange({ ...value, pattern: e.target.value })} />
      </Field>
    </div>
  );
}

export function LogOutputsEditor({ value, onChange }: { value: LogOutputs; onChange: (v: LogOutputs) => void }) {
  const fileOn = value.file !== null;
  return (
    <Section title="Log outputs" hint="Besides the run history, write each run's log to a file or to the system log (Windows Event Log, syslog).">
      <Check label="Write to a log file" checked={fileOn} onChange={(v) => onChange({ ...value, file: v ? "" : null })} />
      {fileOn && (
        <div className="space-y-2 pl-5">
          <div className="grid grid-cols-1 sm:grid-cols-3 gap-2">
            <div className="sm:col-span-2">
              <Field label="File">
                <input className={`${inputCls} font-mono`} value={value.file ?? ""} placeholder="/var/log/ufop/docs.log" onChange={(e) => onChange({ ...value, file: e.target.value })} data-testid="log-file-path" />
              </Field>
            </div>
            <Field label="Rotate above (MiB, 0 = never)">
              <input className={inputCls} type="number" min={0} value={value.file_max_mb} onChange={(e) => onChange({ ...value, file_max_mb: Math.max(0, Number(e.target.value) || 0) })} />
            </Field>
          </div>
          <FilterFields label="file" value={value.file_filter} onChange={(f) => onChange({ ...value, file_filter: f })} />
        </div>
      )}
      <Check label="Write to the system log" checked={value.system_log} onChange={(v) => onChange({ ...value, system_log: v, system_filter: value.system_filter ?? newLogFilter() })} />
      {value.system_log && (
        <div className="pl-5">
          <FilterFields label="system" value={value.system_filter} onChange={(f) => onChange({ ...value, system_filter: f })} />
        </div>
      )}
    </Section>
  );
}
