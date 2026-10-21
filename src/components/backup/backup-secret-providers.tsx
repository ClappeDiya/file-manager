/**
 * Secret providers (environment, file, pass, HashiCorp Vault, AWS Secrets
 * Manager, Google Secret Manager, Azure Key Vault) and the per-job choice of
 * providers. Job settings written as `$name` or containing `${name}` are
 * filled from these at run time; provider credentials go to the secure
 * store, never into the settings.
 */
import { useEffect, useState } from "react";
import { tauriInvoke } from "@/hooks/use-tauri";
import { inputCls } from "@/lib/backup-helpers";
import { errorText, type PlaceholderReport, type SecretProvider, type SecretProviderKind, type SecretProviderTest, type SecretSettings, type TlsOptions } from "@/stores/backup-store";
import { Banner, Btn, Check, Field, Section } from "./backup-ui";

const TLS: TlsOptions = { accept_invalid_certs: false, ca_pem: null, pinned_sha256: [] };

export const PROVIDER_KINDS: { type: SecretProviderKind["type"]; label: string; credential: string | null }[] = [
  { type: "env", label: "Environment variables", credential: null },
  { type: "file", label: "JSON / .env file", credential: null },
  { type: "pass", label: "pass (Linux password store)", credential: null },
  { type: "vault", label: "HashiCorp Vault (KV)", credential: "Vault token" },
  { type: "aws_secrets_manager", label: "AWS Secrets Manager", credential: "Secret access key (optionally + newline + session token)" },
  { type: "gcp_secret_manager", label: "Google Secret Manager", credential: "Service-account key JSON or access token" },
  { type: "azure_key_vault", label: "Azure Key Vault", credential: "Client secret" },
];

export function newProvider(type: SecretProviderKind["type"]): SecretProvider {
  const base = { id: crypto.randomUUID(), name: PROVIDER_KINDS.find((k) => k.type === type)!.label, credential_id: crypto.randomUUID(), enabled: true, global: true };
  switch (type) {
    case "env":
      return { ...base, type, prefix: "" };
    case "file":
      return { ...base, type, path: "" };
    case "pass":
      return { ...base, type, executable: "", prefix: "" };
    case "vault":
      return { ...base, type, address: "", mount: "secret", path: "", namespace: "", kv_version: 2, tls: TLS };
    case "aws_secrets_manager":
      return { ...base, type, region: "us-east-1", access_key_id: "", endpoint: "", secret_id: "", tls: TLS };
    case "gcp_secret_manager":
      return { ...base, type, project: "", endpoint: "", tls: TLS };
    case "azure_key_vault":
      return { ...base, type, vault_url: "", tenant_id: "", client_id: "", authority: "", tls: TLS };
  }
}

/** Editable text fields per provider kind: [key, label, placeholder]. */
const FIELDS: Record<SecretProviderKind["type"], [string, string, string][]> = {
  env: [["prefix", "Variable prefix", "UFOP_SECRET_"]],
  file: [["path", "File", "/etc/ufop/secrets.json"]],
  pass: [
    ["prefix", "Folder in the store", "backups"],
    ["executable", "pass executable (empty = pass)", "/usr/bin/pass"],
  ],
  vault: [
    ["address", "Vault address", "https://vault.example.com:8200"],
    ["mount", "KV mount", "secret"],
    ["path", "Secret path", "ufop/prod"],
    ["namespace", "Namespace (Enterprise)", ""],
  ],
  aws_secrets_manager: [
    ["region", "Region", "us-east-1"],
    ["access_key_id", "Access key id", "AKIA…"],
    ["secret_id", "Single secret holding JSON (optional)", "prod/ufop"],
    ["endpoint", "Endpoint (empty = AWS)", ""],
  ],
  gcp_secret_manager: [
    ["project", "Project id", "my-project"],
    ["endpoint", "Endpoint (empty = Google)", ""],
  ],
  azure_key_vault: [
    ["vault_url", "Vault URL", "https://my-vault.vault.azure.net"],
    ["tenant_id", "Tenant id", ""],
    ["client_id", "Client (application) id", ""],
    ["authority", "Sign-in endpoint (empty = login.microsoftonline.com)", ""],
  ],
};

