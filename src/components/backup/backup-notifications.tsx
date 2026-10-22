/**
 * Job editor pieces for result notifications (email, Telegram, XMPP), report
 * privacy and TLS settings, and per-job log outputs (file, system log).
 * Channel passwords and bot tokens go to the secure store under each
 * notification's `credential_id`, never into the job.
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
} from "@/stores/backup-store";
import { Banner, Btn, Check, Field, Section } from "./backup-ui";

const RESULTS = ["success", "warning", "error"] as const;
const LEVELS: LogLevel[] = ["info", "warning", "error"];
const SECRET_LABEL: Record<NotificationChannel["type"], string> = { email: "SMTP password", telegram: "Bot token", xmpp: "Account password" };

export function newNotification(type: NotificationChannel["type"]): BackupNotification {
  const base = { send_on: ["warning", "error"], credential_id: crypto.randomUUID(), template: "", cert: { accept_invalid_certs: false, ca_pem: null, pinned_sha256: [] } };
  if (type === "telegram") return { type, chat_id: "", api_base: "", ...base };
  if (type === "xmpp") return { type, jid: "", to: "", host: "", port: null, tls: "start_tls", ...base };
  return { type: "email", host: "", port: null, tls: "start_tls", username: "", from: "", to: [], subject: "UFOP {{operation}} {{result}}: {{name}}", ...base };
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

function ChannelFields({ n, set }: { n: BackupNotification; set: (p: Partial<BackupNotification>) => void }) {
  const upd = set as (p: Record<string, unknown>) => void;
  if (n.type === "email")
    return (
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

function NotificationCard({ n, onChange, onRemove }: { n: BackupNotification; onChange: (n: BackupNotification) => void; onRemove: () => void }) {
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
      await tauriInvoke("backup_test_notification", { notification: n });
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
        <span className="text-xs font-medium capitalize">{n.type === "xmpp" ? "XMPP (Jabber)" : n.type}</span>
        <div className="flex gap-1">
          <Btn testId="notification-test" disabled={busy} onClick={test}>
            {busy ? "Sending…" : "Send test"}
          </Btn>
          <Btn tone="danger" onClick={onRemove}>
            Remove
          </Btn>
        </div>
      </div>
      <ChannelFields n={n} set={set} />
      <div className="flex gap-1 items-end">
        <Field label={`${SECRET_LABEL[n.type]} ${hasSecret ? "(saved)" : ""}`}>
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
      {msg && (
        <Banner tone={msg.tone} onClose={() => setMsg(null)}>
          {msg.text}
        </Banner>
      )}
    </div>
  );
}

export function NotificationsEditor({ value, onChange }: { value: BackupNotification[]; onChange: (v: BackupNotification[]) => void }) {
  return (
    <Section title="Email, Telegram and XMPP" hint="Send the run report to people or chats. Each channel has its own result filter.">
      {value.map((n, i) => (
        <NotificationCard key={n.credential_id || i} n={n} onChange={(x) => onChange(value.map((y, j) => (j === i ? x : y)))} onRemove={() => onChange(value.filter((_, j) => j !== i))} />
      ))}
      <div className="flex gap-1">
        <Btn testId="notification-add-email" onClick={() => onChange([...value, newNotification("email")])}>
          + Email
        </Btn>
        <Btn testId="notification-add-telegram" onClick={() => onChange([...value, newNotification("telegram")])}>
          + Telegram
        </Btn>
        <Btn testId="notification-add-xmpp" onClick={() => onChange([...value, newNotification("xmpp")])}>
          + XMPP
        </Btn>
      </div>
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
