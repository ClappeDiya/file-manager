import { type CloudConfig } from "@/stores/backup-store";


export type FieldDef = { key: string; label: string; kind?: "text" | "number" | "bool" | "select" | "list"; options?: string[]; hint?: string; path?: string[] };


/** Transport options shared by every HTTP provider (`TlsOptions` in Rust). */
export const TLS_FIELDS: FieldDef[] = [
  { key: "accept_invalid_certs", label: "Accept invalid TLS certificates (test setups only)", kind: "bool", path: ["tls"] },
  { key: "ca_pem", label: "Trusted CA certificate (PEM)", path: ["tls"] },
  { key: "pinned_sha256", label: "Pinned certificate SHA-256 fingerprints", kind: "list", path: ["tls"], hint: "Only these certificates are accepted. Comma-separated; use Get fingerprint on the server." },
];


/** Per-provider fields and defaults (identical to the Rust `Default`s). */
export const BASE_FIELDS: Record<string, { defaults: Record<string, unknown>; fields: FieldDef[] }> = {
  s3: {
    defaults: { preset: "aws", endpoint: "", region: "us-east-1", bucket: "", prefix: "", access_key_id: "", path_style: false, storage_class: "", part_size_mb: 64, cold_storage: { restore_days: 3, tier: "Standard", wait_minutes: 0 }, create_with_object_lock: false, tls: { accept_invalid_certs: false, ca_pem: null, pinned_sha256: [] }, credentials: { type: "static" }, assume_role: null, storage_class_by_kind: { dblock: "", dindex: "", dlist: "", other: "" }, point_in_time: null },
    fields: [
      { key: "preset", label: "Service", kind: "select", options: ["aws", "idrive_e2", "storj", "aliyun_oss", "tencent_cos", "wasabi", "minio", "custom"] },
      { key: "endpoint", label: "Endpoint (empty = from service + region)", hint: "e.g. https://s3.example.com or http://127.0.0.1:9000" },
      { key: "region", label: "Region" },
      { key: "bucket", label: "Bucket" },
      { key: "prefix", label: "Folder in bucket" },
      { key: "access_key_id", label: "Access key id (static keys)" },
      { key: "storage_class", label: "Storage class", kind: "select", options: ["", "STANDARD", "STANDARD_IA", "ONEZONE_IA", "INTELLIGENT_TIERING", "GLACIER_IR", "GLACIER", "DEEP_ARCHIVE"] },
      { key: "part_size_mb", label: "Multipart part size (MiB, 0 = single upload)", kind: "number" },
      { key: "path_style", label: "Path-style addressing (bucket in the path)", kind: "bool" },
      { key: "create_with_object_lock", label: "Create the bucket with Object Lock enabled", kind: "bool" },
      { key: "restore_days", label: "Archive restore: days available", kind: "number", path: ["cold_storage"] },
      { key: "tier", label: "Archive restore tier", kind: "select", options: ["Standard", "Bulk", "Expedited"], path: ["cold_storage"] },
      { key: "wait_minutes", label: "Wait for archive restores (minutes, 0 = fail fast)", kind: "number", path: ["cold_storage"] },
    ],
  },
  azure: {
    defaults: { account: "", container: "", prefix: "", endpoint: "", access_tier: "", block_size_mb: 4, rehydrate_priority: "Standard", tls: { accept_invalid_certs: false, ca_pem: null, pinned_sha256: [] }, auth: { type: "shared_key" }, authority_host: "", access_tier_by_kind: { dblock: "", dindex: "", dlist: "", other: "" } },
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
    defaults: { bucket: "", prefix: "", project: "", storage_class: "", endpoint: "", checksums: true, tls: { accept_invalid_certs: false, ca_pem: null, pinned_sha256: [] }, scope: "", storage_class_by_kind: { dblock: "", dindex: "", dlist: "", other: "" } },
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
    defaults: { auth_url: "", auth_version: "3", username: "", user_domain: "Default", project: "", project_domain: "Default", region: "", container: "", prefix: "", tls: { accept_invalid_certs: false, ca_pem: null, pinned_sha256: [] } },
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
    defaults: { remote: "", path: "", binary: "rclone", config_file: null, extra_args: [], timeout_secs: 0, startup_timeout_secs: 0, env: {}, isolate_env: false, env_passthrough: [] },
    fields: [{ key: "remote", label: "Remote (from `rclone config`, e.g. mega)" }, { key: "path", label: "Folder" }, { key: "binary", label: "rclone executable" }],
  },
  filen: {
    defaults: { email: "", folder: "", use_trash: false, gateway_url: "", ingest_url: "", egest_url: "", tls: { accept_invalid_certs: false, ca_pem: null, pinned_sha256: [] } },
    fields: [
      { key: "email", label: "Account e-mail" },
      { key: "folder", label: "Folder", hint: "Encrypted in Filen's own format; readable in Filen's apps. Auth version 2 accounts." },
      { key: "use_trash", label: "Move deleted volumes to the Filen trash", kind: "bool" },
    ],
  },
  drime: {
    defaults: { folder: "", workspace_id: 0, page_size: 50, soft_delete: false, api_url: "", tls: { accept_invalid_certs: false, ca_pem: null, pinned_sha256: [] } },
    fields: [
      { key: "folder", label: "Folder" },
      { key: "workspace_id", label: "Workspace id (0 = personal drive)", kind: "number" },
      { key: "page_size", label: "Listing page size (1–100)", kind: "number" },
      { key: "soft_delete", label: "Move deleted volumes to the Drime trash", kind: "bool" },
    ],
  },
  movistar_cloud: {
    defaults: { email: "", device_id: "", folder: "", list_limit: 2000, wait_for_validation: true, validation_timeout_secs: 600, validation_poll_secs: 2, base_url: "", upload_url: "", tls: { accept_invalid_certs: false, ca_pem: null, pinned_sha256: [] } },
    fields: [
      { key: "email", label: "Account e-mail" },
      { key: "device_id", label: "Device id", hint: "The X-Deviceid request header of a signed-in micloud.movistar.es browser session (developer tools → Network)." },
      { key: "folder", label: "Folder" },
      { key: "list_limit", label: "Listing limit (items)", kind: "number" },
      { key: "wait_for_validation", label: "Wait until each upload is validated", kind: "bool" },
      { key: "validation_timeout_secs", label: "Validation timeout (seconds)", kind: "number" },
    ],
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


export const PROVIDER_FIELDS: Record<string, { defaults: Record<string, unknown>; fields: FieldDef[] }> = Object.fromEntries(
  Object.entries(BASE_FIELDS).map(([k, v]) => [k, "tls" in v.defaults ? { ...v, fields: [...v.fields, ...TLS_FIELDS] } : v]),
);


export function newCloudConfig(provider: string): CloudConfig {
  return { provider, ...structuredClone(PROVIDER_FIELDS[provider]?.defaults ?? {}) } as CloudConfig;
}