function ProviderCard({ p, onChange, onRemove, test }: { p: SecretProvider; onChange: (p: SecretProvider) => void; onRemove: () => void; test: SecretProviderTest | undefined }) {
  const kind = PROVIDER_KINDS.find((k) => k.type === p.type)!;
  const [secret, setSecret] = useState("");
  const [hasSecret, setHasSecret] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);
  const values = p as unknown as Record<string, unknown>;
  useEffect(() => {
    if (kind.credential) tauriInvoke<boolean>("backup_has_destination_secret", { credentialId: p.credential_id }, false).then(setHasSecret).catch(() => setHasSecret(false));
  }, [kind.credential, p.credential_id]);
  const saveSecret = async () => {
    try {
      await tauriInvoke("backup_set_destination_secret", { credentialId: p.credential_id, secret });
      setHasSecret(secret.length > 0);
      setSecret("");
      setMsg("Saved to the secure store.");
    } catch (e) {
      setMsg(errorText(e));
    }
  };
  return (
    <div className="border border-zinc-200 dark:border-zinc-700 rounded p-2 space-y-2" data-testid={`secret-provider-${p.type}`}>
      <div className="flex gap-2 items-end">
        <Field label={kind.label}>
          <input className={inputCls} value={p.name} onChange={(e) => onChange({ ...p, name: e.target.value })} aria-label="Provider name" />
        </Field>
        <Btn tone="danger" onClick={onRemove}>
          Remove
        </Btn>
      </div>
      <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
        {FIELDS[p.type].map(([key, label, ph]) => (
          <Field key={key} label={label}>
            <input className={inputCls} value={String(values[key] ?? "")} placeholder={ph} onChange={(e) => onChange({ ...values, [key]: e.target.value } as unknown as SecretProvider)} data-testid={`sp-${key}`} />
          </Field>
        ))}
        {p.type === "vault" && (
          <Field label="KV version">
            <select className={inputCls} value={p.kv_version} onChange={(e) => onChange({ ...p, kv_version: Number(e.target.value) })}>
              <option value={2}>2</option>
              <option value={1}>1</option>
            </select>
          </Field>
        )}
      </div>
      {"tls" in p && (
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
          <Check label="Accept invalid TLS certificates (tests only)" checked={p.tls.accept_invalid_certs} onChange={(v) => onChange({ ...p, tls: { ...p.tls, accept_invalid_certs: v } })} />
          <Field label="Pinned SHA-256 fingerprints (optional)">
            <input className={`${inputCls} font-mono`} value={p.tls.pinned_sha256.join(", ")} onChange={(e) => onChange({ ...p, tls: { ...p.tls, pinned_sha256: e.target.value.split(/[,\s]+/).filter(Boolean) } })} />
          </Field>
        </div>
      )}
      {kind.credential && (
        <div className="flex gap-1 items-end">
          <Field label={`${kind.credential} ${hasSecret ? "(saved)" : "(not saved)"}`}>
            <textarea className={`${inputCls} font-mono`} rows={1} autoComplete="off" value={secret} onChange={(e) => setSecret(e.target.value)} data-testid="sp-secret" />
          </Field>
          <Btn disabled={!secret} onClick={() => void saveSecret()} testId="sp-secret-save">
            Save
          </Btn>
        </div>
      )}
      <div className="flex gap-3">
        <Check label="Enabled" checked={p.enabled} onChange={(v) => onChange({ ...p, enabled: v })} />
        <Check label="Use for every job" checked={p.global} onChange={(v) => onChange({ ...p, global: v })} hint="Otherwise only jobs that select it." />
      </div>
      {test && (
        <div className={`text-xs ${test.ok ? "text-green-600" : "text-red-600"}`} data-testid="sp-test-result">
          {test.ok ? "Reachable" : "Problem"} · {test.latency_ms} ms
          {test.found.length > 0 && ` · found ${test.found.join(", ")}`}
          {test.missing.length > 0 && ` · missing ${test.missing.join(", ")}`}
          {test.errors.map((e) => (
            <div key={e}>{e}</div>
          ))}
        </div>
      )}
      {msg && <div className="text-xs text-zinc-500">{msg}</div>}
    </div>
  );
}

const EMPTY_SETTINGS: SecretSettings = { providers: [], cache: "memory", cache_ttl_secs: 3600 };
const normalize = (r: Partial<SecretSettings> | null | undefined): SecretSettings => ({ ...EMPTY_SETTINGS, ...(r ?? {}), providers: r?.providers ?? [] });

