/**
 * Connection Manager Panel (T-019 Frontend)
 *
 * Features:
 * - List saved connections grouped by folders
 * - Quick-connect bar with autocomplete
 * - One-click connection test
 * - Save/edit/delete connections
 * - Import/export connections
 * - Live connection status polling
 * - Dynamic protocol list from backend
 * - Delete connection groups
 * - Starred connections first; show/hide on password fields
 * - Google Drive / Dropbox / OneDrive sign-in and where they open
 * - "Credentials rejected" flag with Retry; remote quota; folder size
 */
import { useState, useCallback, useEffect, useMemo, useRef } from "react";
import { ShareLinkDialog } from "@/components/share-link-dialog";
import { SHAREABLE_PROTOCOLS } from "@/stores/share-store";
import { tauriInvoke } from "@/hooks/use-tauri";
import { formatBytes } from "@/lib/format-bytes";
import { pickFiles, pickFolder } from "@/lib/folder-picker";
import { promptText, confirmText } from "@/lib/prompt-dialog";
import { errorText } from "@/lib/error-text";
import { PRESET_FIELD_LABELS, fillPreset, type ProviderPreset } from "@/lib/provider-presets";
import { PasswordInput } from "@/components/password-input";
import { OAuthSignIn } from "@/components/oauth-sign-in";
import { CloudScopeFields, type CloudExtras, type OAuthSettings } from "@/components/cloud-connection-fields";
import { CONNECTIONS_CHANGED_EVENT, OAUTH_PROTOCOLS, googleDownloadName, isSignedIn, signOut } from "@/lib/oauth";
import { useUIStore } from "@/stores/ui-store";

// ── Types (mirrors Rust types) ──

interface ConnectionProfile {
  id: string;
  name: string;
  protocol: string;
  host: string;
  port: number | null;
  username: string | null;
  credential_ref: string | null;
  remote_path: string;
  created_at: string;
  last_used: string | null;
  group_id: string | null;
  bandwidth_limit_bps: number;
  verify_checksums: boolean;
  // Proxy configuration
  proxy_type: string | null;
  proxy_host: string | null;
  proxy_port: number | null;
  proxy_username: string | null;
  proxy_password: string | null;
  // Default paths
  default_local_dir: string | null;
  default_remote_dir: string | null;
  // Charset
  charset: string | null;
  // Upload-time permission defaults
  default_file_mode: string | null;
  default_dir_mode: string | null;
  /** Mirrors `ConnectionExtras` (core/types.rs). */
  extras?: ConnectionExtras;
}

interface AssumeRoleConfig {
  role_arn: string;
  external_id: string | null;
  session_name: string | null;
  duration_secs: number | null;
}

type SwiftAuthKind = "password" | "application_credential" | "temp_auth";

/** Mirrors `SwiftSettings` (core/types.rs). */
interface SwiftSettings {
  auth_kind: SwiftAuthKind;
  auth_url: string | null;
  project_name: string | null;
  domain_name: string | null;
  region: string | null;
  application_credential_id: string | null;
}

type WebDavAuthMode = "basic" | "bearer" | "none";
type WebDavVendor = "generic" | "nextcloud" | "owncloud";

/** Mirrors `WebDavSettings`. */
interface WebDavSettings {
  auth: WebDavAuthMode;
  vendor: WebDavVendor;
}

/** Mirrors `SmbSettings`. */
interface SmbSettings {
  domain: string | null;
}

interface ConnectionExtras extends CloudExtras {
  assume_role: AssumeRoleConfig | null;
  rclone_remote: string | null;
  preset: string | null;
  swift: SwiftSettings | null;
  webdav?: WebDavSettings | null;
  smb?: SmbSettings | null;
}

/** Mirrors `ConnectionStatus` (core/types.rs). */
interface ConnectionStatus {
  id: string;
  starred: boolean;
  credentials_rejected_at: string | null;
  credentials_rejected_reason: string | null;
}

/** Mirrors `RemoteQuota`. */
interface RemoteQuota {
  total: number | null;
  used: number | null;
  free: number | null;
  trashed: number | null;
}

/** Mirrors `FolderSize`. */
interface FolderSize {
  bytes: number;
  files: number;
  folders: number | null;
  complete: boolean;
}

type SizeState = { jobId: string; running: true } | { running: false; result?: FolderSize; error?: string };

interface ConnectionGroup {
  id: string;
  name: string;
  parent_id: string | null;
  sort_order: number;
  created_at: string;
}

interface ConnectionTestResult {
  reachable: boolean;
  latency_ms: number;
  error: string | null;
}

interface RemoteFileEntry {
  name: string;
  path: string;
  is_dir: boolean;
  size: number;
  modified: string | null;
  extension?: string | null;
}

interface ProtocolOption {
  value: string;
  label: string;
  port: number;
}

interface SshConfigResult {
  host: string;
  user: string | null;
  port: number | null;
  identity_file: string | null;
}

interface AwsProfile {
  name: string;
  access_key_id: string | null;
  region: string | null;
}

type ThirdPartySource = "filezilla" | "cyberduck" | "winscp" | "transmit";

/** Azure storage services: account + key/SAS/connection string + optional endpoint. */
const AZURE_PROTOCOLS = new Set(["azure_blob", "adls_gen2", "azure_files"]);
const AZURE_ENDPOINT_HINT: Record<string, string> = {
  azure_blob: "myaccount.blob.core.usgovcloudapi.net or http://127.0.0.1:10000/devstoreaccount1",
  adls_gen2: "myaccount.dfs.core.chinacloudapi.cn",
  azure_files: "myaccount.file.core.usgovcloudapi.net",
};

const FALLBACK_PROTOCOLS: ProtocolOption[] = [
  { value: "sftp", label: "SFTP", port: 22 },
  { value: "ftp", label: "FTP", port: 21 },
  { value: "ftps", label: "FTPS", port: 990 },
  { value: "webdav", label: "WebDAV", port: 443 },
  { value: "smb", label: "SMB", port: 445 },
  { value: "nfs", label: "NFS", port: 2049 },
  { value: "s3", label: "S3", port: 443 },
  { value: "azure_blob", label: "Azure Blob Storage", port: 443 },
  { value: "adls_gen2", label: "Azure Data Lake Storage Gen2", port: 443 },
  { value: "azure_files", label: "Azure Files", port: 443 },
  { value: "swift", label: "OpenStack Swift", port: 5000 },
  { value: "google_drive", label: "Google Drive", port: 443 },
  { value: "dropbox", label: "Dropbox", port: 443 },
  { value: "onedrive", label: "OneDrive", port: 443 },
  { value: "b2", label: "Backblaze B2", port: 443 },
  { value: "rclone", label: "rclone (GCS, Box, Mega, 70+ more)", port: 0 },
];

// Default port mapping for dynamically fetched protocols
const DEFAULT_PORTS: Record<string, number> = {
  sftp: 22,
  ftp: 21,
  ftps: 990,
  webdav: 443,
  smb: 445,
  nfs: 2049,
  s3: 443,
  azure_blob: 443,
  adls_gen2: 443,
  azure_files: 443,
  swift: 5000,
  google_drive: 443,
  dropbox: 443,
  onedrive: 443,
  b2: 443,
  https: 443,
  http: 80,
};


// ── Helpers ──

/** Sizes follow Settings → Appearance → Size units. */
const formatFileSize = (bytes: number): string => formatBytes(bytes);

/** "4 GB of 15 GB used · 11 GB free" from whatever the provider reports. */
function formatQuota(q: RemoteQuota): string {
  const parts: string[] = [];
  if (q.used != null) parts.push(q.total != null ? `${formatFileSize(q.used)} of ${formatFileSize(q.total)} used` : `${formatFileSize(q.used)} used`);
  else if (q.total != null) parts.push(`${formatFileSize(q.total)} total`);
  if (q.free != null) parts.push(`${formatFileSize(q.free)} free`);
  if (q.trashed) parts.push(`${formatFileSize(q.trashed)} in trash`);
  return parts.join(" · ");
}

function formatModified(dateStr: string): string {
  try {
    const d = new Date(dateStr);
    const now = new Date();
    const diffMs = now.getTime() - d.getTime();
    const diffDays = Math.floor(diffMs / (1000 * 60 * 60 * 24));
    if (diffDays === 0) return "today";
    if (diffDays === 1) return "yesterday";
    if (diffDays < 30) return `${diffDays}d ago`;
    return d.toLocaleDateString();
  } catch {
    return dateStr;
  }
}

// ── Quick Connect Bar ──

