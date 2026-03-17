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
 */
import { useState, useCallback, useEffect, useMemo, useRef } from "react";
import { tauriInvoke } from "@/hooks/use-tauri";

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
}

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

const FALLBACK_PROTOCOLS: ProtocolOption[] = [
  { value: "sftp", label: "SFTP", port: 22 },
  { value: "ftp", label: "FTP", port: 21 },
  { value: "ftps", label: "FTPS", port: 990 },
  { value: "webdav", label: "WebDAV", port: 443 },
  { value: "smb", label: "SMB", port: 445 },
  { value: "nfs", label: "NFS", port: 2049 },
  { value: "s3", label: "S3", port: 443 },
  { value: "azure_blob", label: "Azure Blob Storage", port: 443 },
  { value: "swift", label: "OpenStack Swift", port: 5000 },
  { value: "google_drive", label: "Google Drive", port: 443 },
  { value: "dropbox", label: "Dropbox", port: 443 },
  { value: "onedrive", label: "OneDrive", port: 443 },
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
  swift: 5000,
  google_drive: 443,
  dropbox: 443,
  onedrive: 443,
  https: 443,
  http: 80,
};

// S3-compatible provider presets
const S3_PRESETS = [
  { id: "aws", label: "Amazon S3", endpoint: "", region: "us-east-1", helpText: "" },
  { id: "wasabi", label: "Wasabi", endpoint: "s3.wasabisys.com", region: "us-east-1", helpText: "" },
  { id: "minio", label: "MinIO", endpoint: "localhost:9000", region: "us-east-1", helpText: "Self-hosted MinIO instance" },
  {
    id: "gcs",
    label: "Google Cloud Storage (S3 Compatible)",
    endpoint: "storage.googleapis.com",
    region: "auto",
    helpText: "Create HMAC keys in GCP Console > Cloud Storage > Settings > Interoperability",
  },
] as const;

// ── Helpers ──