export function SecretProvidersEditor() {
  const [s, setS] = useState<SecretSettings | null>(null);
  const [probe, setProbe] = useState("");
  const [tests, setTests] = useState<SecretProviderTest[]>([]);
  const [msg, setMsg] = useState<{ tone: "success" | "error"; text: string } | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    tauriInvoke<SecretSettings>("backup_get_secret_settings", undefined, EMPTY_SETTINGS)
      .then((r) => setS(normalize(r)))
      .catch((e) => {
        setS(EMPTY_SETTINGS);
        setMsg({ tone: "error", text: errorText(e) });
      });
  }, []);
  if (!s) return null;
  const save = async () => {
    setBusy(true);
    try {
      setS(normalize(await tauriInvoke<SecretSettings>("backup_set_secret_settings", { settings: s })));
      setMsg({ tone: "success", text: "Secret providers saved." });
    } catch (e) {
      setMsg({ tone: "error", text: errorText(e) });
    } finally {
      setBusy(false);
    }
  };
  const runTests = async () => {
    setBusy(true);
    try {
      setTests((await tauriInvoke<SecretProviderTest[]>("backup_test_secret_providers", { providerId: null, names: probe.split(/[,\s]+/).filter(Boolean) })) ?? []);
    } catch (e) {
      setMsg({ tone: "error", text: errorText(e) });
    } finally {
      setBusy(false);
    }
  };
  return (
    <Section
      title="Secret providers"
      hint="Write a setting as $name (or put ${name} inside it) and the value is read from these providers when the job runs, in order. Stored passwords can be placeholders too. $$ is a literal $."
    >
      {s.providers.map((p, i) => (
        <ProviderCard
          key={p.id}
          p={p}
          test={tests.find((t) => t.provider_id === p.id)}
          onChange={(x) => setS({ ...s, providers: s.providers.map((y, j) => (j === i ? x : y)) })}
          onRemove={() => setS({ ...s, providers: s.providers.filter((_, j) => j !== i) })}
        />
      ))}
      <div className="flex gap-1 items-end flex-wrap">
        <Field label="Add provider">
          <select className={inputCls} value="" onChange={(e) => e.target.value && setS({ ...s, providers: [...s.providers, newProvider(e.target.value as SecretProviderKind["type"])] })} data-testid="sp-add">
            <option value="">Choose…</option>
            {PROVIDER_KINDS.map((k) => (
              <option key={k.type} value={k.type}>
                {k.label}
              </option>
            ))}
          </select>
        </Field>
        <Field label="Cache">
          <select className={inputCls} value={s.cache} onChange={(e) => setS({ ...s, cache: e.target.value as SecretSettings["cache"] })}>
            <option value="none">No cache</option>
            <option value="memory">In memory</option>
            <option value="persistent">In memory and the secure store (survives outages)</option>
          </select>
        </Field>
        <Field label="Cache lifetime (s)">
          <input className={inputCls} type="number" min={1} value={s.cache_ttl_secs} onChange={(e) => setS({ ...s, cache_ttl_secs: Math.max(1, Number(e.target.value) || 1) })} />
        </Field>
      </div>
      <div className="flex gap-1 items-end flex-wrap">
        <Btn tone="primary" disabled={busy} onClick={() => void save()} testId="sp-save">
          Save providers
        </Btn>
        <Field label="Test with names (optional)">
          <input className={inputCls} value={probe} placeholder="db_pass, smtp_pass" onChange={(e) => setProbe(e.target.value)} data-testid="sp-probe" />
        </Field>
        <Btn disabled={busy} onClick={() => void runTests()} testId="sp-test">
          Test saved providers
        </Btn>
      </div>
      {msg && (
        <Banner tone={msg.tone} onClose={() => setMsg(null)}>
          {msg.text}
        </Banner>
      )}
    </Section>
  );
}

/** Job editor: which non-global providers this job uses, and a placeholder check. */
export function JobSecretProviders({ jobId, value, onChange }: { jobId: string; value: string[]; onChange: (ids: string[]) => void }) {
  const [providers, setProviders] = useState<SecretProvider[]>([]);
  const [report, setReport] = useState<PlaceholderReport | null>(null);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    tauriInvoke<SecretSettings>("backup_get_secret_settings", undefined, EMPTY_SETTINGS)
      .then((s) => setProviders(normalize(s).providers))
      .catch(() => setProviders([]));
  }, []);
  const check = async () => {
    setErr(null);
    try {
      setReport(await tauriInvoke<PlaceholderReport>("backup_job_placeholders", { jobId }));
    } catch (e) {
      setErr(errorText(e));
    }
  };
  return (
    <Section title="Secrets" hint="Settings written as $name or containing ${name} are filled from secret providers at run time (Defaults & destinations → Secret providers).">
      {providers.length === 0 ? (
        <div className="text-xs text-zinc-500">No secret providers are configured.</div>
      ) : (
        <div className="flex gap-3 flex-wrap">
          {providers.map((p) => (
            <Check
              key={p.id}
              label={`${p.name}${p.global ? " (all jobs)" : ""}`}
              checked={p.global || value.includes(p.id)}
              onChange={(v) => !p.global && onChange(v ? [...value, p.id] : value.filter((x) => x !== p.id))}
            />
          ))}
        </div>
      )}
      {jobId && (
        <Btn testId="job-placeholders-check" onClick={() => void check()}>
          Check placeholders (saved job)
        </Btn>
      )}
      {report && (
        <ul className="text-xs" data-testid="job-placeholders">
          {report.placeholders.length === 0 && <li className="text-zinc-500">This job uses no placeholders.</li>}
          {report.placeholders.map((p) => (
            <li key={p.name} className={p.provider ? "text-green-600" : "text-red-600"}>
              ${p.name}: {p.provider ? `from ${p.provider}${p.from_cache ? " (cached)" : ""}` : p.error}
            </li>
          ))}
        </ul>
      )}
      {err && <div className="text-xs text-red-600">{err}</div>}
    </Section>
  );
}
