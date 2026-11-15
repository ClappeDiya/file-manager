import { FieldDef,PROVIDER_FIELDS,newCloudConfig } from './backup-cloud-destination-helpers';
/**
 * Cloud / network destination editor. Fields mirror the Rust provider
 * configs in `src-tauri/src/backup_engine/stores/*` (snake_case keys); the
 * secret is sent separately to the credential store and never kept in the
 * job.
 */
import { tauriInvoke } from "@/hooks/use-tauri";
import { secretHint } from "@/lib/backup-cloud-auth";
import { inputCls } from "@/lib/backup-helpers";
import { errorText,type CloudConfig,type CloudProviderInfo } from "@/stores/backup-store";
import { useEffect,useState } from "react";
import { CloudProviderExtras } from "./backup-cloud-advanced";
import { Banner,Btn,Check,Field } from "./backup-ui";

/** Read a TLS server's certificate fingerprint so it can be compared and pinned. */
export function FingerprintFetcher({ onPin }: { onPin: (fingerprint: string) => void }) {
  const [target, setTarget] = useState("");
  const [fp, setFp] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const fetchIt = async () => {
    setErr(null);
    setFp(null);
    const m = /^(?:https?:\/\/)?([^/:]+)(?::(\d+))?/.exec(target.trim());
    if (!m) return setErr("Enter host or host:port");
    try {
      setFp(await tauriInvoke<string>("tls_fetch_fingerprint", { host: m[1], port: Number(m[2] ?? 443) }));
    } catch (e) {
      setErr(errorText(e));
    }
  };
  return (
    <div className="space-y-1" data-testid="fingerprint-fetcher">
      <div className="flex gap-1 items-end">
        <Field label="Server certificate fingerprint">
          <input className={inputCls} value={target} placeholder="storage.example.com:443" onChange={(e) => setTarget(e.target.value)} data-testid="fingerprint-target" />
        </Field>
        <Btn testId="fingerprint-get" disabled={!target.trim()} onClick={() => void fetchIt()}>
          Get fingerprint
        </Btn>
      </div>
      {fp && (
        <div className="flex gap-2 items-center text-[11px] font-mono break-all">
          <span data-testid="fingerprint-value">{fp}</span>
          <Btn testId="fingerprint-pin" onClick={() => onPin(fp)}>
            Pin
          </Btn>
        </div>
      )}
      {err && <div className="text-xs text-red-600">{err}</div>}
    </div>
  );
}

function get(o: Record<string, unknown>, f: FieldDef): unknown {
  let cur: unknown = o;
  for (const p of f.path ?? []) cur = (cur as Record<string, unknown>)?.[p];
  return (cur as Record<string, unknown>)?.[f.key];
}

function set(o: Record<string, unknown>, f: FieldDef, v: unknown): Record<string, unknown> {
  if (!f.path?.length) return { ...o, [f.key]: v };
  const [head, ...rest] = f.path;
  return { ...o, [head]: set((o[head] as Record<string, unknown>) ?? {}, { ...f, path: rest }, v) };
}

const VERIFIED_LABEL: Record<string, string> = {
  emulator: "tested with a local emulator",
  "real-server": "tested against a real server",
  cli: "uses the installed tool",
  "api-mock": "API-level tests only — not yet tried against the live service",
};

