/**
 * Sign-in and advanced settings of native cloud destinations whose shape is
 * not a flat field list: the S3 credential chain and assume-role, Azure
 * Microsoft Entra ID sign-in, storage class per volume kind, the GCS scope
 * and rclone process control. Values mirror the Rust configs in
 * `src-tauri/src/backup_engine/stores/*` (serde snake_case, `type` tags).
 */
import { inputCls } from "@/lib/backup-helpers";
import { AZURE_AUTH_LABELS, azureAuthOf, EMPTY_CLASS_BY_KIND, S3_CREDENTIAL_LABELS, s3CredentialsOf, type AzureAuth, type ClassByKind, type Obj, type S3AssumeRole, type S3Credentials } from "@/lib/backup-cloud-auth";
import type { CloudConfig } from "@/stores/backup-store";
import { AdvancedOnly } from "./advanced-only";
import { Check, Field, Section } from "./backup-ui";

const CLASS_OPTIONS: Record<string, string[]> = {
  s3: ["", "STANDARD", "STANDARD_IA", "ONEZONE_IA", "INTELLIGENT_TIERING", "GLACIER_IR"],
  azure: ["", "Hot", "Cool", "Cold"],
  gcs: ["", "STANDARD", "NEARLINE", "COLDLINE", "ARCHIVE"],
};

const TEXT = (label: string, value: string, onChange: (v: string) => void, testId: string, hint?: string, placeholder?: string) => (
  <Field label={label} hint={hint}>
    <input className={inputCls} value={value} placeholder={placeholder} onChange={(e) => onChange(e.target.value)} data-testid={testId} />
  </Field>
);

function S3CredentialsEditor({ value, onChange }: { value: S3Credentials; onChange: (v: S3Credentials) => void }) {
  return (
    <div className="space-y-2" data-testid="s3-credentials">
      <Field label="Credentials from">
        <select className={inputCls} value={value.type} onChange={(e) => onChange(s3CredentialsOf(e.target.value as S3Credentials["type"], value))} data-testid="s3-credential-source">
          {Object.entries(S3_CREDENTIAL_LABELS).map(([k, l]) => (
            <option key={k} value={k}>
              {l}
            </option>
          ))}
        </select>
      </Field>
      {value.type === "profile" && (
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
          {TEXT("Profile name (empty = AWS_PROFILE or default)", value.name, (name) => onChange({ ...value, name }), "s3-profile-name")}
          {TEXT("Credentials file (empty = ~/.aws/credentials)", value.file ?? "", (f) => onChange({ ...value, file: f || null }), "s3-profile-file")}
        </div>
      )}
      {value.type === "web_identity" && (
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
          {TEXT("Role ARN", value.role_arn, (role_arn) => onChange({ ...value, role_arn }), "s3-wi-role", undefined, "arn:aws:iam::123456789012:role/Backup")}
          {TEXT("Token file", value.token_file, (token_file) => onChange({ ...value, token_file }), "s3-wi-token")}
        </div>
      )}
      {value.type !== "static" && <p className="text-xs text-zinc-500">Temporary credentials are refreshed automatically before they expire.</p>}
    </div>
  );
}

function AssumeRoleEditor({ value, onChange }: { value: S3AssumeRole | null; onChange: (v: S3AssumeRole | null) => void }) {
  const v = value ?? { role_arn: "", external_id: null, session_name: null, duration_secs: null, sts_endpoint: null, region: null };
  const set = (p: Partial<S3AssumeRole>) => {
    const n = { ...v, ...p };
    onChange(n.role_arn.trim() ? n : null);
  };
  return (
    <Section title="Assume an IAM role" hint="STS AssumeRole with the credentials above; the role's temporary keys are cached and renewed.">
      <div className="grid grid-cols-1 sm:grid-cols-2 gap-2" data-testid="s3-assume-role">
        {TEXT("Role ARN (empty = off)", v.role_arn, (role_arn) => set({ role_arn }), "s3-role-arn", undefined, "arn:aws:iam::123456789012:role/Backup")}
        {TEXT("External ID", v.external_id ?? "", (x) => set({ external_id: x || null }), "s3-role-external-id")}
        {TEXT("Session name", v.session_name ?? "", (x) => set({ session_name: x || null }), "s3-role-session")}
        <Field label="Session length (s, 900–43200)">
          <input className={inputCls} type="number" min={900} max={43200} value={v.duration_secs ?? ""} placeholder="3600" onChange={(e) => set({ duration_secs: e.target.value === "" ? null : Math.max(0, Number(e.target.value)) })} data-testid="s3-role-duration" />
        </Field>
        {TEXT("STS endpoint (empty = regional)", v.sts_endpoint ?? "", (x) => set({ sts_endpoint: x || null }), "s3-role-sts")}
        {TEXT("STS region (empty = the bucket's)", v.region ?? "", (x) => set({ region: x || null }), "s3-role-region")}
      </div>
    </Section>
  );
}

