/**
 * Cloud / network destination editor. Fields mirror the Rust provider
 * configs in `src-tauri/src/backup_engine/stores/*` (snake_case keys); the
 * secret is sent separately to the credential store and never kept in the
 * job.
 */
import { useEffect, useState } from "react";
import { tauriInvoke } from "@/hooks/use-tauri";
import { inputCls } from "@/lib/backup-helpers";
import { errorText, type CloudConfig, type CloudProviderInfo } from "@/stores/backup-store";
import { Banner, Btn, Check, Field } from "./backup-ui";

type FieldDef = { key: string; label: string; kind?: "text" | "number" | "bool" | "select"; options?: string[]; hint?: string; path?: string[] };

/** Per-provider fields and defaults (identical to the Rust `Default`s). */
export const PROVIDER_FIELDS: Record<string, { defaults: Record<string, unknown>; fields: FieldDef[] }> = {
  s3: {
    defaults: { preset: "aws", endpoint: "", region: "us-east-1", bucket: "", prefix: "", access_key_id: "", path_style: false, storage_class: "", part_size_mb: 64, cold_storage: { restore_days: 3, tier: "Standard", wait_minutes: 0 }, create_with_object_lock: false, tls: { accept_invalid_certs: false, ca_pem: null } },
    fields: [
      { key: "preset", label: "Service", kind: "select", options: ["aws", "idrive_e2", "storj", "aliyun_oss", "tencent_cos", "wasabi", "minio", "custom"] },
      { key: "endpoint", label: "Endpoint (empty = from service + region)", hint: "e.g. https://s3.example.com or http://127.0.0.1:9000" },
      { key: "region", label: "Region" },
      { key: "bucket", label: "Bucket" },
      { key: "prefix", label: "Folder in bucket" },
      { key: "access_key_id", label: "Access key id" },
      { key: "storage_class", label: "Storage class", kind: "select", options: ["", "STANDARD", "STANDARD_IA", "ONEZONE_IA", "INTELLIGENT_TIERING", "GLACIER_IR", "GLACIER", "DEEP_ARCHIVE"] },
      { key: "part_size_mb", label: "Multipart part size (MiB, 0 = single upload)", kind: "number" },
      { key: "path_style", label: "Path-style addressing (bucket in the path)", kind: "bool" },
      { key: "create_with_object_lock", label: "Create the bucket with Object Lock enabled", kind: "bool" },
      { key: "restore_days", label: "Archive restore: days available", kind: "number", path: ["cold_storage"] },
      { key: "tier", label: "Archive restore tier", kind: "select", options: ["Standard", "Bulk", "Expedited"], path: ["cold_storage"] },
      { key: "wait_minutes", label: "Wait for archive restores (minutes, 0 = fail fast)", kind: "number", path: ["cold_storage"] },
      { key: "accept_invalid_certs", label: "Accept invalid TLS certificates (test setups only)", kind: "bool", path: ["tls"] },
    ],
  },
  azure: {
    defaults: { account: "", container: "", prefix: "", endpoint: "", access_tier: "", block_size_mb: 4, rehydrate_priority: "Standard", tls: { accept_invalid_certs: false, ca_pem: null } },
    fields: [
      { key: "account", label: "Storage account" },
      { key: "container", label: "Container" },
      { key: "prefix", label: "Folder" },
      { key: "endpoint", label: "Endpoint (empty = <account>.blob.core.windows.net)" },
      { key: "access_tier", label: "Access tier", kind: "select", options: ["", "Hot", "Cool", "Cold", "Archive"] },
      { key: "block_size_mb", label: "Block upload size (MiB)", kind: "number" },
      { key: "rehydrate_priority", label: "Archive rehydration priority", kind: "select", options: ["Standard", "High"] },
    ],
  },
  gcs: {
    defaults: { bucket: "", prefix: "", project: "", storage_class: "", endpoint: "", checksums: true, tls: { accept_invalid_certs: false, ca_pem: null } },
    fields: [
      { key: "bucket", label: "Bucket" },
      { key: "prefix", label: "Folder" },
      { key: "project", label: "Project id (to create the bucket)" },
      { key: "storage_class", label: "Storage class", kind: "select", options: ["", "STANDARD", "NEARLINE", "COLDLINE", "ARCHIVE"] },
      { key: "endpoint", label: "Endpoint (empty = storage.googleapis.com)" },
      { key: "checksums", label: "Send and verify CRC32C/MD5 checksums", kind: "bool" },
    ],
  },
  b2: { defaults: { key_id: "", bucket: "", prefix: "", api_url: "" }, fields: [{ key: "key_id", label: "Key id" }, { key: "bucket", label: "Bucket" }, { key: "prefix", label: "Folder" }] },
  swift: {
    defaults: { auth_url: "", auth_version: "3", username: "", user_domain: "Default", project: "", project_domain: "Default", region: "", container: "", prefix: "", tls: { accept_invalid_certs: false, ca_pem: null } },
    fields: [
      { key: "auth_url", label: "Auth URL", hint: "Keystone: https://…/v3" },
      { key: "auth_version", label: "Auth version", kind: "select", options: ["3", "1"] },
      { key: "username", label: "User" },
      { key: "user_domain", label: "User domain" },
      { key: "project", label: "Project" },
      { key: "project_domain", label: "Project domain" },
      { key: "region", label: "Region" },
      { key: "container", label: "Container" },
      { key: "prefix", label: "Folder" },
    ],
  },
  dropbox: { defaults: { folder: "", api_base: "", content_base: "", region: "" }, fields: [{ key: "folder", label: "Folder" }] },
  google_drive: { defaults: { folder: "", api_base: "", content_base: "", region: "" }, fields: [{ key: "folder", label: "Folder" }] },
  box: { defaults: { folder: "", api_base: "", content_base: "", region: "" }, fields: [{ key: "folder", label: "Folder" }] },
  pcloud: { defaults: { folder: "", api_base: "", content_base: "", region: "us" }, fields: [{ key: "folder", label: "Folder" }, { key: "region", label: "Data region", kind: "select", options: ["us", "eu"] }] },
  onedrive: {
    defaults: { drive: "me", folder: "", api_base: "" },
    fields: [
      { key: "drive", label: "Drive", hint: "me (OneDrive), site:<site-id> (SharePoint), group:<group-id> (Microsoft 365 group) or drive:<drive-id>" },
      { key: "folder", label: "Folder" },
    ],
  },
  tahoe_lafs: { defaults: { node_url: "http://127.0.0.1:3456", dir_cap: "", path: "" }, fields: [{ key: "node_url", label: "Gateway URL" }, { key: "dir_cap", label: "Directory capability (URI:DIR2:…)" }, { key: "path", label: "Folder" }] },
  rclone: {
    defaults: { remote: "", path: "", binary: "rclone", config_file: null, extra_args: [] },
    fields: [{ key: "remote", label: "Remote (from `rclone config`, e.g. mega)" }, { key: "path", label: "Folder" }, { key: "binary", label: "rclone executable" }],
  },
  smb: {
    defaults: { server: "", share: "", path: "", username: "", domain: "", port: null, min_protocol: "SMB3", encrypt: false, binary: "smbclient" },
    fields: [
      { key: "server", label: "Server" },
      { key: "share", label: "Share" },
      { key: "path", label: "Folder" },
      { key: "username", label: "User" },
      { key: "domain", label: "Domain" },
      { key: "min_protocol", label: "Minimum protocol", kind: "select", options: ["SMB2", "SMB3", "SMB3_11"] },
      { key: "encrypt", label: "Require SMB3 encryption", kind: "bool" },
    ],
  },
};

export function newCloudConfig(provider: string): CloudConfig {
  return { provider, ...structuredClone(PROVIDER_FIELDS[provider]?.defaults ?? {}) } as CloudConfig;
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
      <Field label={`Credentials${info ? ` — ${info.secret}` : ""}`} hint={hasSecret ? "Stored. Enter a new value to replace it." : "Not stored yet."}>
        <textarea data-testid="cloud-secret" className={`${inputCls} font-mono`} rows={2} value={secret} onChange={(e) => setSecret(e.target.value)} autoComplete="off" />
      </Field>
      <Btn disabled={!secret} testId="cloud-secret-save" onClick={() => void saveSecret()}>
        Save credentials
      </Btn>
      {msg && <Banner tone={msg.tone} onClose={() => setMsg(null)}>{msg.text}</Banner>}
    </div>
  );
}