export function CloudDestinationForm({
  config,
  credentialId,
  onChange,
}: {
  config: CloudConfig;
  credentialId: string;
  onChange: (config: CloudConfig) => void;
}) {
  const [providers, setProviders] = useState<CloudProviderInfo[]>([]);
  const [showHidden, setShowHidden] = useState(false);
  const [secret, setSecret] = useState("");
  const [hasSecret, setHasSecret] = useState(false);
  const [msg, setMsg] = useState<{ tone: "success" | "error"; text: string } | null>(null);

  useEffect(() => {
    tauriInvoke<CloudProviderInfo[]>("backup_cloud_providers", undefined, []).then(setProviders).catch(() => setProviders([]));
  }, []);
  useEffect(() => {
    if (credentialId) tauriInvoke<boolean>("backup_has_destination_secret", { credentialId }, false).then(setHasSecret).catch(() => setHasSecret(false));
  }, [credentialId]);

  const info = providers.find((p) => p.id === config.provider);
  const schema = PROVIDER_FIELDS[config.provider];
  const values = config as unknown as Record<string, unknown>;
  const saveSecret = async () => {
    try {
      await tauriInvoke("backup_set_destination_secret", { credentialId, secret });
      setHasSecret(secret.length > 0);
      setSecret("");
      setMsg({ tone: "success", text: "Credentials saved to the secure store." });
    } catch (e) {
      setMsg({ tone: "error", text: errorText(e) });
    }
  };

  return (
    <div className="space-y-2" data-testid="cloud-destination">
      <div className="flex gap-2 items-end">
        <Field label="Provider">
          <select data-testid="cloud-provider" className={inputCls} value={config.provider} onChange={(e) => onChange(newCloudConfig(e.target.value))}>
            {providers
              .filter((p) => showHidden || !p.hidden || p.id === config.provider)
              .map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                  {p.verified === "api-mock" ? " (untested)" : ""}
                </option>
              ))}
          </select>
        </Field>
        <Check label="Show untested providers" checked={showHidden} onChange={setShowHidden} />
      </div>
      {info && <div className="text-xs text-zinc-500">Status: {VERIFIED_LABEL[info.verified] ?? info.verified}</div>}
      <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
        {schema?.fields.map((f) => {
          const v = get(values, f);
          const update = (nv: unknown) => onChange(set(values, f, nv) as unknown as CloudConfig);
          if (f.kind === "bool") return <Check key={f.key} label={f.label} checked={Boolean(v)} onChange={update} />;
          if (f.kind === "list")
            return (
              <Field key={(f.path ?? []).concat(f.key).join(".")} label={f.label} hint={f.hint}>
                <input
                  className={`${inputCls} font-mono`}
                  value={Array.isArray(v) ? v.join(", ") : ""}
                  onChange={(e) => update(e.target.value.split(/[,\s]+/).filter(Boolean))}
                  data-testid={`cloud-${f.key}`}
                />
              </Field>
            );
          return (
            <Field key={(f.path ?? []).concat(f.key).join(".")} label={f.label} hint={f.hint}>
              {f.kind === "select" ? (
                <select className={inputCls} value={String(v ?? "")} onChange={(e) => update(e.target.value)}>
                  {f.options!.map((o) => (
                    <option key={o} value={o}>
                      {o || "(default)"}
                    </option>
                  ))}
                </select>
              ) : (
                <input
                  className={inputCls}
                  type={f.kind === "number" ? "number" : "text"}
                  value={v == null ? "" : String(v)}
                  onChange={(e) => update(f.kind === "number" ? Math.max(0, Number(e.target.value) || 0) : e.target.value)}
                  data-testid={`cloud-${f.key}`}
                />
              )}
            </Field>
          );
        })}
      </div>
      <CloudProviderExtras config={config} onChange={onChange} />
      {"tls" in values && (
        <FingerprintFetcher
          onPin={(fp) => {
            const tls = values.tls as { pinned_sha256?: string[] };
            const pins = tls.pinned_sha256 ?? [];
            if (!pins.includes(fp)) onChange({ ...values, tls: { ...tls, pinned_sha256: [...pins, fp] } } as unknown as CloudConfig);
          }}
        />
      )}
      <Field label={`Credentials${info ? ` — ${info.secret}` : ""}`} hint={[secretHint(config), hasSecret ? "Stored. Enter a new value to replace it." : "Not stored yet."].filter(Boolean).join(" ")}>
        <textarea data-testid="cloud-secret" className={`${inputCls} font-mono`} rows={2} value={secret} onChange={(e) => setSecret(e.target.value)} autoComplete="off" />
      </Field>
      <Btn disabled={!secret} testId="cloud-secret-save" onClick={() => void saveSecret()}>
        Save credentials
      </Btn>
      {msg && <Banner tone={msg.tone} onClose={() => setMsg(null)}>{msg.text}</Banner>}
    </div>
  );
}