function formatFileSize(bytes: number): string {
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  const size = bytes / Math.pow(1024, i);
  return `${size.toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
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
  onKeyGenerated: (keyPath: string) => void;
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
      onKeyGenerated(res.private_key_path);
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
              <input
                type="password"
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
  const [selectedS3Preset, setSelectedS3Preset] = useState("");

  // Azure Blob Storage fields
  const [azureAccountName, setAzureAccountName] = useState("");
  const [azureAuthMethod, setAzureAuthMethod] = useState<
    "account_key" | "sas_token" | "connection_string" | "azure_ad"
  >("account_key");
  const [azureSecret, setAzureSecret] = useState("");
  const [azureCustomEndpoint, setAzureCustomEndpoint] = useState("");

  // OpenStack Swift fields
  const [swiftAuthUrl, setSwiftAuthUrl] = useState("");
  const [swiftProjectName, setSwiftProjectName] = useState("");
  const [swiftDomainName, setSwiftDomainName] = useState("Default");
  const [swiftRegion, setSwiftRegion] = useState("");

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
        { host: host.trim() },
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
  const handleS3PresetSelect = useCallback(
    (presetId: string) => {
      setSelectedS3Preset(presetId);
      const preset = S3_PRESETS.find((p) => p.id === presetId);
      if (preset && preset.endpoint) {
        setHost(preset.endpoint);
      }
    },
    []
  );

  const isFtpProtocol = protocol === "ftp" || protocol === "ftps";
  const isSshProtocol = protocol === "sftp";
  const isAzureBlob = protocol === "azure_blob";
  const isSwift = protocol === "swift";
  const isS3 = protocol === "s3";

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
      </div>
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
          required
          placeholder="example.com"
          className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
        />
      </div>
      <div className="grid grid-cols-2 gap-2">
        <div>
          <label className="block text-xs font-medium mb-1">Username</label>
          <input
            type="text"
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
          />
        </div>
        <div>
          <label className="block text-xs font-medium mb-1">Password</label>
          <input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            placeholder={initialData ? "(unchanged)" : ""}
            className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
          />
        </div>
      </div>
      {/* SSH Key Generation (SFTP only) */}
      {isSshProtocol && (
        <SshKeyGenSection
          onKeyGenerated={(keyPath) => {
            setPassword(keyPath);
            setSshConfigHint("key generated");
          }}
        />
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
      {/* S3-compatible provider presets (S3 only) */}
      {isS3 && (
        <div>
          <label className="block text-xs font-medium mb-1">S3 Provider Preset</label>
          <select
            value={selectedS3Preset}
            onChange={(e) => handleS3PresetSelect(e.target.value)}
            className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
          >
            <option value="">Custom / Amazon S3</option>
            {S3_PRESETS.filter((p) => p.id !== "aws").map((p) => (
              <option key={p.id} value={p.id}>
                {p.label}
              </option>
            ))}
          </select>
          {selectedS3Preset && (() => {
            const preset = S3_PRESETS.find((p) => p.id === selectedS3Preset);
            return preset?.helpText ? (
              <p className="text-[10px] text-blue-500 dark:text-blue-400 mt-0.5">
                {preset.helpText}
              </p>
            ) : null;
          })()}
        </div>
      )}
      {/* Azure Blob Storage fields */}
      {isAzureBlob && (
        <div className="space-y-2">
          <div>
            <label className="block text-xs font-medium mb-1">Account Name</label>
            <input
              type="text"
              value={azureAccountName}
              onChange={(e) => setAzureAccountName(e.target.value)}
              placeholder="mystorageaccount"
              className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
            />
          </div>
          <div>
            <label className="block text-xs font-medium mb-1">Auth Method</label>
            <select
              value={azureAuthMethod}
              onChange={(e) => setAzureAuthMethod(e.target.value as typeof azureAuthMethod)}
              className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
            >
              <option value="account_key">Account Key</option>
              <option value="sas_token">SAS Token</option>
              <option value="connection_string">Connection String</option>
              <option value="azure_ad">Azure AD</option>
            </select>
          </div>
          {azureAuthMethod !== "azure_ad" && (
            <div>
              <label className="block text-xs font-medium mb-1">
                {azureAuthMethod === "account_key"
                  ? "Account Key"
                  : azureAuthMethod === "sas_token"
                    ? "SAS Token"
                    : "Connection String"}
              </label>
              <input
                type="password"
                value={azureSecret}
                onChange={(e) => setAzureSecret(e.target.value)}
                placeholder={
                  azureAuthMethod === "connection_string"
                    ? "DefaultEndpointsProtocol=https;AccountName=..."
                    : ""
                }
                className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
              />
            </div>
          )}
          <div>
            <label className="block text-xs font-medium mb-1">
              Custom Endpoint URL
              <span className="text-gray-400 dark:text-gray-500 font-normal"> (optional)</span>
            </label>
            <input
              type="text"
              value={azureCustomEndpoint}
              onChange={(e) => setAzureCustomEndpoint(e.target.value)}
              placeholder="For Azure Gov, China, or Azurite"
              className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
            />
            <p className="text-[10px] text-gray-400 dark:text-gray-500 mt-0.5">
              Leave blank for standard Azure public cloud
            </p>
          </div>
        </div>
      )}
      {/* OpenStack Swift fields */}
      {isSwift && (
        <div className="space-y-2">
          <div>
            <label className="block text-xs font-medium mb-1">Auth URL</label>
            <input
              type="text"
              value={swiftAuthUrl}
              onChange={(e) => setSwiftAuthUrl(e.target.value)}
              placeholder="https://keystone.example.com:5000/v3"
              className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
            />
          </div>
          <div>
            <label className="block text-xs font-medium mb-1">Project Name</label>
            <input
              type="text"
              value={swiftProjectName}
              onChange={(e) => setSwiftProjectName(e.target.value)}
              placeholder="my-project"
              className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
            />
          </div>
          <div>
            <label className="block text-xs font-medium mb-1">Domain Name</label>
            <input
              type="text"
              value={swiftDomainName}
              onChange={(e) => setSwiftDomainName(e.target.value)}
              placeholder="Default"
              className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
            />
          </div>
          <div>
            <label className="block text-xs font-medium mb-1">
              Region
              <span className="text-gray-400 dark:text-gray-500 font-normal"> (optional)</span>
            </label>
            <input
              type="text"
              value={swiftRegion}
              onChange={(e) => setSwiftRegion(e.target.value)}
              placeholder="RegionOne"
              className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
            />
          </div>
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
                      <input
                        type="password"
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
}: {
  connection: ConnectionProfile;
  onConnect: (id: string) => void;
  onTest: (id: string) => void;
  onEdit: (conn: ConnectionProfile) => void;
  onDelete: (id: string) => void;
  testResult: ConnectionTestResult | null;
  testing: boolean;
  isConnected: boolean;
}) {
  return (
    <div className="flex items-center justify-between px-3 py-2 border-b border-gray-200 dark:border-gray-700 last:border-b-0 hover:bg-gray-50 dark:hover:bg-gray-800/50">
      <button
        onClick={() => onConnect(connection.id)}
        className="flex-1 min-w-0 text-left"
        type="button"
      >
        <div className="flex items-center gap-2">
          {/* Connection status indicator */}
          <span
            className={`w-2 h-2 rounded-full flex-shrink-0 ${
              isConnected ? "bg-green-500" : "bg-gray-300 dark:bg-gray-600"
            }`}
            title={isConnected ? "Connected" : "Disconnected"}
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
      </button>
      <div className="flex items-center gap-1 ml-2">
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
  }, []);

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
        await tauriInvoke("save_connection", data as Record<string, unknown>);
        setShowForm(false);
        setEditingConnection(null);
        refresh();
      } catch (err) {
        console.error("Failed to save connection:", err);
      }
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
        });

        // 3. List remote directory contents
        const initialPath = conn.remote_path || "/";
        const files = await tauriInvoke<RemoteFileEntry[]>(
          "connector_list_remote",
          { protocol: conn.protocol, path: initialPath },
          []
        );

        // 4. Update state
        setConnectedId(id);
        setConnectedProtocol(conn.protocol);
        setRemotePath(initialPath);
        setRemoteFiles(files);
        // Mark as connected immediately
        setConnectedProtocols((prev) => new Set([...prev, conn.protocol]));
      } catch (err) {
        console.error("Failed to connect:", err);
        setBrowseError(
          err instanceof Error ? err.message : String(err)
        );
      } finally {
        setBrowsing(false);
      }
    },
    []
  );

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
  }, [connectedProtocol]);

  const handleBrowse = useCallback(
    async (path: string) => {
      if (!connectedProtocol) return;
      setBrowsing(true);
      setBrowseError(null);
      try {
        const files = await tauriInvoke<RemoteFileEntry[]>(
          "connector_list_remote",
          { protocol: connectedProtocol, path },
          []
        );
        setRemoteFiles(files);
        setRemotePath(path);
      } catch (err) {
        setBrowseError(
          err instanceof Error ? err.message : "Browse failed"
        );
      } finally {
        setBrowsing(false);
      }
    },
    [connectedProtocol]
  );

  // Derive the connected connection name for the header
  const connectedName = useMemo(() => {
    if (!connectedId) return null;
    const conn = connections.find((c) => c.id === connectedId);
    return conn?.name || connectedId;
  }, [connectedId, connections]);

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
            setImportError(err instanceof Error ? err.message : String(err));
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
              sourceApp: pendingImportSource,
            });
            refresh();
            setImportError(null);
          } catch (err) {
            console.error(`${pendingImportSource} import failed:`, err);
            setImportError(err instanceof Error ? err.message : String(err));
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

  // Group connections by group_id
  const ungrouped = connections.filter((c) => !c.group_id);
  const grouped = groups.map((g) => ({
    group: g,
    connections: connections.filter((c) => c.group_id === g.id),
  }));

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
          </div>

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
                <button
                  key={entry.path}
                  onClick={() => {
                    if (entry.is_dir) {
                      handleBrowse(entry.path);
                    }
                  }}
                  className={`w-full text-left flex items-center justify-between px-3 py-1.5 text-sm border-b border-gray-100 dark:border-gray-800 last:border-b-0 ${
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
                    {entry.modified && (
                      <span className="text-xs text-gray-400 dark:text-gray-500">
                        {formatModified(entry.modified)}
                      </span>
                    )}
                  </div>
                </button>
              ))}
          </div>
        </div>
      )}
    </div>
  );
}

export default ConnectionPanel;