function AzureAuthEditor({ value, onChange }: { value: AzureAuth; onChange: (v: AzureAuth) => void }) {
  return (
    <div className="space-y-2" data-testid="azure-auth">
      <Field label="Sign in with">
        <select className={inputCls} value={value.type} onChange={(e) => onChange(azureAuthOf(e.target.value as AzureAuth["type"], value))} data-testid="azure-auth-type">
          {Object.entries(AZURE_AUTH_LABELS).map(([k, l]) => (
            <option key={k} value={k}>
              {l}
            </option>
          ))}
        </select>
      </Field>
      {"tenant" in value && (
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
          {TEXT("Tenant (directory) id", value.tenant, (tenant) => onChange({ ...value, tenant }), "azure-tenant")}
          {TEXT("Client (application) id", value.client_id, (client_id) => onChange({ ...value, client_id }), "azure-client-id")}
          {value.type === "service_principal_certificate" &&
            TEXT("Certificate file (PEM with key + certificate, or PFX)", value.cert_path, (cert_path) => onChange({ ...value, cert_path }), "azure-cert-path")}
          {value.type === "federated_token" && TEXT("Federated token file", value.token_file, (token_file) => onChange({ ...value, token_file }), "azure-token-file", "AKS: the AZURE_FEDERATED_TOKEN_FILE path.")}
        </div>
      )}
      {value.type === "managed_identity" &&
        TEXT("User-assigned identity client id (empty = system-assigned)", value.client_id ?? "", (c) => onChange({ type: "managed_identity", client_id: c || null }), "azure-mi-client")}
      {!["shared_key", "sas"].includes(value.type) && <p className="text-xs text-zinc-500">The identity needs the "Storage Blob Data Contributor" role (Reader for read-only jobs) on the container.</p>}
    </div>
  );
}

/** Storage class / access tier per volume kind; archive tiers are not offered (refused by the store). */
export function ClassByKindEditor({ provider, value, onChange }: { provider: string; value: ClassByKind; onChange: (v: ClassByKind) => void }) {
  const opts = CLASS_OPTIONS[provider] ?? [""];
  const row = (k: keyof ClassByKind, label: string) => (
    <Field key={k} label={label}>
      <select className={inputCls} value={value[k]} onChange={(e) => onChange({ ...value, [k]: e.target.value })} data-testid={`class-${k}`}>
        {(opts.includes(value[k]) ? opts : [...opts, value[k]]).map((o) => (
          <option key={o} value={o}>
            {o || "(the single class)"}
          </option>
        ))}
      </select>
    </Field>
  );
  return (
    <Section title={provider === "azure" ? "Access tier per volume kind" : "Storage class per volume kind"} hint="Block volumes are read on restore; index and file lists on every rebuild and verification. Archive tiers (thaw before reading) are refused here.">
      <div className="grid grid-cols-2 sm:grid-cols-4 gap-2" data-testid="class-by-kind">
        {row("dblock", "Data blocks")}
        {row("dindex", "Indexes")}
        {row("dlist", "File lists")}
        {row("other", "Other files")}
      </div>
    </Section>
  );
}