function QuickConnectBar({
  onConnect,
}: {
  onConnect: (id: string) => void;
}) {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<ConnectionProfile[]>([]);
  const [showResults, setShowResults] = useState(false);

  const search = useCallback(async (q: string) => {
    if (q.length < 1) {
      setResults([]);
      return;
    }
    try {
      const r = await tauriInvoke<ConnectionProfile[]>("search_connections", {
        query: q,
      });
      setResults(r);
    } catch (err) {
      console.error("Search failed:", err);
    }
  }, []);

  useEffect(() => {
    const timer = setTimeout(() => search(query), 150);
    return () => clearTimeout(timer);
  }, [query, search]);

  return (
    <div className="relative">
      <input
        type="text"
        value={query}
        onChange={(e) => {
          setQuery(e.target.value);
          setShowResults(true);
        }}
        onFocus={() => setShowResults(true)}
        onBlur={() => setTimeout(() => setShowResults(false), 200)}
        placeholder="Quick connect..."
        className="w-full text-sm px-3 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900 focus:outline-none focus:ring-2 focus:ring-blue-500"
        aria-label="Quick connect search"
      />
      {showResults && results.length > 0 && (
        <div className="absolute top-full left-0 right-0 mt-1 bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-md shadow-lg z-50 max-h-48 overflow-y-auto">
          {results.map((conn) => (
            <button
              key={conn.id}
              onClick={() => {
                onConnect(conn.id);
                setQuery("");
                setShowResults(false);
              }}
              className="w-full text-left px-3 py-2 hover:bg-gray-100 dark:hover:bg-gray-700 text-sm"
              type="button"
            >
              <div className="font-medium">{conn.name}</div>
              <div className="text-xs text-gray-500 dark:text-gray-400">
                {conn.protocol.toUpperCase()} - {conn.host}
                {conn.port ? `:${conn.port}` : ""}
              </div>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

// ── Connection Form ──

// Charset options for FTP/FTPS legacy servers
const CHARSET_OPTIONS = [
  { value: "utf-8", label: "UTF-8" },
  { value: "iso-8859-1", label: "Latin-1 (ISO-8859-1)" },
  { value: "windows-1252", label: "Windows-1252" },
  { value: "shift-jis", label: "Shift-JIS" },
  { value: "euc-kr", label: "EUC-KR" },
  { value: "gb2312", label: "GB2312" },
  { value: "auto", label: "Auto-detect" },
];

// ── SSH Key Generation Section ──

function SshKeyGenSection({
  onKeyGenerated,
}: {
  onKeyGenerated: (keyPath: string, passphrase: string | null) => void;
}) {
  const [showModal, setShowModal] = useState(false);
  const [algorithm, setAlgorithm] = useState("ed25519");
  const [passphrase, setPassphrase] = useState("");
  const [showPassphrase, setShowPassphrase] = useState(false);
  const [generating, setGenerating] = useState(false);
  const [result, setResult] = useState<{
    private_key_path: string;
    public_key_path: string;
    public_key_content: string;
    algorithm: string;
  } | null>(null);
  const [genError, setGenError] = useState<string | null>(null);

  const handleGenerate = async () => {
    setGenerating(true);
    setGenError(null);
    try {
      const res = await tauriInvoke<{
        private_key_path: string;
        public_key_path: string;
        public_key_content: string;
        algorithm: string;
      }>("generate_ssh_key", {
        algorithm,
        passphrase: showPassphrase ? passphrase || null : null,
        path: null,
        comment: null,
      });
      setResult(res);
      onKeyGenerated(res.private_key_path, showPassphrase && passphrase ? passphrase : null);
    } catch (e: any) {
      setGenError(e?.message || "Key generation failed");
    } finally {
      setGenerating(false);
    }
  };

  if (!showModal) {
    return (
      <button
        type="button"
        onClick={() => setShowModal(true)}
        className="text-xs text-blue-500 hover:text-blue-600 dark:text-blue-400 dark:hover:text-blue-300"
      >
        Generate New SSH Key
      </button>
    );
  }

  return (
    <div className="border border-blue-200 dark:border-blue-800 rounded-md p-3 bg-blue-50/50 dark:bg-blue-900/10 space-y-2">
      <div className="flex items-center justify-between">
        <span className="text-xs font-semibold text-blue-700 dark:text-blue-400">
          Generate SSH Key
        </span>
        <button
          type="button"
          onClick={() => { setShowModal(false); setResult(null); setGenError(null); }}
          className="text-xs text-gray-400 hover:text-gray-600"
        >
          Close
        </button>
      </div>

      {!result ? (
        <>
          <div>
            <label className="block text-xs font-medium mb-1">Algorithm</label>
            <select
              value={algorithm}
              onChange={(e) => setAlgorithm(e.target.value)}
              className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
            >
              <option value="ed25519">Ed25519 (recommended)</option>
              <option value="rsa4096">RSA-4096</option>
            </select>
          </div>

          <div>
            <label className="flex items-center gap-2 text-xs mb-1">
              <input
                type="checkbox"
                checked={showPassphrase}
                onChange={(e) => setShowPassphrase(e.target.checked)}
              />
              Add Passphrase
            </label>
            {showPassphrase && (
              <PasswordInput
                value={passphrase}
                onChange={(e) => setPassphrase(e.target.value)}
                placeholder="Optional passphrase"
                className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
              />
            )}
          </div>

          {genError && (
            <p className="text-xs text-red-500">{genError}</p>
          )}

          <button
            type="button"
            onClick={handleGenerate}
            disabled={generating}
            className="w-full text-xs px-3 py-1.5 rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-50"
          >
            {generating ? "Generating..." : "Generate Key"}
          </button>
        </>
      ) : (
        <div className="space-y-2">
          <p className="text-xs text-green-600 dark:text-green-400 font-medium">
            Key generated successfully!
          </p>
          <div className="text-xs space-y-1">
            <div>
              <span className="text-gray-500 dark:text-gray-400">Private:</span>{" "}
              <code className="text-[11px] bg-gray-100 dark:bg-gray-800 px-1 rounded">
                {result.private_key_path}
              </code>
            </div>
            <div>
              <span className="text-gray-500 dark:text-gray-400">Public:</span>{" "}
              <code className="text-[11px] bg-gray-100 dark:bg-gray-800 px-1 rounded">
                {result.public_key_path}
              </code>
            </div>
          </div>
          <div>
            <label className="block text-xs font-medium mb-1 text-gray-500 dark:text-gray-400">
              Public key (add to server&apos;s authorized_keys):
            </label>
            <textarea
              readOnly
              value={result.public_key_content}
              rows={2}
              className="w-full text-[10px] px-2 py-1 rounded border border-gray-300 dark:border-gray-600 bg-gray-50 dark:bg-gray-800 font-mono resize-none"
              onClick={(e) => (e.target as HTMLTextAreaElement).select()}
            />
          </div>
          <p className="text-[10px] text-gray-400">
            Key auto-selected for this connection.
          </p>
        </div>
      )}
    </div>
  );
}

function ConnectionForm({
  initialData,
  onSave,
  onCancel,
  groups,
  protocols,
}: {
  initialData?: ConnectionProfile | null;
  onSave: (data: Record<string, unknown>) => void;
  onCancel: () => void;
  groups: ConnectionGroup[];
  protocols: ProtocolOption[];
}) {
  const [name, setName] = useState(initialData?.name || "");
  const [protocol, setProtocol] = useState(initialData?.protocol || "sftp");
  const [host, setHost] = useState(initialData?.host || "");
  const [port, setPort] = useState<string>(
    initialData?.port?.toString() || ""
  );
  const [username, setUsername] = useState(initialData?.username || "");
  const [password, setPassword] = useState("");
  const [remotePath, setRemotePath] = useState(
    initialData?.remote_path || "/"
  );
  const [groupId, setGroupId] = useState(initialData?.group_id || "");

  // Advanced section toggle
  const [showAdvanced, setShowAdvanced] = useState(false);

  // Proxy fields
  const [proxyType, setProxyType] = useState(initialData?.proxy_type || "");
  const [proxyHost, setProxyHost] = useState(initialData?.proxy_host || "");
  const [proxyPort, setProxyPort] = useState<string>(
    initialData?.proxy_port?.toString() || ""
  );
  const [proxyUsername, setProxyUsername] = useState(
    initialData?.proxy_username || ""
  );
  const [proxyPassword, setProxyPassword] = useState(
    initialData?.proxy_password || ""
  );

  // Default paths
  const [defaultLocalDir, setDefaultLocalDir] = useState(
    initialData?.default_local_dir || ""
  );
  const [defaultRemoteDir, setDefaultRemoteDir] = useState(
    initialData?.default_remote_dir || ""
  );

  // Charset
  const [charset, setCharset] = useState(initialData?.charset || "utf-8");

  // FTP data type (Binary/ASCII/Auto)
  const [ftpDataType, setFtpDataType] = useState(
    (initialData as any)?.ftp_data_type || "binary"
  );

  // Upload-time permission defaults
  const [defaultFileMode, setDefaultFileMode] = useState(
    initialData?.default_file_mode || ""
  );
  const [defaultDirMode, setDefaultDirMode] = useState(
    initialData?.default_dir_mode || ""
  );

  // SSH config auto-fill
  const [sshConfigHint, setSshConfigHint] = useState<string | null>(null);

  // AWS profile selector (S3 only)
  const [awsProfiles, setAwsProfiles] = useState<AwsProfile[]>([]);
  const [selectedAwsProfile, setSelectedAwsProfile] = useState("");
  const [awsProfilesLoaded, setAwsProfilesLoaded] = useState(false);

  // S3-compatible preset selector
  const [selectedPreset, setSelectedPreset] = useState(initialData?.extras?.preset ?? "");
  const [presets, setPresets] = useState<ProviderPreset[]>([]);
  const [presetValues, setPresetValues] = useState<Record<string, string>>({});
  useEffect(() => {
    tauriInvoke<ProviderPreset[]>("connection_presets", undefined, []).then((p) => setPresets(p ?? []), () => setPresets([]));
  }, []);

  // rclone remotes from the user's rclone config
  const [rcloneRemotes, setRcloneRemotes] = useState<{ name: string; backend: string }[] | null>(null);
  const [rcloneError, setRcloneError] = useState<string | null>(null);
  useEffect(() => {
    if (protocol !== "rclone" || rcloneRemotes !== null) return;
    tauriInvoke<{ name: string; backend: string }[]>("rclone_list_remotes")
      .then((r) => setRcloneRemotes(r ?? []))
      .catch((e) => {
        setRcloneRemotes([]);
        setRcloneError(errorText(e));
      });
  }, [protocol, rcloneRemotes]);

  // STS AssumeRole (S3): temporary credentials from the keys above
  const [roleArn, setRoleArn] = useState(initialData?.extras?.assume_role?.role_arn ?? "");
  const [externalId, setExternalId] = useState(initialData?.extras?.assume_role?.external_id ?? "");
  const [roleDuration, setRoleDuration] = useState(String(initialData?.extras?.assume_role?.duration_secs ?? ""));

  // Azure (Blob, Data Lake, Files): the account is the user name, the
  // endpoint the host and the key / SAS / connection string the password.
  const [azureAuthMethod, setAzureAuthMethod] = useState<"account_key" | "sas_token" | "connection_string">("account_key");

  // OpenStack Swift fields
  const initialSwift = initialData?.extras?.swift;
  const [swiftAuthKind, setSwiftAuthKind] = useState<SwiftAuthKind>(initialSwift?.auth_kind ?? "password");
  const [swiftAuthUrl, setSwiftAuthUrl] = useState(initialSwift?.auth_url ?? "");
  const [swiftProjectName, setSwiftProjectName] = useState(initialSwift?.project_name ?? "");
  const [swiftDomainName, setSwiftDomainName] = useState(initialSwift?.domain_name ?? "Default");
  const [swiftRegion, setSwiftRegion] = useState(initialSwift?.region ?? "");
  const [swiftCredentialId, setSwiftCredentialId] = useState(initialSwift?.application_credential_id ?? "");

  // SFTP private-key passphrase (stored in the credential store; blank = unchanged)
  const [keyPassphrase, setKeyPassphrase] = useState("");
  // Editing: forget the stored passphrase (sends "" — the backend deletes it)
  const [removeKeyPassphrase, setRemoveKeyPassphrase] = useState(false);

  // WebDAV sign-in mode and server vendor
  const [webdavAuth, setWebdavAuth] = useState<WebDavAuthMode>(initialData?.extras?.webdav?.auth ?? "basic");
  const [webdavVendor, setWebdavVendor] = useState<WebDavVendor>(initialData?.extras?.webdav?.vendor ?? "generic");

  // SMB Windows domain
  const [smbDomain, setSmbDomain] = useState(initialData?.extras?.smb?.domain ?? "");

  // Google Drive / OneDrive / Dropbox: app, account kind and where it opens
  const [cloud, setCloud] = useState<CloudExtras>({
    oauth: initialData?.extras?.oauth ?? null,
    google_drive: initialData?.extras?.google_drive ?? null,
    onedrive: initialData?.extras?.onedrive ?? null,
    dropbox: initialData?.extras?.dropbox ?? null,
  });
  const advancedView = useUIStore((st) => st.appMode === "advanced");

  // Auto-expand Advanced section when editing a connection that has advanced fields set
  useEffect(() => {
    if (
      initialData &&
      (initialData.proxy_type ||
        initialData.default_local_dir ||
        initialData.default_remote_dir ||
        initialData.default_file_mode ||
        initialData.default_dir_mode ||
        (initialData.charset && initialData.charset !== "utf-8"))
    ) {
      setShowAdvanced(true);
    }
  }, [initialData]);

  // Fetch AWS profiles when protocol is S3
  useEffect(() => {
    if (protocol === "s3" && !awsProfilesLoaded) {
      setAwsProfilesLoaded(true);
      tauriInvoke<AwsProfile[]>("list_aws_profiles", undefined, [])
        .then((profiles) => setAwsProfiles(profiles))
        .catch((err) => console.error("Failed to load AWS profiles:", err));
    }
  }, [protocol, awsProfilesLoaded]);

  // SSH config auto-fill on host blur
  const handleHostBlur = useCallback(async () => {
    if (protocol !== "sftp" || !host.trim()) return;
    try {
      const result = await tauriInvoke<SshConfigResult | null>(
        "resolve_ssh_config",
        { hostAlias: host.trim() },
        null
      );
      if (result) {
        if (result.user && !username) setUsername(result.user);
        if (result.port && !port) setPort(result.port.toString());
        if (result.identity_file && !password) {
          // Store identity file path as credential hint
          setPassword(result.identity_file);
        }
        setSshConfigHint("from SSH config");
      } else {
        setSshConfigHint(null);
      }
    } catch {
      // SSH config resolution is best-effort
      setSshConfigHint(null);
    }
  }, [protocol, host, username, port, password]);

  // Handle AWS profile selection
  const handleAwsProfileSelect = useCallback(
    (profileName: string) => {
      setSelectedAwsProfile(profileName);
      const profile = awsProfiles.find((p) => p.name === profileName);
      if (profile) {
        if (profile.access_key_id && !username) setUsername(profile.access_key_id);
        if (profile.region && !host) setHost(`s3.${profile.region}.amazonaws.com`);
      }
    },
    [awsProfiles, username, host]
  );

  // Handle S3 preset selection
  // Apply a provider preset: host (and port) from its template.
  const applyPreset = useCallback(
    (presetId: string, values: Record<string, string>) => {
      const preset = presets.find((p) => p.id === presetId);
      if (!preset) return;
      const vals = { region: preset.default_region ?? "", ...values };
      setHost(fillPreset(preset.host, vals));
      if (preset.port) setPort(String(preset.port));
    },
    [presets],
  );
  const isFtpProtocol = protocol === "ftp" || protocol === "ftps";
  const isSshProtocol = protocol === "sftp";
  const isAzure = AZURE_PROTOCOLS.has(protocol);
  const isRclone = protocol === "rclone";
  const isSwift = protocol === "swift";
  const isS3 = protocol === "s3";
  const isWebDav = protocol === "webdav";
  const isSmb = protocol === "smb";
  const isB2 = protocol === "b2";
  const isOAuth = OAUTH_PROTOCOLS.has(protocol);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    onSave({
      connectionId: initialData?.id || null,
      name,
      protocol,
      host,
      port: port ? parseInt(port, 10) : null,
      username: username || null,
      password: password || null,
      remotePath,
      groupId: groupId || null,
      // Proxy fields
      proxyType: proxyType || null,
      proxyHost: proxyType ? proxyHost || null : null,
      proxyPort: proxyType && proxyPort ? parseInt(proxyPort, 10) : null,
      proxyUsername: proxyType ? proxyUsername || null : null,
      proxyPassword: proxyType ? proxyPassword || null : null,
      // Default paths
      defaultLocalDir: defaultLocalDir || null,
      defaultRemoteDir: defaultRemoteDir || null,
      // Charset
      charset: isFtpProtocol ? charset || null : null,
      // FTP data type
      ftpDataType: isFtpProtocol ? ftpDataType : null,
      // Upload-time permission defaults (SFTP/FTP only)
      defaultFileMode: isSshProtocol || isFtpProtocol ? defaultFileMode || null : null,
      defaultDirMode: isSshProtocol || isFtpProtocol ? defaultDirMode || null : null,
      // SFTP key passphrase: sent when typed, "" removes the stored one,
      // null keeps it
      keyPassphrase: !isSshProtocol ? null : removeKeyPassphrase ? "" : keyPassphrase || null,
      extras: {
        // Keep settings this form does not show (other protocols, pickers).
        ...(initialData?.extras ?? {}),
        rclone_remote: protocol === "rclone" ? host.trim() || null : initialData?.extras?.rclone_remote ?? null,
        preset: selectedPreset || null,
        assume_role:
          isS3 && roleArn.trim()
            ? { role_arn: roleArn.trim(), external_id: externalId.trim() || null, session_name: null, duration_secs: roleDuration ? Number(roleDuration) : null }
            : null,
        swift: isSwift
          ? {
              auth_kind: swiftAuthKind,
              auth_url: swiftAuthUrl.trim() || null,
              project_name: swiftAuthKind === "password" ? swiftProjectName.trim() || null : null,
              domain_name: swiftAuthKind === "password" ? swiftDomainName.trim() || null : null,
              region: swiftAuthKind === "temp_auth" ? null : swiftRegion.trim() || null,
              application_credential_id: swiftAuthKind === "application_credential" ? swiftCredentialId.trim() || null : null,
            }
          : null,
        ...(isWebDav ? { webdav: { auth: webdavAuth, vendor: webdavVendor } } : {}),
        ...(isSmb ? { smb: { domain: smbDomain.trim() || null } } : {}),
        ...(isOAuth
          ? {
              oauth: cloud.oauth ? { ...cloud.oauth, client_id: cloud.oauth.client_id?.trim() || null } : null,
              google_drive: protocol === "google_drive" ? cloud.google_drive ?? null : initialData?.extras?.google_drive,
              onedrive: protocol === "onedrive" ? cloud.onedrive ?? null : initialData?.extras?.onedrive,
              dropbox: protocol === "dropbox" ? cloud.dropbox ?? null : initialData?.extras?.dropbox,
            }
          : {}),
      } satisfies ConnectionExtras,
    });
  };

  return (
    <form onSubmit={handleSubmit} className="space-y-3 p-3">
      <div>
        <label className="block text-xs font-medium mb-1">Name</label>
        <input
          type="text"
          value={name}
          onChange={(e) => setName(e.target.value)}
          required
          className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
        />
      </div>
      <div className="grid grid-cols-2 gap-2">
        <div>
          <label className="block text-xs font-medium mb-1">Protocol</label>
          <select
            value={protocol}
            onChange={(e) => {
              setProtocol(e.target.value);
              const p = protocols.find((p) => p.value === e.target.value);
              if (p && !port) setPort(p.port.toString());
            }}
            className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
          >
            {protocols.map((p) => (
              <option key={p.value} value={p.value}>
                {p.label}
              </option>
            ))}
          </select>
        </div>
        {!isOAuth && !isB2 && (
        <div>
          <label className="block text-xs font-medium mb-1">Port</label>
          <input
            type="number"
            value={port}
            onChange={(e) => setPort(e.target.value)}
            placeholder="Auto"
            className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
          />
        </div>
        )}
      </div>
      {isOAuth && (
        <p className="text-xs text-gray-500 dark:text-gray-400" data-testid="oauth-form-hint">
          {initialData ? "Use Sign in on the connection to change the account." : "After saving, sign in with your browser; no password is stored."}
        </p>
      )}
      {!isAzure && !isOAuth && (
      <>
      {!isB2 && (
      <div>
        <div className="flex items-center gap-1 mb-1">
          <label className="block text-xs font-medium">Host</label>
          {sshConfigHint && (
            <span className="text-[10px] text-gray-400 dark:text-gray-500 italic">
              ({sshConfigHint})
            </span>
          )}
        </div>
        <input
          type="text"
          value={host}
          onChange={(e) => {
            setHost(e.target.value);
            setSshConfigHint(null);
          }}
          onBlur={handleHostBlur}
          required={!(isSwift && swiftAuthUrl.trim())}
          placeholder="example.com"
          className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
        />
      </div>
      )}
      {isWebDav && (
        <div className="grid grid-cols-2 gap-2" data-testid="webdav-fields">
          <div>
            <label className="block text-xs font-medium mb-1" htmlFor="webdav-auth">Sign in with</label>
            <select id="webdav-auth" value={webdavAuth} onChange={(e) => setWebdavAuth(e.target.value as WebDavAuthMode)} className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900">
              <option value="basic">User name and password</option>
              <option value="bearer">Bearer token</option>
              <option value="none">No sign-in</option>
            </select>
          </div>
          <div>
            <label className="block text-xs font-medium mb-1" htmlFor="webdav-vendor">Server</label>
            <select id="webdav-vendor" value={webdavVendor} onChange={(e) => setWebdavVendor(e.target.value as WebDavVendor)} className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900">
              <option value="generic">Generic WebDAV</option>
              <option value="nextcloud">Nextcloud</option>
              <option value="owncloud">ownCloud</option>
            </select>
          </div>
        </div>
      )}
      {!(isWebDav && webdavAuth === "none") && (
      <div className="grid grid-cols-2 gap-2">
        {!(isWebDav && webdavAuth === "bearer") && (
        <div>
          <label className="block text-xs font-medium mb-1" htmlFor="conn-username">{isB2 ? "Application key ID" : "Username"}</label>
          <input
            id="conn-username"
            type="text"
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            required={isB2}
            className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
          />
        </div>
        )}
        <div>
          <label className="block text-xs font-medium mb-1" htmlFor="conn-password">{isB2 ? "Application key" : isWebDav && webdavAuth === "bearer" ? "Bearer token" : "Password"}</label>
          <PasswordInput
            id="conn-password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            placeholder={initialData ? "(unchanged)" : ""}
            className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
          />
        </div>
      </div>
      )}
      {isSmb && (
        <div>
          <label className="block text-xs font-medium mb-1" htmlFor="smb-domain">
            Domain
            <span className="text-gray-400 dark:text-gray-500 font-normal"> (optional)</span>
          </label>
          <input id="smb-domain" type="text" value={smbDomain} onChange={(e) => setSmbDomain(e.target.value)} placeholder="CORP" className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900" />
        </div>
      )}
      </>
      )}
      {isOAuth && (
        <>
          <CloudScopeFields protocol={protocol} connectionId={initialData?.id ?? null} value={cloud} onChange={setCloud} />
          {advancedView && (
            <div>
              <label className="block text-xs font-medium mb-1" htmlFor="oauth-app-client-id">
                OAuth client ID
                <span className="text-gray-400 dark:text-gray-500 font-normal"> (your own app; blank = built-in)</span>
              </label>
              <input
                id="oauth-app-client-id"
                value={cloud.oauth?.client_id ?? ""}
                onChange={(e) => setCloud((c) => ({ ...c, oauth: { ...(c.oauth ?? ({ account_kind: "personal" } as OAuthSettings)), client_id: e.target.value } }))}
                className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900 font-mono"
              />
            </div>
          )}
        </>
      )}
      {/* SSH Key Generation (SFTP only) */}
      {isSshProtocol && (
        <SshKeyGenSection
          onKeyGenerated={(keyPath, keyPass) => {
            setPassword(keyPath);
            // The new key's passphrase unlocks it when connecting.
            if (keyPass) setKeyPassphrase(keyPass);
            setSshConfigHint("key generated");
          }}
        />
      )}
      {isSshProtocol && (
        <div>
          <label className="block text-xs font-medium mb-1" htmlFor="sftp-key-passphrase">
            Key passphrase
            <span className="text-gray-400 dark:text-gray-500 font-normal"> (if the private key has one)</span>
          </label>
          <PasswordInput
            id="sftp-key-passphrase"
            value={removeKeyPassphrase ? "" : keyPassphrase}
            onChange={(e) => setKeyPassphrase(e.target.value)}
            placeholder={initialData ? (removeKeyPassphrase ? "(will be removed)" : "(unchanged)") : ""}
            disabled={removeKeyPassphrase}
            className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900 disabled:opacity-50"
          />
          {initialData && (
            <label className="mt-1 flex items-center gap-1.5 text-xs text-gray-600 dark:text-gray-400">
              <input type="checkbox" checked={removeKeyPassphrase} onChange={(e) => setRemoveKeyPassphrase(e.target.checked)} />
              Remove saved passphrase
            </label>
          )}
        </div>
      )}
      {/* AWS Profile selector (S3 only) */}
      {protocol === "s3" && awsProfiles.length > 0 && (
        <div>
          <label className="block text-xs font-medium mb-1">AWS Profile</label>
          <select
            value={selectedAwsProfile}
            onChange={(e) => handleAwsProfileSelect(e.target.value)}
            className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
          >
            <option value="">Select a profile...</option>
            {awsProfiles.map((p) => (
              <option key={p.name} value={p.name}>
                {p.name}
                {p.region ? ` (${p.region})` : ""}
              </option>
            ))}
          </select>
          <p className="text-[10px] text-gray-400 dark:text-gray-500 mt-0.5">
            Auto-fills access key and region from ~/.aws/credentials
          </p>
        </div>
      )}
      {/* Provider presets (S3-compatible stores, hosted SFTP) */}
      {presets.some((p) => p.protocol === protocol) && (() => {
        const preset = presets.find((p) => p.id === selectedPreset && p.protocol === protocol);
        return (
          <div className="space-y-1">
            <label className="block text-xs font-medium mb-1" htmlFor="provider-preset">Provider</label>
            <select
              id="provider-preset"
              value={preset ? selectedPreset : ""}
              onChange={(e) => {
                setSelectedPreset(e.target.value);
                setPresetValues({});
                applyPreset(e.target.value, {});
              }}
              className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
            >
              <option value="">{isS3 ? "Custom / Amazon S3" : "Custom server"}</option>
              {presets.filter((p) => p.protocol === protocol).map((p) => (
                <option key={p.id} value={p.id}>{p.label}</option>
              ))}
            </select>
            {preset?.fields.map((f) => (
              <div key={f}>
                <label className="block text-xs font-medium mb-1" htmlFor={`preset-${f}`}>{PRESET_FIELD_LABELS[f] ?? f}</label>
                <input
                  id={`preset-${f}`}
                  value={presetValues[f] ?? (f === "region" ? preset.default_region ?? "" : "")}
                  onChange={(e) => {
                    const next = { ...presetValues, [f]: e.target.value };
                    setPresetValues(next);
                    applyPreset(preset.id, next);
                  }}
                  className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
                />
              </div>
            ))}
            {preset?.username && (
              <p className="text-[10px] text-blue-500 dark:text-blue-400" data-testid="preset-username">
                Username format: {fillPreset(preset.username, { ...presetValues, user: "your-username" })}
              </p>
            )}
            {preset?.help && <p className="text-[10px] text-blue-500 dark:text-blue-400">{preset.help}</p>}
          </div>
        );
      })()}
      {/* rclone remote picker */}
      {isRclone && (
        <div>
          <label className="block text-xs font-medium mb-1" htmlFor="rclone-remote">rclone remote</label>
          <select
            id="rclone-remote"
            value={rcloneRemotes?.some((r) => r.name === host) ? host : ""}
            onChange={(e) => {
              setHost(e.target.value);
              if (!name) setName(e.target.value.replace(/:$/, ""));
            }}
            className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
          >
            <option value="">{rcloneRemotes === null ? "Loading…" : rcloneRemotes.length ? "Choose a remote…" : "No remotes configured"}</option>
            {rcloneRemotes?.map((r) => (
              <option key={r.name} value={r.name}>{r.name} ({r.backend})</option>
            ))}
          </select>
          <p className="text-[10px] text-gray-400 dark:text-gray-500 mt-0.5">
            {rcloneError ?? "Remotes come from `rclone config`. You can also type one in Host, e.g. gcs:my-bucket or :s3,provider=AWS:."}
          </p>
        </div>
      )}
      {/* STS AssumeRole (S3 only) */}
      {isS3 && (
        <details className="rounded border border-gray-200 dark:border-gray-700 p-2" open={!!roleArn}>
          <summary className="text-xs font-medium cursor-pointer">Assume an IAM role</summary>
          <div className="space-y-2 mt-2">
            <div>
              <label className="block text-xs font-medium mb-1" htmlFor="role-arn">Role ARN</label>
              <input id="role-arn" value={roleArn} onChange={(e) => setRoleArn(e.target.value)} placeholder="arn:aws:iam::123456789012:role/Reader" className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900 font-mono" />
            </div>
            <div className="grid grid-cols-2 gap-2">
              <div>
                <label className="block text-xs font-medium mb-1" htmlFor="role-external-id">External ID</label>
                <input id="role-external-id" value={externalId} onChange={(e) => setExternalId(e.target.value)} placeholder="optional" className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900" />
              </div>
              <div>
                <label className="block text-xs font-medium mb-1" htmlFor="role-duration">Session length (s)</label>
                <input id="role-duration" type="number" min={900} max={43200} value={roleDuration} onChange={(e) => setRoleDuration(e.target.value)} placeholder="3600" className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900" />
              </div>
            </div>
            <p className="text-[10px] text-gray-400 dark:text-gray-500">
              The access key and secret above sign the AssumeRole call; the app uses the temporary credentials and renews them before they expire.
            </p>
          </div>
        </details>
      )}
      {/* Azure Blob / Data Lake / Files fields */}
      {isAzure && (
        <div className="space-y-2" data-testid="azure-fields">
          <div>
            <label className="block text-xs font-medium mb-1" htmlFor="azure-account">Storage account</label>
            <input
              id="azure-account"
              type="text"
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              placeholder={azureAuthMethod === "connection_string" ? "(from the connection string)" : "mystorageaccount"}
              className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
            />
          </div>
          <div>
            <label className="block text-xs font-medium mb-1" htmlFor="azure-auth">Sign in with</label>
            <select
              id="azure-auth"
              value={azureAuthMethod}
              onChange={(e) => setAzureAuthMethod(e.target.value as typeof azureAuthMethod)}
              className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
            >
              <option value="account_key">Account key</option>
              <option value="sas_token">SAS token</option>
              <option value="connection_string">Connection string</option>
            </select>
          </div>
          <div>
            <label className="block text-xs font-medium mb-1" htmlFor="azure-secret">
              {azureAuthMethod === "account_key" ? "Account key" : azureAuthMethod === "sas_token" ? "SAS token" : "Connection string"}
            </label>
            <PasswordInput
              id="azure-secret"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              placeholder={initialData ? "(unchanged)" : azureAuthMethod === "connection_string" ? "DefaultEndpointsProtocol=https;AccountName=…" : azureAuthMethod === "sas_token" ? "sv=…&sig=…" : ""}
              className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
            />
          </div>
          <div>
            <label className="block text-xs font-medium mb-1" htmlFor="azure-endpoint">
              Endpoint
              <span className="text-gray-400 dark:text-gray-500 font-normal"> (optional)</span>
            </label>
            <input
              id="azure-endpoint"
              type="text"
              value={host}
              onChange={(e) => setHost(e.target.value)}
              placeholder={AZURE_ENDPOINT_HINT[protocol]}
              className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
            />
            <p className="text-[10px] text-gray-400 dark:text-gray-500 mt-0.5">
              Leave blank for the public Azure cloud. For Azure Government, Azure China or the Azurite emulator, enter the host or URL.
            </p>
          </div>
        </div>
      )}
      {/* OpenStack Swift fields */}
      {isSwift && (
        <div className="space-y-2" data-testid="swift-fields">
          <div>
            <label className="block text-xs font-medium mb-1" htmlFor="swift-auth-kind">Sign in with</label>
            <select
              id="swift-auth-kind"
              value={swiftAuthKind}
              onChange={(e) => setSwiftAuthKind(e.target.value as SwiftAuthKind)}
              className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
            >
              <option value="password">Keystone user name and password</option>
              <option value="application_credential">Keystone application credential</option>
              <option value="temp_auth">TempAuth (v1)</option>
            </select>
          </div>
          <div>
            <label className="block text-xs font-medium mb-1" htmlFor="swift-auth-url">Auth URL</label>
            <input
              id="swift-auth-url"
              type="text"
              value={swiftAuthUrl}
              onChange={(e) => setSwiftAuthUrl(e.target.value)}
              placeholder={swiftAuthKind === "temp_auth" ? "https://swift.example.com/auth/v1.0" : "https://keystone.example.com:5000/v3"}
              className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
            />
            <p className="text-[10px] text-gray-400 dark:text-gray-500 mt-0.5">
              Leave blank to use the host and port above{swiftAuthKind === "temp_auth" ? " with /auth/v1.0" : " with /v3"}.
            </p>
          </div>
          {swiftAuthKind === "application_credential" && (
            <div>
              <label className="block text-xs font-medium mb-1" htmlFor="swift-credential-id">Application credential ID</label>
              <input
                id="swift-credential-id"
                type="text"
                value={swiftCredentialId}
                onChange={(e) => setSwiftCredentialId(e.target.value)}
                className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
              />
              <p className="text-[10px] text-gray-400 dark:text-gray-500 mt-0.5">Enter the credential secret as the password; the credential's own project applies.</p>
            </div>
          )}
          {swiftAuthKind === "password" && (
            <div className="grid grid-cols-2 gap-2">
              <div>
                <label className="block text-xs font-medium mb-1" htmlFor="swift-project">Project name</label>
                <input
                  id="swift-project"
                  type="text"
                  value={swiftProjectName}
                  onChange={(e) => setSwiftProjectName(e.target.value)}
                  placeholder="my-project"
                  className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
                />
              </div>
              <div>
                <label className="block text-xs font-medium mb-1" htmlFor="swift-domain">Domain</label>
                <input
                  id="swift-domain"
                  type="text"
                  value={swiftDomainName}
                  onChange={(e) => setSwiftDomainName(e.target.value)}
                  placeholder="Default"
                  className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
                />
              </div>
            </div>
          )}
          {swiftAuthKind === "temp_auth" ? (
            <p className="text-[10px] text-gray-400 dark:text-gray-500">Enter the user as account:user and the key as the password.</p>
          ) : (
            <div>
              <label className="block text-xs font-medium mb-1" htmlFor="swift-region">
                Region
                <span className="text-gray-400 dark:text-gray-500 font-normal"> (optional)</span>
              </label>
              <input
                id="swift-region"
                type="text"
                value={swiftRegion}
                onChange={(e) => setSwiftRegion(e.target.value)}
                placeholder="RegionOne"
                className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
              />
            </div>
          )}
        </div>
      )}
      <div>
        <label className="block text-xs font-medium mb-1">Remote Path</label>
        <input
          type="text"
          value={remotePath}
          onChange={(e) => setRemotePath(e.target.value)}
          className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
        />
      </div>
      {groups.length > 0 && (
        <div>
          <label className="block text-xs font-medium mb-1">Group</label>
          <select
            value={groupId}
            onChange={(e) => setGroupId(e.target.value)}
            className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
          >
            <option value="">None</option>
            {groups.map((g) => (
              <option key={g.id} value={g.id}>
                {g.name}
              </option>
            ))}
          </select>
        </div>
      )}

      {/* ── Advanced Section (collapsible) ── */}
      <div className="border-t border-gray-200 dark:border-gray-700 pt-2">
        <button
          type="button"
          onClick={() => setShowAdvanced(!showAdvanced)}
          className="flex items-center gap-1 text-xs font-medium text-gray-500 dark:text-gray-400 hover:text-gray-700 dark:hover:text-gray-300 w-full"
        >
          <span
            className={`transition-transform duration-150 inline-block ${showAdvanced ? "rotate-90" : ""}`}
          >
            &#9654;
          </span>
          Advanced
        </button>

        {showAdvanced && (
          <div className="mt-2 space-y-3 pl-1">
            {/* ── Proxy Configuration ── */}
            <div className="space-y-2">
              <label className="block text-xs font-semibold text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                Proxy
              </label>
              <div>
                <label className="block text-xs font-medium mb-1">
                  Proxy Type
                </label>
                <select
                  value={proxyType}
                  onChange={(e) => setProxyType(e.target.value)}
                  className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
                >
                  <option value="">None</option>
                  <option value="http">HTTP CONNECT</option>
                  <option value="socks5">SOCKS5</option>
                </select>
              </div>
              {proxyType && (
                <>
                  <div className="grid grid-cols-3 gap-2">
                    <div className="col-span-2">
                      <label className="block text-xs font-medium mb-1">
                        Proxy Host
                      </label>
                      <input
                        type="text"
                        value={proxyHost}
                        onChange={(e) => setProxyHost(e.target.value)}
                        placeholder="proxy.example.com"
                        className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
                      />
                    </div>
                    <div>
                      <label className="block text-xs font-medium mb-1">
                        Port
                      </label>
                      <input
                        type="number"
                        value={proxyPort}
                        onChange={(e) => setProxyPort(e.target.value)}
                        placeholder={proxyType === "http" ? "8080" : "1080"}
                        className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
                      />
                    </div>
                  </div>
                  <div className="grid grid-cols-2 gap-2">
                    <div>
                      <label className="block text-xs font-medium mb-1">
                        Proxy Username
                      </label>
                      <input
                        type="text"
                        value={proxyUsername}
                        onChange={(e) => setProxyUsername(e.target.value)}
                        placeholder="(optional)"
                        className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
                      />
                    </div>
                    <div>
                      <label className="block text-xs font-medium mb-1">
                        Proxy Password
                      </label>
                      <PasswordInput
                        value={proxyPassword}
                        onChange={(e) => setProxyPassword(e.target.value)}
                        placeholder="(optional)"
                        className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
                      />
                    </div>
                  </div>
                </>
              )}
            </div>

            {/* ── Default Paths ── */}
            <div className="space-y-2">
              <label className="block text-xs font-semibold text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                Default Paths
              </label>
              <div>
                <label className="block text-xs font-medium mb-1">
                  Default Local Directory
                </label>
                <input
                  type="text"
                  value={defaultLocalDir}
                  onChange={(e) => setDefaultLocalDir(e.target.value)}
                  placeholder="/home/user/downloads"
                  className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
                />
              </div>
              <div>
                <label className="block text-xs font-medium mb-1">
                  Default Remote Directory
                </label>
                <input
                  type="text"
                  value={defaultRemoteDir}
                  onChange={(e) => setDefaultRemoteDir(e.target.value)}
                  placeholder="/var/www/html"
                  className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
                />
              </div>
            </div>

            {/* ── Upload Permission Defaults (SFTP/FTP only) ── */}
            {(isSshProtocol || isFtpProtocol) && (
              <div className="space-y-2">
                <label className="block text-xs font-semibold text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                  Upload Permission Defaults
                </label>
                <p className="text-xs text-gray-400 dark:text-gray-500">
                  Permissions applied to files and directories after upload (octal, e.g. 644)
                </p>
                <div className="grid grid-cols-2 gap-3">
                  <div>
                    <label className="block text-xs font-medium mb-1">
                      File Mode
                    </label>
                    <input
                      type="text"
                      value={defaultFileMode}
                      onChange={(e) => setDefaultFileMode(e.target.value)}
                      placeholder="644"
                      maxLength={4}
                      className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900 font-mono"
                    />
                  </div>
                  <div>
                    <label className="block text-xs font-medium mb-1">
                      Directory Mode
                    </label>
                    <input
                      type="text"
                      value={defaultDirMode}
                      onChange={(e) => setDefaultDirMode(e.target.value)}
                      placeholder="755"
                      maxLength={4}
                      className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900 font-mono"
                    />
                  </div>
                </div>
              </div>
            )}

            {/* ── Character Set (FTP/FTPS only) ── */}
            {isFtpProtocol && (
              <div className="space-y-2">
                <label className="block text-xs font-semibold text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                  Character Set
                </label>
                <div>
                  <select
                    value={charset}
                    onChange={(e) => setCharset(e.target.value)}
                    className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
                  >
                    {CHARSET_OPTIONS.map((opt) => (
                      <option key={opt.value} value={opt.value}>
                        {opt.label}
                      </option>
                    ))}
                  </select>
                  <p className="text-xs text-gray-400 dark:text-gray-500 mt-1">
                    For legacy FTP servers with non-UTF-8 filename encoding
                  </p>
                </div>
              </div>
            )}

            {/* ── Transfer Data Type (FTP/FTPS only) ── */}
            {isFtpProtocol && (
              <div className="space-y-2">
                <label className="block text-xs font-semibold text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                  Transfer Type
                </label>
                <div>
                  <select
                    value={ftpDataType}
                    onChange={(e) => setFtpDataType(e.target.value)}
                    className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
                  >
                    <option value="binary">Binary (default)</option>
                    <option value="ascii">ASCII</option>
                    <option value="auto">Auto (text extensions use ASCII)</option>
                  </select>
                  <p className="text-xs text-gray-400 dark:text-gray-500 mt-1">
                    Binary is correct for most files. ASCII converts line endings for text files across platforms.
                  </p>
                </div>
              </div>
            )}
          </div>
        )}
      </div>

      <div className="flex justify-end gap-2 pt-2">
        <button
          type="button"
          onClick={onCancel}
          className="text-sm px-3 py-1.5 rounded border border-gray-300 dark:border-gray-600 hover:bg-gray-100 dark:hover:bg-gray-800"
        >
          Cancel
        </button>
        <button
          type="submit"
          className="text-sm px-3 py-1.5 rounded bg-blue-500 text-white hover:bg-blue-600"
        >
          {initialData ? "Update" : "Save"}
        </button>
      </div>
    </form>
  );
}

// ── Connection Item ──

function ConnectionItem({
  connection,
  onConnect,
  onTest,
  onEdit,
  onDelete,
  testResult,
  testing,
  isConnected,
  status,
  onToggleStar,
  onRetry,
  onSignIn,
  signedIn,
  onSignOut,
}: {
  connection: ConnectionProfile;
  onConnect: (id: string) => void;
  onTest: (id: string) => void;
  onEdit: (conn: ConnectionProfile) => void;
  onDelete: (id: string) => void;
  testResult: ConnectionTestResult | null;
  testing: boolean;
  isConnected: boolean;
  status?: ConnectionStatus;
  onToggleStar?: (id: string, starred: boolean) => void;
  onRetry?: (id: string) => void;
  onSignIn?: (conn: ConnectionProfile) => void;
  /** Google Drive / Dropbox / OneDrive: whether a sign-in is stored (null: unknown). */
  signedIn?: boolean | null;
  onSignOut?: (conn: ConnectionProfile) => void;
}) {
  const starred = !!status?.starred;
  const isOAuth = OAUTH_PROTOCOLS.has(connection.protocol);
  const rejected = !!status?.credentials_rejected_at;
  return (
    <div className="flex items-center justify-between px-3 py-2 border-b border-gray-200 dark:border-gray-700 last:border-b-0 hover:bg-gray-50 dark:hover:bg-gray-800/50">
      {onToggleStar && (
        <button
          onClick={() => onToggleStar(connection.id, !starred)}
          className={`mr-1 text-sm leading-none ${starred ? "text-amber-500" : "text-gray-300 dark:text-gray-600 hover:text-amber-400"}`}
          type="button"
          aria-label={starred ? `Unstar ${connection.name}` : `Star ${connection.name}`}
          aria-pressed={starred}
          title={starred ? "Unstar" : "Star (list first)"}
          data-testid={`star-${connection.id}`}
        >
          {starred ? "\u2605" : "\u2606"}
        </button>
      )}
      <button
        onClick={() => onConnect(connection.id)}
        className="flex-1 min-w-0 text-left"
        type="button"
      >
        <div className="flex items-center gap-2">
          {/* Connection status indicator */}
          <span
            className={`w-2 h-2 rounded-full flex-shrink-0 ${
              rejected ? "bg-red-500" : isConnected ? "bg-green-500" : "bg-gray-300 dark:bg-gray-600"
            }`}
            title={rejected ? "Credentials rejected" : isConnected ? "Connected" : "Disconnected"}
          />
          <span className="text-sm font-medium truncate">
            {connection.name}
          </span>
          <span className="text-xs text-gray-400 dark:text-gray-500 uppercase">
            {connection.protocol}
          </span>
          {testResult && (
            <span
              className={`text-xs ${testResult.reachable ? "text-green-500" : "text-red-500"}`}
            >
              {testResult.reachable
                ? `${testResult.latency_ms}ms`
                : "unreachable"}
            </span>
          )}
        </div>
        <div className="text-xs text-gray-500 dark:text-gray-400 truncate">
          {connection.host}
          {connection.port ? `:${connection.port}` : ""}{" "}
          {connection.username ? `(${connection.username})` : ""}
        </div>
        {isOAuth && signedIn === false && (
          <div className="text-xs text-amber-600 dark:text-amber-400 truncate" data-testid={`not-signed-in-${connection.id}`}>
            Not signed in: click Sign in to connect.
          </div>
        )}
        {rejected && (
          <div className="text-xs text-red-600 dark:text-red-400 truncate" data-testid={`rejected-${connection.id}`} title={status?.credentials_rejected_reason ?? undefined}>
            Credentials rejected: automatic reconnects are paused. Edit the connection or click Retry.
          </div>
        )}
      </button>
      <div className="flex items-center gap-1 ml-2">
        {rejected && onRetry && (
          <button
            onClick={() => onRetry(connection.id)}
            className="text-xs px-2 py-1 rounded bg-red-500 text-white hover:bg-red-600"
            type="button"
            title="Retry"
          >
            Retry
          </button>
        )}
        {onSignIn && isOAuth && (
          <button
            onClick={() => onSignIn(connection)}
            className="text-xs px-2 py-1 rounded hover:bg-gray-200 dark:hover:bg-gray-700"
            type="button"
            title="Sign in"
          >
            Sign in
          </button>
        )}
        {onSignOut && isOAuth && signedIn === true && (
          <button
            onClick={() => onSignOut(connection)}
            className="text-xs px-2 py-1 rounded hover:bg-gray-200 dark:hover:bg-gray-700"
            type="button"
            title="Sign out"
            aria-label={`Sign out ${connection.name}`}
          >
            Sign out
          </button>
        )}
        <button
          onClick={() => onTest(connection.id)}
          disabled={testing}
          className="text-xs px-2 py-1 rounded hover:bg-gray-200 dark:hover:bg-gray-700 disabled:opacity-50"
          type="button"
          title="Test connection"
        >
          {testing ? "..." : "Test"}
        </button>
        <button
          onClick={() => onEdit(connection)}
          className="text-xs px-2 py-1 rounded hover:bg-gray-200 dark:hover:bg-gray-700"
          type="button"
          title="Edit"
        >
          Edit
        </button>
        <button
          onClick={() => onDelete(connection.id)}
          className="text-xs px-2 py-1 rounded hover:bg-red-100 dark:hover:bg-red-900/30 text-red-600 dark:text-red-400"
          type="button"
          title="Delete"
        >
          Del
        </button>
      </div>
    </div>
  );
}

// ── Main Connection Panel ──

export function ConnectionPanel() {
  const [connections, setConnections] = useState<ConnectionProfile[]>([]);
  const [groups, setGroups] = useState<ConnectionGroup[]>([]);
  const [protocols, setProtocols] = useState<ProtocolOption[]>(FALLBACK_PROTOCOLS);
  const [showForm, setShowForm] = useState(false);
  const [editingConnection, setEditingConnection] =
    useState<ConnectionProfile | null>(null);
  const [testResults, setTestResults] = useState<
    Record<string, ConnectionTestResult>
  >({});
  const [testingIds, setTestingIds] = useState<Set<string>>(new Set());

  // Connection status tracking
  const [connectedProtocols, setConnectedProtocols] = useState<Set<string>>(new Set());
  const statusPollRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // Remote browser state
  const [connectedId, setConnectedId] = useState<string | null>(null);
  const [connectedProtocol, setConnectedProtocol] = useState<string | null>(null);
  const [remotePath, setRemotePath] = useState("/");
  const [remoteFiles, setRemoteFiles] = useState<RemoteFileEntry[]>([]);
  const [browsing, setBrowsing] = useState(false);
  const [browseError, setBrowseError] = useState<string | null>(null);
  const [transferStatus, setTransferStatus] = useState<string | null>(null);

  // Starred / credentials-rejected flags, by connection id
  const [statuses, setStatuses] = useState<Record<string, ConnectionStatus>>({});
  // Google Drive / Dropbox / OneDrive connection being signed in
  const [signInFor, setSignInFor] = useState<ConnectionProfile | null>(null);
  // Whether each cloud connection has a stored sign-in, by connection id
  const [signedIn, setSignedIn] = useState<Record<string, boolean>>({});
  const [signOutError, setSignOutError] = useState<string | null>(null);
  // Storage of the connected account
  const [quota, setQuota] = useState<RemoteQuota | null>(null);
  // "Calculate size" results, by remote path
  const [sizes, setSizes] = useState<Record<string, SizeState>>({});
  const [entryMenu, setEntryMenu] = useState<{ entry: RemoteFileEntry; x: number; y: number } | null>(null);
  const setActiveGuidedFlow = useUIStore((st) => st.setActiveGuidedFlow);
  const [showCloudMenu, setShowCloudMenu] = useState(false);

  const refreshStatus = useCallback(async () => {
    const list = await tauriInvoke<ConnectionStatus[]>("list_connection_status", undefined, []).catch(() => []);
    setStatuses(Object.fromEntries((list ?? []).map((st) => [st.id, st])));
  }, []);

  const refresh = useCallback(async () => {
    try {
      const [conns, grps] = await Promise.all([
        tauriInvoke<ConnectionProfile[]>("list_connections", undefined, []),
        tauriInvoke<ConnectionGroup[]>("list_connection_groups", undefined, []),
      ]);
      setConnections(conns);
      setGroups(grps);
    } catch (err) {
      console.error("Failed to load connections:", err);
    }
    void refreshStatus();
  }, [refreshStatus]);

  // Which cloud connections have a stored sign-in.
  useEffect(() => {
    let live = true;
    const cloud = connections.filter((c) => OAUTH_PROTOCOLS.has(c.protocol));
    void Promise.all(cloud.map(async (c) => [c.id, await isSignedIn(c.id).catch(() => false)] as const)).then((pairs) => {
      if (live) setSignedIn(Object.fromEntries(pairs));
    });
    return () => {
      live = false;
    };
  }, [connections]);

  // A guided cloud wizard saved a connection elsewhere: reload.
  useEffect(() => {
    const onChanged = () => void refresh();
    window.addEventListener(CONNECTIONS_CHANGED_EVENT, onChanged);
    return () => window.removeEventListener(CONNECTIONS_CHANGED_EVENT, onChanged);
  }, [refresh]);

  // (2) connector_list_protocols - fetch available protocols on mount
  const fetchProtocols = useCallback(async () => {
    try {
      const protocolList = await tauriInvoke<string[]>(
        "connector_list_protocols",
        undefined,
        []
      );
      if (protocolList.length > 0) {
        // Merge fetched protocols with fallback data
        const merged: ProtocolOption[] = protocolList.map((p) => {
          const existing = FALLBACK_PROTOCOLS.find((fp) => fp.value === p);
          return {
            value: p,
            label: existing?.label || p.toUpperCase(),
            port: existing?.port || DEFAULT_PORTS[p] || 22,
          };
        });
        setProtocols(merged);
      }
    } catch (err) {
      console.error("Failed to fetch protocols, using fallback list:", err);
      // Keep FALLBACK_PROTOCOLS
    }
  }, []);

  // (1) connector_is_connected - poll connection status
  const pollConnectionStatus = useCallback(async () => {
    if (!connectedProtocol) return;
    try {
      const isConnected = await tauriInvoke<boolean>(
        "connector_is_connected",
        { protocol: connectedProtocol },
        false
      );
      setConnectedProtocols((prev) => {
        const next = new Set(prev);
        if (isConnected) {
          next.add(connectedProtocol);
        } else {
          next.delete(connectedProtocol);
        }
        return next;
      });
    } catch (err) {
      console.error("Failed to check connection status:", err);
    }
  }, [connectedProtocol]);

  useEffect(() => {
    refresh();
    fetchProtocols();
  }, [refresh, fetchProtocols]);

  // Poll connection status periodically when connected
  useEffect(() => {
    if (connectedProtocol) {
      pollConnectionStatus();
      statusPollRef.current = setInterval(pollConnectionStatus, 5000);
    }
    return () => {
      if (statusPollRef.current) {
        clearInterval(statusPollRef.current);
        statusPollRef.current = null;
      }
    };
  }, [connectedProtocol, pollConnectionStatus]);

  const handleSave = useCallback(
    async (data: Record<string, unknown>) => {
      try {
        const saved = await tauriInvoke<ConnectionProfile | undefined>("save_connection", data as Record<string, unknown>);
        setShowForm(false);
        setEditingConnection(null);
        refresh();
        // A new Google Drive / Dropbox / OneDrive connection signs in next.
        if (saved && !data.connectionId && OAUTH_PROTOCOLS.has(saved.protocol)) setSignInFor(saved);
      } catch (err) {
        console.error("Failed to save connection:", err);
      }
    },
    [refresh]
  );

  const handleToggleStar = useCallback(
    async (id: string, starred: boolean) => {
      try {
        await tauriInvoke("set_connection_starred", { connectionId: id, starred });
      } catch (err) {
        console.error("Failed to star connection:", err);
      }
      refresh();
    },
    [refresh]
  );

  const handleDelete = useCallback(
    async (id: string) => {
      try {
        await tauriInvoke("delete_connection", { connectionId: id });
        refresh();
      } catch (err) {
        console.error("Failed to delete connection:", err);
      }
    },
    [refresh]
  );

  // (3) delete_connection_group
  const handleDeleteGroup = useCallback(
    async (groupId: string) => {
      try {
        await tauriInvoke("delete_connection_group", { groupId });
        refresh();
      } catch (err) {
        console.error("Failed to delete connection group:", err);
      }
    },
    [refresh]
  );

  const handleTest = useCallback(async (id: string) => {
    setTestingIds((prev) => new Set([...prev, id]));
    try {
      const result = await tauriInvoke<ConnectionTestResult>("test_connection", {
        connectionId: id,
      });
      setTestResults((prev) => ({ ...prev, [id]: result }));
    } catch (err) {
      console.error("Failed to test connection:", err);
    } finally {
      setTestingIds((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      });
    }
  }, []);

  const handleConnect = useCallback(
    async (id: string) => {
      setBrowseError(null);
      setBrowsing(true);
      try {
        // 1. Get connection profile
        const conn = await tauriInvoke<ConnectionProfile>("get_connection", {
          connectionId: id,
        });

        // 2. Connect to the remote server
        await tauriInvoke("connector_connect", {
          protocol: conn.protocol,
          host: conn.host,
          port: conn.port,
          username: conn.username,
          remotePath: conn.remote_path,
          connectionId: id,
        });

        // 3. List remote directory contents
        const initialPath = conn.remote_path || "/";
        const files = await tauriInvoke<RemoteFileEntry[]>(
          "connector_list_remote",
          { protocol: conn.protocol, path: initialPath, connectionId: id },
          []
        );

        // 4. Update state
        setConnectedId(id);
        setConnectedProtocol(conn.protocol);
        setRemotePath(initialPath);
        setRemoteFiles(files);
        setSizes({});
        // Mark as connected immediately
        setConnectedProtocols((prev) => new Set([...prev, conn.protocol]));
        // 5. Storage quota, where the provider reports one
        setQuota(null);
        tauriInvoke<RemoteQuota | null>("connector_quota", { protocol: conn.protocol, connectionId: id }, null)
          .then((q) => setQuota(q ?? null))
          .catch(() => setQuota(null));
      } catch (err) {
        console.error("Failed to connect:", err);
        setBrowseError(
          errorText(err)
        );
      } finally {
        setBrowsing(false);
        // A rejected sign-in flags the connection (and a success clears it).
        void refreshStatus();
      }
    },
    [refreshStatus]
  );

  // Retry: clear the credentials-rejected flag, then connect.
  const handleRetry = useCallback(
    async (id: string) => {
      try {
        await tauriInvoke("clear_connection_credentials_rejected", { connectionId: id });
      } catch (err) {
        console.error("Failed to clear the flag:", err);
      }
      await handleConnect(id);
    },
    [handleConnect]
  );

  const sizeJob = useRef(0);
  const handleCalculateSize = useCallback(
    async (entry: RemoteFileEntry) => {
      if (!connectedProtocol) return;
      const jobId = `size-${Date.now()}-${++sizeJob.current}`;
      setSizes((prev) => ({ ...prev, [entry.path]: { jobId, running: true } }));
      try {
        const result = await tauriInvoke<FolderSize>("connector_folder_size", { protocol: connectedProtocol, path: entry.path, jobId, connectionId: connectedId });
        setSizes((prev) => ({ ...prev, [entry.path]: { running: false, result } }));
      } catch (err) {
        setSizes((prev) => ({ ...prev, [entry.path]: { running: false, error: errorText(err) } }));
      }
    },
    [connectedProtocol, connectedId]
  );

  const handleCancelSize = useCallback(async (path: string) => {
    const st = sizes[path];
    if (st?.running) await tauriInvoke("connector_folder_size_cancel", { jobId: st.jobId }, false).catch(() => false);
  }, [sizes]);

  // Close the entry context menu on any outside click / Escape
  useEffect(() => {
    if (!entryMenu) return;
    const close = () => setEntryMenu(null);
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && close();
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", onKey);
    };
  }, [entryMenu]);

  const handleDisconnect = useCallback(async () => {
    if (connectedProtocol) {
      try {
        await tauriInvoke("connector_disconnect", {
          protocol: connectedProtocol,
        });
      } catch (err) {
        console.error("Disconnect failed:", err);
      }
      setConnectedProtocols((prev) => {
        const next = new Set(prev);
        next.delete(connectedProtocol);
        return next;
      });
    }
    setConnectedId(null);
    setConnectedProtocol(null);
    setRemoteFiles([]);
    setRemotePath("/");
    setBrowseError(null);
    setQuota(null);
    setSizes({});
  }, [connectedProtocol]);

  // Forget a cloud connection's sign-in; stop browsing it if it is open.
  const handleSignOut = useCallback(
    async (conn: ConnectionProfile) => {
      setSignOutError(null);
      try {
        await signOut(conn.id);
      } catch (err) {
        setSignOutError(`${conn.name}: ${errorText(err)}`);
        return;
      }
      // The row now reads "Not signed in".
      setSignedIn((prev) => ({ ...prev, [conn.id]: false }));
      if (connectedId === conn.id) await handleDisconnect();
      void refreshStatus();
    },
    [connectedId, handleDisconnect, refreshStatus]
  );

  const handleBrowse = useCallback(
    async (path: string) => {
      if (!connectedProtocol) return;
      setBrowsing(true);
      setBrowseError(null);
      try {
        const files = await tauriInvoke<RemoteFileEntry[]>(
          "connector_list_remote",
          { protocol: connectedProtocol, path, connectionId: connectedId },
          []
        );
        setRemoteFiles(files);
        setRemotePath(path);
      } catch (err) {
        setBrowseError(
          errorText(err)
        );
      } finally {
        setBrowsing(false);
      }
    },
    [connectedProtocol, connectedId]
  );

  // Derive the connected connection name for the header
  const connectedName = useMemo(() => {
    if (!connectedId) return null;
    const conn = connections.find((c) => c.id === connectedId);
    return conn?.name || connectedId;
  }, [connectedId, connections]);

  const [sharingEntry, setSharing] = useState<RemoteFileEntry | null>(null);

  const joinRemote = (dir: string, name: string) => (dir.endsWith("/") ? `${dir}${name}` : `${dir}/${name}`);

  const reloadRemote = useCallback(async () => {
    if (!connectedProtocol) return;
    const files = await tauriInvoke<RemoteFileEntry[]>(
      "connector_list_remote",
      { protocol: connectedProtocol, path: remotePath, connectionId: connectedId },
      [],
    );
    setRemoteFiles(files);
  }, [connectedProtocol, connectedId, remotePath]);

  const handleUpload = useCallback(async () => {
    if (!connectedProtocol) return;
    const files = await pickFiles("Select files to upload");
    if (files.length === 0) return;
    setBrowseError(null);
    let done = 0;
    try {
      for (const local of files) {
        const name = local.split(/[\\/]/).pop() || local;
        setTransferStatus(`Uploading ${name} (${done + 1}/${files.length})...`);
        await tauriInvoke("connector_upload", {
          protocol: connectedProtocol,
          localPath: local,
          remotePath: joinRemote(remotePath, name),
          connectionId: connectedId,
        });
        done++;
      }
      setTransferStatus(`Uploaded ${done} file${done === 1 ? "" : "s"}.`);
      await reloadRemote();
    } catch (err) {
      setTransferStatus(null);
      setBrowseError(`Upload failed after ${done}/${files.length}: ${errorText(err)}`);
    }
  }, [connectedProtocol, connectedId, remotePath, reloadRemote]);

  const handleDownload = useCallback(async (entry: RemoteFileEntry) => {
    if (!connectedProtocol) return;
    const dir = await pickFolder(`Choose where to save ${entry.name}`);
    if (!dir) return;
    setBrowseError(null);
    // A native Google file downloads as an export: give it the format's extension.
    const fileName = connectedProtocol === "google_drive"
      ? googleDownloadName(entry.name, entry.extension, connections.find((c) => c.id === connectedId)?.extras?.google_drive?.export_formats)
      : entry.name;
    setTransferStatus(`Downloading ${entry.name}...`);
    try {
      const bytes = await tauriInvoke<number>("connector_download", {
        protocol: connectedProtocol,
        remotePath: entry.path,
        localPath: `${dir.replace(/\/$/, "")}/${fileName}`,
        connectionId: connectedId,
      });
      setTransferStatus(`Downloaded ${fileName} (${formatFileSize(bytes)}) to ${dir}.`);
    } catch (err) {
      setTransferStatus(null);
      setBrowseError(`Download failed: ${errorText(err)}`);
    }
  }, [connectedProtocol, connectedId, connections]);

  const handleRemoteMkdir = useCallback(async () => {
    if (!connectedProtocol) return;
    const name = (await promptText("New remote folder name:", "New Folder"))?.trim();
    if (!name) return;
    setBrowseError(null);
    try {
      await tauriInvoke("connector_make_directory", {
        protocol: connectedProtocol,
        remotePath: joinRemote(remotePath, name),
        connectionId: connectedId,
      });
      setTransferStatus(`Created folder ${name}.`);
      await reloadRemote();
    } catch (err) {
      setBrowseError(`Create folder failed: ${errorText(err)}`);
    }
  }, [connectedProtocol, connectedId, remotePath, reloadRemote]);

  const handleRemoteDelete = useCallback(async (entry: RemoteFileEntry) => {
    if (!connectedProtocol) return;
    const ok = await confirmText(
      `Delete ${entry.is_dir ? "folder" : "file"} "${entry.name}" from the server? This cannot be undone.`,
      "Delete",
      true,
    );
    if (!ok) return;
    setBrowseError(null);
    try {
      await tauriInvoke("connector_remove", {
        protocol: connectedProtocol,
        remotePath: entry.path,
        isDir: entry.is_dir,
        connectionId: connectedId,
      });
      setTransferStatus(`Deleted ${entry.name}.`);
      await reloadRemote();
    } catch (err) {
      setBrowseError(`Delete failed: ${errorText(err)}`);
    }
  }, [connectedProtocol, connectedId, reloadRemote]);

  const handleExport = useCallback(async () => {
    try {
      const json = await tauriInvoke<string>("export_connections", undefined, "[]");
      const blob = new Blob([json], { type: "application/json" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = "connections.json";
      a.click();
      URL.revokeObjectURL(url);
    } catch (err) {
      console.error("Failed to export:", err);
    }
  }, []);

  // ── Import: UFOP JSON + third-party ──
  const [showImportMenu, setShowImportMenu] = useState(false);
  const [importError, setImportError] = useState<string | null>(null);
  const importFileRef = useRef<HTMLInputElement>(null);
  const importMenuRef = useRef<HTMLDivElement>(null);
  const [pendingImportSource, setPendingImportSource] = useState<
    "ufop" | ThirdPartySource | null
  >(null);

  // Close import menu on outside click
  useEffect(() => {
    if (!showImportMenu) return;
    const handleClick = (e: MouseEvent) => {
      if (importMenuRef.current && !importMenuRef.current.contains(e.target as Node)) {
        setShowImportMenu(false);
      }
    };
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, [showImportMenu]);

  const handleImportSelect = useCallback((source: "ufop" | ThirdPartySource) => {
    setShowImportMenu(false);
    setImportError(null);
    setPendingImportSource(source);
    // Trigger native file input
    if (importFileRef.current) {
      importFileRef.current.value = "";
      importFileRef.current.click();
    }
  }, []);

  const handleImportFile = useCallback(
    async (e: React.ChangeEvent<HTMLInputElement>) => {
      const file = e.target.files?.[0];
      if (!file || !pendingImportSource) return;

      if (pendingImportSource === "ufop") {
        // Existing UFOP JSON import
        const reader = new FileReader();
        reader.onload = async () => {
          try {
            await tauriInvoke("import_connections", {
              json: reader.result as string,
            });
            refresh();
            setImportError(null);
          } catch (err) {
            console.error("UFOP import failed:", err);
            setImportError(errorText(err));
          }
        };
        reader.readAsText(file);
      } else {
        // Third-party import: read file, send to backend with source_app
        const reader = new FileReader();
        reader.onload = async () => {
          try {
            await tauriInvoke("import_from_third_party", {
              fileContent: reader.result as string,
              fileName: file.name,
              sourceApp: pendingImportSource,
            });
            refresh();
            setImportError(null);
          } catch (err) {
            console.error(`${pendingImportSource} import failed:`, err);
            setImportError(errorText(err));
          }
        };
        reader.readAsText(file);
      }
      setPendingImportSource(null);
    },
    [pendingImportSource, refresh]
  );

  // Check if a specific connection's protocol is connected
  const isProtocolConnected = useCallback(
    (protocol: string) => connectedProtocols.has(protocol),
    [connectedProtocols]
  );

  // Starred connections first, then by name (within each group too)
  const sorted = useMemo(
    () =>
      connections
        .slice()
        .sort((a, b) => Number(!!statuses[b.id]?.starred) - Number(!!statuses[a.id]?.starred) || a.name.localeCompare(b.name)),
    [connections, statuses]
  );

  // Group connections by group_id
  const ungrouped = sorted.filter((c) => !c.group_id);
  const grouped = groups.map((g) => ({
    group: g,
    connections: sorted.filter((c) => c.group_id === g.id),
  }));

  const itemProps = (conn: ConnectionProfile) => ({
    status: statuses[conn.id],
    onToggleStar: (id: string, starred: boolean) => void handleToggleStar(id, starred),
    onRetry: (id: string) => void handleRetry(id),
    onSignIn: (c: ConnectionProfile) => setSignInFor(c),
    signedIn: signedIn[conn.id] ?? null,
    onSignOut: (c: ConnectionProfile) => void handleSignOut(c),
  });

  return (
    <div
      className="flex flex-col h-full"
      role="region"
      aria-label="Connection manager"
    >
      {/* Header */}
      <div className="flex items-center justify-between px-3 py-2 border-b border-gray-200 dark:border-gray-700">
        <h2 className="text-sm font-semibold">Connections</h2>
        <div className="flex items-center gap-1">
          <button
            onClick={() => {
              setEditingConnection(null);
              setShowForm(true);
            }}
            className="text-xs px-2 py-1 rounded bg-blue-500 text-white hover:bg-blue-600"
            type="button"
          >
            + New
          </button>
          <div className="relative">
            <button
              onClick={() => setShowCloudMenu((v) => !v)}
              className="text-xs px-2 py-1 rounded hover:bg-gray-200 dark:hover:bg-gray-700"
              type="button"
              aria-haspopup="menu"
              aria-expanded={showCloudMenu}
            >
              + Cloud
            </button>
            {showCloudMenu && (
              <div className="absolute right-0 top-full mt-1 w-48 bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-md shadow-lg z-50" role="menu">
                {([
                  ["connect-gdrive", "Google Drive"],
                  ["connect-dropbox", "Dropbox"],
                  ["connect-onedrive", "OneDrive / SharePoint"],
                  ["connect-gcs", "Google Cloud Storage"],
                ] as const).map(([flow, label]) => (
                  <button
                    key={flow}
                    role="menuitem"
                    onClick={() => {
                      setShowCloudMenu(false);
                      setActiveGuidedFlow(flow);
                    }}
                    className="w-full text-left px-3 py-1.5 text-xs hover:bg-gray-100 dark:hover:bg-gray-700"
                    type="button"
                  >
                    {label}
                  </button>
                ))}
              </div>
            )}
          </div>
          <button
            onClick={handleExport}
            className="text-xs px-2 py-1 rounded hover:bg-gray-200 dark:hover:bg-gray-700"
            type="button"
          >
            Export
          </button>
          {/* Import dropdown */}
          <div className="relative" ref={importMenuRef}>
            <button
              onClick={() => setShowImportMenu((v) => !v)}
              className="text-xs px-2 py-1 rounded hover:bg-gray-200 dark:hover:bg-gray-700"
              type="button"
            >
              Import
            </button>
            {showImportMenu && (
              <div className="absolute right-0 top-full mt-1 w-40 bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-md shadow-lg z-50">
                <button
                  onClick={() => handleImportSelect("ufop")}
                  className="w-full text-left px-3 py-1.5 text-xs hover:bg-gray-100 dark:hover:bg-gray-700 rounded-t-md"
                  type="button"
                >
                  UFOP JSON
                </button>
                <button
                  onClick={() => handleImportSelect("filezilla")}
                  className="w-full text-left px-3 py-1.5 text-xs hover:bg-gray-100 dark:hover:bg-gray-700"
                  type="button"
                >
                  FileZilla
                </button>
                <button
                  onClick={() => handleImportSelect("cyberduck")}
                  className="w-full text-left px-3 py-1.5 text-xs hover:bg-gray-100 dark:hover:bg-gray-700"
                  type="button"
                >
                  Cyberduck
                </button>
                <button
                  onClick={() => handleImportSelect("winscp")}
                  className="w-full text-left px-3 py-1.5 text-xs hover:bg-gray-100 dark:hover:bg-gray-700"
                  type="button"
                >
                  WinSCP
                </button>
                <button
                  onClick={() => handleImportSelect("transmit")}
                  className="w-full text-left px-3 py-1.5 text-xs hover:bg-gray-100 dark:hover:bg-gray-700 rounded-b-md"
                  type="button"
                >
                  Transmit
                </button>
              </div>
            )}
          </div>
          {/* Hidden file input for imports */}
          <input
            ref={importFileRef}
            type="file"
            accept=".json,.xml,.ini,.plist"
            onChange={handleImportFile}
            className="hidden"
            aria-hidden="true"
          />
        </div>
      </div>
      {/* Import error banner */}
      {importError && (
        <div className="px-3 py-1.5 text-xs text-red-600 dark:text-red-400 bg-red-50 dark:bg-red-900/20 border-b border-gray-200 dark:border-gray-700 flex items-center justify-between">
          <span>Import failed: {importError}</span>
          <button
            onClick={() => setImportError(null)}
            className="text-xs ml-2 hover:text-red-800 dark:hover:text-red-300"
            type="button"
          >
            Dismiss
          </button>
        </div>
      )}

      {signOutError && (
        <div className="px-3 py-1.5 text-xs text-red-600 dark:text-red-400 bg-red-50 dark:bg-red-900/20 border-b border-gray-200 dark:border-gray-700 flex items-center justify-between" role="alert">
          <span>Sign out failed: {signOutError}</span>
          <button onClick={() => setSignOutError(null)} className="text-xs ml-2 hover:text-red-800 dark:hover:text-red-300" type="button">
            Dismiss
          </button>
        </div>
      )}

      {/* Quick connect */}
      <div className="px-3 py-2 border-b border-gray-200 dark:border-gray-700">
        <QuickConnectBar onConnect={handleConnect} />
      </div>

      {/* Form */}
      {showForm && (
        <div className="border-b border-gray-200 dark:border-gray-700">
          <ConnectionForm
            initialData={editingConnection}
            onSave={handleSave}
            onCancel={() => {
              setShowForm(false);
              setEditingConnection(null);
            }}
            groups={groups}
            protocols={protocols}
          />
        </div>
      )}

      {/* Connection list */}
      <div className="flex-1 overflow-y-auto">
        {/* Grouped connections */}
        {grouped.map(
          ({ group, connections: conns }) =>
            conns.length > 0 && (
              <div key={group.id}>
                <div className="flex items-center justify-between px-3 py-1.5 bg-gray-50 dark:bg-gray-800/50">
                  <span className="text-xs font-semibold text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                    {group.name}
                  </span>
                  <button
                    onClick={() => handleDeleteGroup(group.id)}
                    className="text-xs px-1.5 py-0.5 rounded hover:bg-red-100 dark:hover:bg-red-900/30 text-red-500 dark:text-red-400"
                    type="button"
                    title={`Delete group "${group.name}"`}
                    aria-label={`Delete connection group ${group.name}`}
                  >
                    X
                  </button>
                </div>
                {conns.map((conn) => (
                  <ConnectionItem
                    key={conn.id}
                    connection={conn}
                    onConnect={handleConnect}
                    onTest={handleTest}
                    onEdit={(c) => {
                      setEditingConnection(c);
                      setShowForm(true);
                    }}
                    onDelete={handleDelete}
                    testResult={testResults[conn.id] || null}
                    testing={testingIds.has(conn.id)}
                    isConnected={isProtocolConnected(conn.protocol)}
                    {...itemProps(conn)}
                  />
                ))}
              </div>
            )
        )}

        {/* Ungrouped connections */}
        {ungrouped.length > 0 && grouped.some((g) => g.connections.length > 0) && (
          <div className="px-3 py-1.5 text-xs font-semibold text-gray-500 dark:text-gray-400 bg-gray-50 dark:bg-gray-800/50 uppercase tracking-wider">
            Ungrouped
          </div>
        )}
        {ungrouped.map((conn) => (
          <ConnectionItem
            key={conn.id}
            connection={conn}
            onConnect={handleConnect}
            onTest={handleTest}
            onEdit={(c) => {
              setEditingConnection(c);
              setShowForm(true);
            }}
            onDelete={handleDelete}
            testResult={testResults[conn.id] || null}
            testing={testingIds.has(conn.id)}
            isConnected={isProtocolConnected(conn.protocol)}
            {...itemProps(conn)}
          />
        ))}

        {connections.length === 0 && !showForm && (
          <div className="flex flex-col items-center justify-center h-32 text-sm text-gray-400 dark:text-gray-500">
            <p>No saved connections</p>
            <button
              onClick={() => setShowForm(true)}
              className="mt-2 text-blue-500 hover:text-blue-600"
              type="button"
            >
              Add your first connection
            </button>
          </div>
        )}
      </div>

      {/* Remote Browser Panel */}
      {connectedId && (
        <div className="flex flex-col border-t border-gray-200 dark:border-gray-700 max-h-[50%]">
          {/* Browser header */}
          <div className="flex items-center justify-between px-3 py-2 bg-green-50 dark:bg-green-900/20 border-b border-gray-200 dark:border-gray-700">
            <div className="flex items-center gap-2 min-w-0">
              <span
                className={`w-2 h-2 rounded-full flex-shrink-0 ${
                  connectedProtocol && connectedProtocols.has(connectedProtocol)
                    ? "bg-green-500"
                    : "bg-yellow-500"
                }`}
                title={
                  connectedProtocol && connectedProtocols.has(connectedProtocol)
                    ? "Connected"
                    : "Connection status unknown"
                }
              />
              <span className="text-sm font-medium truncate">
                {connectedName}
              </span>
              {connectedProtocol && (
                <span className="text-xs text-gray-400 dark:text-gray-500 uppercase flex-shrink-0">
                  {connectedProtocol}
                </span>
              )}
            </div>
            <button
              onClick={handleDisconnect}
              className="text-xs px-2 py-1 rounded bg-red-500 text-white hover:bg-red-600 flex-shrink-0"
              type="button"
            >
              Disconnect
            </button>
          </div>
          {quota && formatQuota(quota) && (
            <div className="px-3 py-1 text-xs text-gray-500 dark:text-gray-400 border-b border-gray-200 dark:border-gray-700" data-testid="remote-quota">
              Storage: {formatQuota(quota)}
            </div>
          )}

          {/* Breadcrumb path */}
          <div className="flex items-center gap-1 px-3 py-1.5 bg-gray-50 dark:bg-gray-800/50 border-b border-gray-200 dark:border-gray-700 overflow-x-auto">
            {remotePath !== "/" && (
              <button
                onClick={() => {
                  const parent = remotePath.replace(/\/[^/]+\/?$/, "") || "/";
                  handleBrowse(parent);
                }}
                className="text-xs px-1.5 py-0.5 rounded hover:bg-gray-200 dark:hover:bg-gray-700 flex-shrink-0"
                type="button"
                title="Go to parent directory"
              >
                ..
              </button>
            )}
            {remotePath.split("/").filter(Boolean).length === 0 ? (
              <span className="text-xs text-gray-500 dark:text-gray-400 font-mono">
                /
              </span>
            ) : (
              remotePath
                .split("/")
                .filter(Boolean)
                .map((segment, idx, arr) => {
                  const segPath = "/" + arr.slice(0, idx + 1).join("/");
                  return (
                    <span key={segPath} className="flex items-center gap-1">
                      <span className="text-xs text-gray-400 dark:text-gray-500">
                        /
                      </span>
                      <button
                        onClick={() => handleBrowse(segPath)}
                        className="text-xs text-blue-500 hover:text-blue-600 hover:underline font-mono"
                        type="button"
                      >
                        {segment}
                      </button>
                    </span>
                  );
                })
            )}
            {browsing && (
              <span className="text-xs text-gray-400 dark:text-gray-500 ml-auto flex-shrink-0 animate-pulse">
                Loading...
              </span>
            )}
            <div className="ml-auto flex items-center gap-1 flex-shrink-0">
              <button
                onClick={handleUpload}
                className="text-xs px-2 py-0.5 rounded bg-blue-500 text-white hover:bg-blue-600"
                type="button"
                data-testid="remote-upload"
              >
                Upload
              </button>
              <button
                onClick={handleRemoteMkdir}
                className="text-xs px-2 py-0.5 rounded border border-gray-300 dark:border-gray-600 hover:bg-gray-100 dark:hover:bg-gray-700"
                type="button"
                data-testid="remote-new-folder"
              >
                New Folder
              </button>
            </div>
          </div>

          {transferStatus && !browseError && (
            <div className="px-3 py-1.5 text-xs text-green-700 dark:text-green-400 bg-green-50 dark:bg-green-900/20 border-b border-gray-200 dark:border-gray-700" data-testid="remote-status">
              {transferStatus}
            </div>
          )}

          {/* Error display */}
          {browseError && (
            <div className="px-3 py-2 text-xs text-red-600 dark:text-red-400 bg-red-50 dark:bg-red-900/20 border-b border-gray-200 dark:border-gray-700">
              {browseError}
            </div>
          )}

          {/* File/folder list */}
          <div className="flex-1 overflow-y-auto">
            {remoteFiles.length === 0 && !browsing && !browseError && (
              <div className="flex items-center justify-center h-16 text-xs text-gray-400 dark:text-gray-500">
                Empty directory
              </div>
            )}
            {remoteFiles
              .slice()
              .sort((a, b) => {
                // Directories first, then alphabetical
                if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
                return a.name.localeCompare(b.name);
              })
              .map((entry) => (
                <div
                  key={entry.path}
                  className="flex items-center border-b border-gray-100 dark:border-gray-800 last:border-b-0 group"
                  onContextMenu={(e) => {
                    e.preventDefault();
                    setEntryMenu({ entry, x: e.clientX, y: e.clientY });
                  }}
                >
                <button
                  onClick={() => {
                    if (entry.is_dir) {
                      handleBrowse(entry.path);
                    }
                  }}
                  className={`flex-1 min-w-0 text-left flex items-center justify-between px-3 py-1.5 text-sm ${
                    entry.is_dir
                      ? "hover:bg-blue-50 dark:hover:bg-blue-900/20 cursor-pointer"
                      : "hover:bg-gray-50 dark:hover:bg-gray-800/30 cursor-default"
                  }`}
                  type="button"
                  disabled={!entry.is_dir}
                >
                  <div className="flex items-center gap-2 min-w-0">
                    <span className="text-xs flex-shrink-0">
                      {entry.is_dir ? "\uD83D\uDCC1" : "\uD83D\uDCC4"}
                    </span>
                    <span
                      className={`truncate ${
                        entry.is_dir
                          ? "font-medium text-blue-600 dark:text-blue-400"
                          : "text-gray-700 dark:text-gray-300"
                      }`}
                    >
                      {entry.name}
                    </span>
                  </div>
                  <div className="flex items-center gap-2 flex-shrink-0 ml-2">
                    {!entry.is_dir && (
                      <span className="text-xs text-gray-400 dark:text-gray-500">
                        {formatFileSize(entry.size)}
                      </span>
                    )}
                    {entry.is_dir && sizes[entry.path] && (() => {
                      const st = sizes[entry.path];
                      if (st.running) return <span className="text-xs text-gray-400 dark:text-gray-500 animate-pulse" data-testid={`size-${entry.name}`}>Calculating…</span>;
                      if (st.error) return <span className="text-xs text-red-500" data-testid={`size-${entry.name}`} title={st.error}>size failed</span>;
                      const r = st.result!;
                      return (
                        <span className="text-xs text-gray-500 dark:text-gray-400" data-testid={`size-${entry.name}`}>
                          {formatFileSize(r.bytes)} in {r.files} file{r.files === 1 ? "" : "s"}
                          {r.complete ? "" : " (stopped)"}
                        </span>
                      );
                    })()}
                    {entry.modified && (
                      <span className="text-xs text-gray-400 dark:text-gray-500">
                        {formatModified(entry.modified)}
                      </span>
                    )}
                  </div>
                </button>
                <div className="flex items-center gap-1 pr-2 flex-shrink-0 opacity-0 group-hover:opacity-100 focus-within:opacity-100">
                  {entry.is_dir && sizes[entry.path]?.running && (
                    <button
                      onClick={() => void handleCancelSize(entry.path)}
                      className="text-xs px-1.5 py-0.5 rounded hover:bg-gray-200 dark:hover:bg-gray-700"
                      type="button"
                      aria-label={`Stop calculating the size of ${entry.name}`}
                    >
                      Stop
                    </button>
                  )}
                  {!entry.is_dir && connectedProtocol && SHAREABLE_PROTOCOLS.has(connectedProtocol) && (
                    <button
                      onClick={() => setSharing(entry)}
                      className="text-xs px-1.5 py-0.5 rounded hover:bg-gray-200 dark:hover:bg-gray-700"
                      type="button"
                      aria-label={`Share ${entry.name}`}
                      data-testid={`remote-share-${entry.name}`}
                    >
                      Share
                    </button>
                  )}
                  {!entry.is_dir && (
                    <button
                      onClick={() => handleDownload(entry)}
                      className="text-xs px-1.5 py-0.5 rounded hover:bg-gray-200 dark:hover:bg-gray-700"
                      type="button"
                      aria-label={`Download ${entry.name}`}
                      data-testid={`remote-download-${entry.name}`}
                    >
                      Download
                    </button>
                  )}
                  <button
                    onClick={() => handleRemoteDelete(entry)}
                    className="text-xs px-1.5 py-0.5 rounded text-red-600 hover:bg-red-50 dark:hover:bg-red-900/20"
                    type="button"
                    aria-label={`Delete ${entry.name}`}
                    data-testid={`remote-delete-${entry.name}`}
                  >
                    Delete
                  </button>
                </div>
                </div>
              ))}
          </div>
        </div>
      )}
      {sharingEntry && connectedProtocol && (
        <ShareLinkDialog protocol={connectedProtocol} connectionId={connectedId} path={sharingEntry.path} name={sharingEntry.name} onClose={() => setSharing(null)} />
      )}
      {entryMenu && (
        <div
          role="menu"
          className="fixed z-50 min-w-40 bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-md shadow-lg py-1"
          style={{ left: entryMenu.x, top: entryMenu.y }}
          onMouseDown={(e) => e.stopPropagation()}
        >
          {entryMenu.entry.is_dir ? (
            <button
              role="menuitem"
              type="button"
              className="w-full text-left px-3 py-1.5 text-xs hover:bg-gray-100 dark:hover:bg-gray-700"
              onClick={() => {
                const entry = entryMenu.entry;
                setEntryMenu(null);
                void handleCalculateSize(entry);
              }}
            >
              Calculate size
            </button>
          ) : (
            <button
              role="menuitem"
              type="button"
              className="w-full text-left px-3 py-1.5 text-xs hover:bg-gray-100 dark:hover:bg-gray-700"
              onClick={() => {
                const entry = entryMenu.entry;
                setEntryMenu(null);
                void handleDownload(entry);
              }}
            >
              Download…
            </button>
          )}
        </div>
      )}
      {signInFor && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40" role="dialog" aria-modal="true" aria-label={`Sign in: ${signInFor.name}`}>
          <div className="w-full max-w-md mx-4 rounded-lg bg-white dark:bg-gray-900 shadow-xl p-4 space-y-3">
            <div className="flex items-center justify-between">
              <h3 className="text-sm font-semibold">Sign in: {signInFor.name}</h3>
              <button type="button" className="text-xs px-2 py-1 rounded hover:bg-gray-200 dark:hover:bg-gray-700" onClick={() => setSignInFor(null)}>
                Close
              </button>
            </div>
            <OAuthSignIn
              connectionId={signInFor.id}
              protocol={signInFor.protocol}
              savedClientId={signInFor.extras?.oauth?.client_id ?? null}
              onSignedIn={(r) => {
                const id = signInFor.id;
                setSignInFor(null);
                setSignedIn((prev) => ({ ...prev, [id]: true }));
                setTransferStatus(r.account ? `Signed in as ${r.account}.` : "Signed in.");
                refresh();
                void handleConnect(id);
              }}
            />
          </div>
        </div>
      )}
    </div>
  );
}

export default ConnectionPanel;