function RcloneProcessEditor({ values, update }: { values: Obj; update: (patch: Obj) => void }) {
  const env = (values.env as Record<string, string>) ?? {};
  const envText = Object.entries(env)
    .map(([k, v]) => `${k}=${v}`)
    .join("\n");
  return (
    <Section title="rclone process" hint="Calls that hang or run too long are stopped together with anything rclone started.">
      <div className="grid grid-cols-1 sm:grid-cols-2 gap-2" data-testid="rclone-process">
        <Field label="Call timeout (s, 0 = none)">
          <input className={inputCls} type="number" min={0} value={Number(values.timeout_secs ?? 0)} onChange={(e) => update({ timeout_secs: Math.max(0, Math.floor(Number(e.target.value) || 0)) })} data-testid="rclone-timeout" />
        </Field>
        <Field label="Startup timeout (s, 0 = none)" hint="Stop rclone when it prints nothing for this long (e.g. waiting for a password).">
          <input className={inputCls} type="number" min={0} value={Number(values.startup_timeout_secs ?? 0)} onChange={(e) => update({ startup_timeout_secs: Math.max(0, Math.floor(Number(e.target.value) || 0)) })} data-testid="rclone-startup-timeout" />
        </Field>
        <Field label="Extra environment (NAME=value per line)">
          <textarea
            className={`${inputCls} font-mono`}
            rows={3}
            defaultValue={envText}
            onBlur={(e) =>
              update({
                env: Object.fromEntries(
                  e.target.value
                    .split("\n")
                    .map((l) => l.trim())
                    .filter((l) => l.includes("="))
                    .map((l) => [l.slice(0, l.indexOf("=")).trim(), l.slice(l.indexOf("=") + 1)]),
                ),
              })
            }
            data-testid="rclone-env"
          />
        </Field>
        <Field label="Pass these app variables through (comma-separated)">
          <input
            className={`${inputCls} font-mono`}
            value={((values.env_passthrough as string[]) ?? []).join(", ")}
            onChange={(e) => update({ env_passthrough: e.target.value.split(/[,\s]+/).filter(Boolean) })}
            data-testid="rclone-passthrough"
          />
        </Field>
      </div>
      <Check label="Isolated environment (only PATH, HOME, temp folders and the variables above)" checked={Boolean(values.isolate_env)} onChange={(v) => update({ isolate_env: v })} />
    </Section>
  );
}

/** The provider-specific blocks of the cloud destination form. */
export function CloudProviderExtras({ config, onChange }: { config: CloudConfig; onChange: (c: CloudConfig) => void }) {
  const values = config as unknown as Obj;
  const update = (patch: Obj) => onChange({ ...values, ...patch } as unknown as CloudConfig);
  const p = config.provider;
  return (
    <div className="space-y-2" data-testid="cloud-provider-extras">
      {p === "s3" && <S3CredentialsEditor value={(values.credentials as S3Credentials) ?? { type: "static" }} onChange={(credentials) => update({ credentials })} />}
      {p === "azure" && <AzureAuthEditor value={(values.auth as AzureAuth) ?? { type: "shared_key" }} onChange={(auth) => update({ auth })} />}
      <AdvancedOnly>
        {p === "s3" && <AssumeRoleEditor value={(values.assume_role as S3AssumeRole | null) ?? null} onChange={(assume_role) => update({ assume_role })} />}
        {p === "azure" && TEXT("Entra ID authority (empty = login.microsoftonline.com)", String(values.authority_host ?? ""), (authority_host) => update({ authority_host }), "azure-authority", "Sovereign clouds, e.g. https://login.microsoftonline.us")}
        {p === "gcs" && (
          <Field label="OAuth scope (service-account keys)" hint="Automatic: read-only for read-only jobs, full control otherwise.">
            <select className={inputCls} value={String(values.scope ?? "")} onChange={(e) => update({ scope: e.target.value })} data-testid="gcs-scope">
              <option value="">Automatic</option>
              <option value="https://www.googleapis.com/auth/devstorage.read_only">Read only</option>
              <option value="https://www.googleapis.com/auth/devstorage.read_write">Read and write</option>
              <option value="https://www.googleapis.com/auth/devstorage.full_control">Full control (object locks)</option>
            </select>
          </Field>
        )}
        {(p === "s3" || p === "gcs") && (
          <ClassByKindEditor provider={p} value={(values.storage_class_by_kind as ClassByKind) ?? EMPTY_CLASS_BY_KIND} onChange={(storage_class_by_kind) => update({ storage_class_by_kind })} />
        )}
        {p === "azure" && <ClassByKindEditor provider="azure" value={(values.access_tier_by_kind as ClassByKind) ?? EMPTY_CLASS_BY_KIND} onChange={(access_tier_by_kind) => update({ access_tier_by_kind })} />}
        {p === "rclone" && <RcloneProcessEditor values={values} update={update} />}
      </AdvancedOnly>
    </div>
  );
}
