/**
 * Vault Manager Panel
 *
 * Features:
 * - List and manage encrypted vaults (lock/unlock)
 * - Create new vaults with configurable encryption patterns
 * - File encrypt/decrypt operations on unlocked vaults
 * - Transport security checks (basic + advanced custom)
 * - Encryption policy management
 * - Cloud encrypt/decrypt (base64 data for upload/download)
 */
import { useState, useCallback, useEffect } from "react";
import { tauriInvoke } from "@/hooks/use-tauri";

// ── Types (mirrors Rust types) ──

interface VaultInfo {
  id: string;
  name: string;
  path: string;
  is_unlocked: boolean;
  created_at: string;
  cryptomator_compat: boolean;
}

interface TransportSecurityCheck {
  secure: boolean;
  warnings: string[];
  recommendations: string[];
}

interface EncryptionPolicy {
  id: string;
  name: string;
  connector_type: string;
  destination_pattern: string;
  file_pattern: string;
  require_encryption: boolean;
}

interface CustomSecuritySettings {
  require_tls: boolean;
  min_tls_version: string;
  verify_certificates: boolean;
  allowed_ciphers: string[];
}

// ── Create Vault Form ──

function CreateVaultForm({ onCreated, onCancel }: {
  onCreated: () => void;
  onCancel: () => void;
}) {
  const [path, setPath] = useState("");
  const [name, setName] = useState("");
  const [password, setPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [encryptPatterns, setEncryptPatterns] = useState("");
  const [excludePatterns, setExcludePatterns] = useState("");
  const [cryptomatorCompat, setCryptomatorCompat] = useState(false);
  const [error, setError] = useState("");
  const [creating, setCreating] = useState(false);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError("");

    if (password.length < 8) {
      setError("Password must be at least 8 characters.");
      return;
    }
    if (password !== confirmPassword) {
      setError("Passwords do not match.");
      return;
    }

    setCreating(true);
    try {
      await tauriInvoke<VaultInfo>("create_vault", {
        path,
        name,
        password,
        encryptPatterns: encryptPatterns
          ? encryptPatterns.split(",").map((s) => s.trim())
          : undefined,
        excludePatterns: excludePatterns
          ? excludePatterns.split(",").map((s) => s.trim())
          : undefined,
        cryptomatorCompat: cryptomatorCompat || undefined,
      });
      onCreated();
    } catch (err) {
      setError(String(err));
    } finally {
      setCreating(false);
    }
  };

  return (
    <form onSubmit={handleSubmit} className="space-y-3 p-3">
      <div>
        <label className="block text-xs font-medium mb-1">Vault Path</label>
        <input
          type="text"
          value={path}
          onChange={(e) => setPath(e.target.value)}
          required
          placeholder="/path/to/vault"
          className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
        />
      </div>
      <div>
        <label className="block text-xs font-medium mb-1">Vault Name</label>
        <input
          type="text"
          value={name}
          onChange={(e) => setName(e.target.value)}
          required
          placeholder="My Vault"
          className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
        />
      </div>
      <div className="grid grid-cols-2 gap-2">
        <div>
          <label className="block text-xs font-medium mb-1">Password</label>
          <input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            required
            minLength={8}
            placeholder="Min 8 characters"
            className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
          />
        </div>
        <div>
          <label className="block text-xs font-medium mb-1">Confirm Password</label>
          <input
            type="password"
            value={confirmPassword}
            onChange={(e) => setConfirmPassword(e.target.value)}
            required
            minLength={8}
            className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
          />
        </div>
      </div>
      <div>
        <label className="block text-xs font-medium mb-1">
          Encrypt Patterns <span className="text-gray-400 font-normal">(optional, comma-separated)</span>
        </label>
        <input
          type="text"
          value={encryptPatterns}
          onChange={(e) => setEncryptPatterns(e.target.value)}
          placeholder="*.doc, *.pdf"
          className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
        />
      </div>
      <div>
        <label className="block text-xs font-medium mb-1">
          Exclude Patterns <span className="text-gray-400 font-normal">(optional, comma-separated)</span>
        </label>
        <input
          type="text"
          value={excludePatterns}
          onChange={(e) => setExcludePatterns(e.target.value)}
          placeholder="*.tmp, .DS_Store"
          className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
        />
      </div>
      <label className="flex items-center gap-2 text-sm cursor-pointer">
        <input
          type="checkbox"
          checked={cryptomatorCompat}
          onChange={(e) => setCryptomatorCompat(e.target.checked)}
          className="rounded"
        />
        Cryptomator compatible
      </label>
      {error && (
        <p className="text-xs text-red-500">{error}</p>
      )}
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
          disabled={creating}
          className="text-sm px-3 py-1.5 rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-50"
        >
          {creating ? "Creating..." : "Create Vault"}
        </button>
      </div>
    </form>
  );
}

// ── Vault Item ──

function VaultItem({
  vault,
  onToggleLock,
  onSelect,
  toggling,
  selected,
}: {
  vault: VaultInfo;
  onToggleLock: (vault: VaultInfo) => void;
  onSelect: (vault: VaultInfo) => void;
  toggling: boolean;
  selected: boolean;
}) {
  return (
    <div
      className={`flex items-center justify-between px-3 py-2 border-b border-gray-200 dark:border-gray-700 last:border-b-0 hover:bg-gray-50 dark:hover:bg-gray-800/50 ${
        selected ? "bg-blue-50 dark:bg-blue-900/20" : ""
      }`}
    >
      <button
        onClick={() => onSelect(vault)}
        className="flex-1 min-w-0 text-left"
        type="button"
      >
        <div className="flex items-center gap-2">
          <span
            className={`w-2 h-2 rounded-full flex-shrink-0 ${
              vault.is_unlocked ? "bg-green-500" : "bg-gray-400"
            }`}
            title={vault.is_unlocked ? "Unlocked" : "Locked"}
          />
          <span className="text-sm font-medium truncate">{vault.name}</span>
          {vault.cryptomator_compat && (
            <span className="text-xs text-gray-400 dark:text-gray-500">CM</span>
          )}
        </div>
        <div className="text-xs text-gray-500 dark:text-gray-400 truncate ml-4">
          {vault.path}
        </div>
      </button>
      <button
        onClick={() => onToggleLock(vault)}
        disabled={toggling}
        className={`text-xs px-2 py-1 rounded ml-2 ${
          vault.is_unlocked
            ? "hover:bg-red-100 dark:hover:bg-red-900/30 text-red-600 dark:text-red-400"
            : "hover:bg-green-100 dark:hover:bg-green-900/30 text-green-600 dark:text-green-400"
        } disabled:opacity-50`}
        type="button"
        title={vault.is_unlocked ? "Lock vault" : "Unlock vault"}
      >
        {toggling ? "..." : vault.is_unlocked ? "Lock" : "Unlock"}
      </button>
    </div>
  );
}

// ── Unlock Prompt ──

function UnlockPrompt({
  vaultName,
  onUnlock,
  onCancel,
}: {
  vaultName: string;
  onUnlock: (password: string) => void;
  onCancel: () => void;
}) {
  const [password, setPassword] = useState("");

  return (
    <div className="p-3 border-b border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-800/50">
      <p className="text-xs text-gray-500 dark:text-gray-400 mb-2">
        Enter password for <strong>{vaultName}</strong>
      </p>
      <div className="flex gap-2">
        <input
          type="password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && password) onUnlock(password);
          }}
          placeholder="Vault password"
          className="flex-1 text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
          autoFocus
        />
        <button
          onClick={() => onUnlock(password)}
          disabled={!password}
          className="text-xs px-3 py-1.5 rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-50"
          type="button"
        >
          Unlock
        </button>
        <button
          onClick={onCancel}
          className="text-xs px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 hover:bg-gray-100 dark:hover:bg-gray-800"
          type="button"
        >
          Cancel
        </button>
      </div>
    </div>
  );
}

// ── File Operations Section ──

function FileOperations({ vaultId }: { vaultId: string }) {
  const [encryptSource, setEncryptSource] = useState("");
  const [encryptRelative, setEncryptRelative] = useState("");
  const [decryptRelative, setDecryptRelative] = useState("");
  const [decryptDest, setDecryptDest] = useState("");
  const [result, setResult] = useState("");
  const [busy, setBusy] = useState(false);

  const handleEncrypt = async () => {
    if (!encryptSource || !encryptRelative) return;
    setBusy(true);
    setResult("");
    try {
      const encrypted = await tauriInvoke<string>("vault_encrypt_file", {
        vaultId,
        sourcePath: encryptSource,
        relativePath: encryptRelative,
      });
      setResult(`Encrypted: ${encrypted}`);
      setEncryptSource("");
      setEncryptRelative("");
    } catch (err) {
      setResult(`Encrypt error: ${err}`);
    } finally {
      setBusy(false);
    }
  };

  const handleDecrypt = async () => {
    if (!decryptRelative || !decryptDest) return;
    setBusy(true);
    setResult("");
    try {
      await tauriInvoke("vault_decrypt_file", {
        vaultId,
        relativePath: decryptRelative,
        destPath: decryptDest,
      });
      setResult("File decrypted successfully.");
      setDecryptRelative("");
      setDecryptDest("");
    } catch (err) {
      setResult(`Decrypt error: ${err}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="p-3 space-y-3">
      <h3 className="text-xs font-semibold text-gray-500 dark:text-gray-400 uppercase tracking-wider">
        File Operations
      </h3>

      {/* Encrypt */}
      <div className="space-y-1">
        <label className="block text-xs font-medium">Encrypt File</label>
        <input
          type="text"
          value={encryptSource}
          onChange={(e) => setEncryptSource(e.target.value)}
          placeholder="Source file path"
          className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
        />
        <input
          type="text"
          value={encryptRelative}
          onChange={(e) => setEncryptRelative(e.target.value)}
          placeholder="Relative path in vault"
          className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
        />
        <button
          onClick={handleEncrypt}
          disabled={busy || !encryptSource || !encryptRelative}
          className="text-xs px-3 py-1.5 rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-50"
          type="button"
        >
          {busy ? "Encrypting..." : "Encrypt"}
        </button>
      </div>

      {/* Decrypt */}
      <div className="space-y-1">
        <label className="block text-xs font-medium">Decrypt File</label>
        <input
          type="text"
          value={decryptRelative}
          onChange={(e) => setDecryptRelative(e.target.value)}
          placeholder="Relative path in vault"
          className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
        />
        <input
          type="text"
          value={decryptDest}
          onChange={(e) => setDecryptDest(e.target.value)}
          placeholder="Destination file path"
          className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
        />
        <button
          onClick={handleDecrypt}
          disabled={busy || !decryptRelative || !decryptDest}
          className="text-xs px-3 py-1.5 rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-50"
          type="button"
        >
          {busy ? "Decrypting..." : "Decrypt"}
        </button>
      </div>

      {result && (
        <p className={`text-xs ${result.startsWith("Encrypt error") || result.startsWith("Decrypt error") ? "text-red-500" : "text-green-500"}`}>
          {result}
        </p>
      )}
    </div>
  );
}

// ── Cloud Encrypt Section ──

function CloudEncrypt({ vaultId }: { vaultId: string }) {
  const [dataBase64, setDataBase64] = useState("");
  const [fileNameInput, setFileNameInput] = useState("");
  const [result, setResult] = useState("");
  const [busy, setBusy] = useState(false);

  const handleEncrypt = async () => {
    if (!dataBase64 || !fileNameInput) return;
    setBusy(true);
    setResult("");
    try {
      const encrypted = await tauriInvoke<string>("encrypt_for_upload", {
        vaultId,
        dataBase64,
        fileName: fileNameInput,
      });
      setResult(encrypted);
    } catch (err) {
      setResult(`Error: ${err}`);
    } finally {
      setBusy(false);
    }
  };

  const handleCopyResult = () => {
    if (result && !result.startsWith("Error:")) {
      navigator.clipboard.writeText(result).catch(() => {});
    }
  };

  const handleFileSelect = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    setFileNameInput(file.name);
    const reader = new FileReader();
    reader.onload = () => {
      const base64 = (reader.result as string).split(",")[1] || "";
      setDataBase64(base64);
    };
    reader.readAsDataURL(file);
  };

  return (
    <div className="p-3 space-y-2 border-t border-gray-100 dark:border-gray-700/50">
      <h3 className="text-xs font-semibold text-gray-500 dark:text-gray-400 uppercase tracking-wider">
        Cloud Encrypt (for Upload)
      </h3>
      <div className="space-y-1">
        <label className="block text-xs font-medium">Select File or Paste Base64</label>
        <input
          type="file"
          onChange={handleFileSelect}
          className="w-full text-xs"
        />
        <textarea
          value={dataBase64}
          onChange={(e) => setDataBase64(e.target.value)}
          placeholder="Paste base64 data here..."
          rows={3}
          className="w-full text-xs px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900 font-mono"
        />
        <input
          type="text"
          value={fileNameInput}
          onChange={(e) => setFileNameInput(e.target.value)}
          placeholder="File name (e.g. document.pdf)"
          className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
        />
        <div className="flex gap-2">
          <button
            onClick={handleEncrypt}
            disabled={busy || !dataBase64 || !fileNameInput}
            className="text-xs px-3 py-1.5 rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-50"
            type="button"
          >
            {busy ? "Encrypting..." : "Encrypt for Upload"}
          </button>
          {result && !result.startsWith("Error:") && (
            <button
              onClick={handleCopyResult}
              className="text-xs px-3 py-1.5 rounded bg-gray-100 hover:bg-gray-200 dark:bg-gray-700 dark:hover:bg-gray-600 text-gray-700 dark:text-gray-300"
              type="button"
            >
              Copy Result
            </button>
          )}
        </div>
      </div>
      {result && (
        <div className={`text-xs break-all ${result.startsWith("Error:") ? "text-red-500" : "text-green-500"}`}>
          {result.startsWith("Error:")
            ? result
            : `Encrypted (${result.length} chars): ${result.substring(0, 80)}...`}
        </div>
      )}
    </div>
  );
}

// ── Cloud Decrypt Section ──

function CloudDecrypt({ vaultId }: { vaultId: string }) {
  const [dataBase64, setDataBase64] = useState("");
  const [fileNameInput, setFileNameInput] = useState("");
  const [result, setResult] = useState("");
  const [busy, setBusy] = useState(false);

  const handleDecrypt = async () => {
    if (!dataBase64 || !fileNameInput) return;
    setBusy(true);
    setResult("");
    try {
      const decrypted = await tauriInvoke<string>("decrypt_from_download", {
        vaultId,
        dataBase64,
        fileName: fileNameInput,
      });
      setResult(decrypted);
    } catch (err) {
      setResult(`Error: ${err}`);
    } finally {
      setBusy(false);
    }
  };

  const handleDownload = () => {
    if (!result || result.startsWith("Error:")) return;
    try {
      const binaryString = atob(result);
      const bytes = new Uint8Array(binaryString.length);
      for (let i = 0; i < binaryString.length; i++) {
        bytes[i] = binaryString.charCodeAt(i);
      }
      const blob = new Blob([bytes]);
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = fileNameInput || "decrypted-file";
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      URL.revokeObjectURL(url);
    } catch {
      console.error("Failed to download decrypted data");
    }
  };

  return (
    <div className="p-3 space-y-2 border-t border-gray-100 dark:border-gray-700/50">
      <h3 className="text-xs font-semibold text-gray-500 dark:text-gray-400 uppercase tracking-wider">
        Cloud Decrypt (from Download)
      </h3>
      <div className="space-y-1">
        <label className="block text-xs font-medium">Paste Encrypted Base64 Data</label>
        <textarea
          value={dataBase64}
          onChange={(e) => setDataBase64(e.target.value)}
          placeholder="Paste encrypted base64 data here..."
          rows={3}
          className="w-full text-xs px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900 font-mono"
        />
        <input
          type="text"
          value={fileNameInput}
          onChange={(e) => setFileNameInput(e.target.value)}
          placeholder="Original file name (e.g. document.pdf)"
          className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
        />
        <div className="flex gap-2">
          <button
            onClick={handleDecrypt}
            disabled={busy || !dataBase64 || !fileNameInput}
            className="text-xs px-3 py-1.5 rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-50"
            type="button"
          >
            {busy ? "Decrypting..." : "Decrypt"}
          </button>
          {result && !result.startsWith("Error:") && (
            <button
              onClick={handleDownload}
              className="text-xs px-3 py-1.5 rounded bg-green-500 text-white hover:bg-green-600"
              type="button"
            >
              Download Decrypted
            </button>
          )}
        </div>
      </div>
      {result && (
        <div className={`text-xs break-all ${result.startsWith("Error:") ? "text-red-500" : "text-green-500"}`}>
          {result.startsWith("Error:")
            ? result
            : `Decrypted (${result.length} chars base64)`}
        </div>
      )}
    </div>
  );
}

// ── Transport Security Section ──

function TransportSecurity() {
  const [protocol, setProtocol] = useState("https");
  const [host, setHost] = useState("");
  const [port, setPort] = useState("");
  const [checking, setChecking] = useState(false);
  const [result, setResult] = useState<TransportSecurityCheck | null>(null);
  const [error, setError] = useState("");

  // Advanced custom security toggle
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [requireTls, setRequireTls] = useState(true);
  const [minTlsVersion, setMinTlsVersion] = useState("1.2");
  const [verifyCertificates, setVerifyCertificates] = useState(true);
  const [allowedCiphers, setAllowedCiphers] = useState("");

  const handleCheck = async () => {
    if (!protocol || !host) return;
    setChecking(true);
    setError("");
    setResult(null);
    try {
      let check: TransportSecurityCheck;
      if (showAdvanced) {
        // Use custom security check
        const security: CustomSecuritySettings = {
          require_tls: requireTls,
          min_tls_version: minTlsVersion,
          verify_certificates: verifyCertificates,
          allowed_ciphers: allowedCiphers
            ? allowedCiphers.split(",").map((s) => s.trim()).filter(Boolean)
            : [],
        };
        check = await tauriInvoke<TransportSecurityCheck>(
          "check_transport_security_custom",
          {
            protocol,
            host,
            port: port ? parseInt(port, 10) : undefined,
            security,
          }
        );
      } else {
        check = await tauriInvoke<TransportSecurityCheck>(
          "check_transport_security",
          {
            protocol,
            host,
            port: port ? parseInt(port, 10) : undefined,
          }
        );
      }
      setResult(check);
    } catch (err) {
      setError(String(err));
    } finally {
      setChecking(false);
    }
  };

  return (
    <div className="p-3 space-y-2">
      <h3 className="text-xs font-semibold text-gray-500 dark:text-gray-400 uppercase tracking-wider">
        Transport Security
      </h3>
      <div className="grid grid-cols-3 gap-2">
        <div>
          <label className="block text-xs font-medium mb-1">Protocol</label>
          <select
            value={protocol}
            onChange={(e) => setProtocol(e.target.value)}
            className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
          >
            <option value="https">HTTPS</option>
            <option value="sftp">SFTP</option>
            <option value="ftps">FTPS</option>
            <option value="ftp">FTP</option>
            <option value="webdav">WebDAV</option>
            <option value="smb">SMB</option>
          </select>
        </div>
        <div>
          <label className="block text-xs font-medium mb-1">Host</label>
          <input
            type="text"
            value={host}
            onChange={(e) => setHost(e.target.value)}
            placeholder="example.com"
            className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
          />
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

      {/* Advanced toggle */}
      <div className="flex items-center gap-2">
        <button
          onClick={handleCheck}
          disabled={checking || !host}
          className="text-xs px-3 py-1.5 rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-50"
          type="button"
        >
          {checking ? "Checking..." : "Check Security"}
        </button>
        <label className="flex items-center gap-1.5 text-xs cursor-pointer">
          <input
            type="checkbox"
            checked={showAdvanced}
            onChange={(e) => setShowAdvanced(e.target.checked)}
            className="h-3.5 w-3.5 rounded border-gray-300 dark:border-gray-600"
          />
          Advanced
        </label>
      </div>

      {/* Advanced custom security fields */}
      {showAdvanced && (
        <div className="p-2 rounded border border-gray-200 dark:border-gray-700 space-y-2 bg-gray-50 dark:bg-gray-800/50">
          <div className="text-xs font-medium text-gray-600 dark:text-gray-400">
            Custom Security Settings
          </div>
          <div className="grid grid-cols-2 gap-2">
            <label className="flex items-center gap-1.5 text-xs cursor-pointer">
              <input
                type="checkbox"
                checked={requireTls}
                onChange={(e) => setRequireTls(e.target.checked)}
                className="h-3.5 w-3.5 rounded border-gray-300 dark:border-gray-600"
              />
              Require TLS
            </label>
            <label className="flex items-center gap-1.5 text-xs cursor-pointer">
              <input
                type="checkbox"
                checked={verifyCertificates}
                onChange={(e) => setVerifyCertificates(e.target.checked)}
                className="h-3.5 w-3.5 rounded border-gray-300 dark:border-gray-600"
              />
              Verify Certificates
            </label>
          </div>
          <div>
            <label className="block text-xs font-medium mb-1">Min TLS Version</label>
            <select
              value={minTlsVersion}
              onChange={(e) => setMinTlsVersion(e.target.value)}
              className="w-full text-xs px-2 py-1 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
            >
              <option value="1.0">TLS 1.0</option>
              <option value="1.1">TLS 1.1</option>
              <option value="1.2">TLS 1.2</option>
              <option value="1.3">TLS 1.3</option>
            </select>
          </div>
          <div>
            <label className="block text-xs font-medium mb-1">
              Allowed Ciphers <span className="text-gray-400 font-normal">(comma-separated)</span>
            </label>
            <input
              type="text"
              value={allowedCiphers}
              onChange={(e) => setAllowedCiphers(e.target.value)}
              placeholder="AES-256-GCM, CHACHA20-POLY1305"
              className="w-full text-xs px-2 py-1 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
            />
          </div>
        </div>
      )}

      {error && <p className="text-xs text-red-500">{error}</p>}
      {result && (
        <div className="text-xs space-y-1 mt-1">
          <div className="flex items-center gap-1">
            <span
              className={`w-2 h-2 rounded-full ${result.secure ? "bg-green-500" : "bg-red-500"}`}
            />
            <span className={result.secure ? "text-green-600 dark:text-green-400" : "text-red-600 dark:text-red-400"}>
              {result.secure ? "Secure" : "Not Secure"}
            </span>
          </div>
          {result.warnings.length > 0 && (
            <div>
              <span className="font-medium text-yellow-600 dark:text-yellow-400">Warnings:</span>
              <ul className="list-disc list-inside text-gray-500 dark:text-gray-400">
                {result.warnings.map((w, i) => (
                  <li key={i}>{w}</li>
                ))}
              </ul>
            </div>
          )}
          {result.recommendations.length > 0 && (
            <div>
              <span className="font-medium">Recommendations:</span>
              <ul className="list-disc list-inside text-gray-500 dark:text-gray-400">
                {result.recommendations.map((r, i) => (
                  <li key={i}>{r}</li>
                ))}
              </ul>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

// ── Encryption Policies Section ──

function EncryptionPolicies() {
  const [policies, setPolicies] = useState<EncryptionPolicy[]>([]);
  const [showAddForm, setShowAddForm] = useState(false);
  const [newName, setNewName] = useState("");
  const [newConnectorType, setNewConnectorType] = useState("");
  const [newDestPattern, setNewDestPattern] = useState("");
  const [newFilePattern, setNewFilePattern] = useState("");
  const [newRequireEncryption, setNewRequireEncryption] = useState(true);
  const [error, setError] = useState("");

  const refresh = useCallback(async () => {
    try {
      const list = await tauriInvoke<EncryptionPolicy[]>(
        "list_encryption_policies",
        undefined,
        []
      );
      setPolicies(list);
    } catch (err) {
      console.error("Failed to load policies:", err);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const handleAdd = async (e: React.FormEvent) => {
    e.preventDefault();
    setError("");
    try {
      await tauriInvoke("add_encryption_policy", {
        policy: {
          name: newName,
          connector_type: newConnectorType,
          destination_pattern: newDestPattern,
          file_pattern: newFilePattern,
          require_encryption: newRequireEncryption,
        },
      });
      setShowAddForm(false);
      setNewName("");
      setNewConnectorType("");
      setNewDestPattern("");
      setNewFilePattern("");
      setNewRequireEncryption(true);
      refresh();
    } catch (err) {
      setError(String(err));
    }
  };

  const handleRemove = async (policyId: string) => {
    try {
      await tauriInvoke("remove_encryption_policy", { policyId });
      refresh();
    } catch (err) {
      console.error("Failed to remove policy:", err);
    }
  };

  return (
    <div className="p-3 space-y-2">
      <div className="flex items-center justify-between">
        <h3 className="text-xs font-semibold text-gray-500 dark:text-gray-400 uppercase tracking-wider">
          Encryption Policies
        </h3>
        <button
          onClick={() => setShowAddForm(!showAddForm)}
          className="text-xs px-2 py-1 rounded bg-blue-500 text-white hover:bg-blue-600"
          type="button"
        >
          {showAddForm ? "Cancel" : "+ Add"}
        </button>
      </div>

      {showAddForm && (
        <form onSubmit={handleAdd} className="space-y-2 p-2 rounded border border-gray-200 dark:border-gray-700">
          <div>
            <label className="block text-xs font-medium mb-1">Policy Name</label>
            <input
              type="text"
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              required
              className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
            />
          </div>
          <div className="grid grid-cols-2 gap-2">
            <div>
              <label className="block text-xs font-medium mb-1">Connector Type</label>
              <input
                type="text"
                value={newConnectorType}
                onChange={(e) => setNewConnectorType(e.target.value)}
                required
                placeholder="sftp, s3, ..."
                className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
              />
            </div>
            <div>
              <label className="block text-xs font-medium mb-1">File Pattern</label>
              <input
                type="text"
                value={newFilePattern}
                onChange={(e) => setNewFilePattern(e.target.value)}
                required
                placeholder="*.pdf"
                className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
              />
            </div>
          </div>
          <div>
            <label className="block text-xs font-medium mb-1">Destination Pattern</label>
            <input
              type="text"
              value={newDestPattern}
              onChange={(e) => setNewDestPattern(e.target.value)}
              required
              placeholder="/remote/path/*"
              className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
            />
          </div>
          <label className="flex items-center gap-2 text-xs cursor-pointer">
            <input
              type="checkbox"
              checked={newRequireEncryption}
              onChange={(e) => setNewRequireEncryption(e.target.checked)}
              className="rounded"
            />
            Require encryption
          </label>
          {error && <p className="text-xs text-red-500">{error}</p>}
          <div className="flex justify-end">
            <button
              type="submit"
              className="text-xs px-3 py-1.5 rounded bg-blue-500 text-white hover:bg-blue-600"
            >
              Add Policy
            </button>
          </div>
        </form>
      )}

      {policies.length === 0 && !showAddForm && (
        <p className="text-xs text-gray-400 dark:text-gray-500">No encryption policies configured.</p>
      )}

      {policies.map((policy) => (
        <div
          key={policy.id}
          className="flex items-center justify-between px-2 py-1.5 rounded border border-gray-200 dark:border-gray-700"
        >
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <span className="text-sm font-medium truncate">{policy.name}</span>
              <span className={`text-xs px-1.5 py-0.5 rounded ${
                policy.require_encryption
                  ? "bg-red-100 dark:bg-red-900/30 text-red-600 dark:text-red-400"
                  : "bg-gray-100 dark:bg-gray-800 text-gray-500 dark:text-gray-400"
              }`}>
                {policy.require_encryption ? "Required" : "Optional"}
              </span>
            </div>
            <div className="text-xs text-gray-500 dark:text-gray-400 truncate">
              {policy.connector_type} | {policy.file_pattern} | {policy.destination_pattern}
            </div>
          </div>
          <button
            onClick={() => handleRemove(policy.id)}
            className="text-xs px-2 py-1 rounded hover:bg-red-100 dark:hover:bg-red-900/30 text-red-600 dark:text-red-400 ml-2"
            type="button"
            title="Remove policy"
          >
            Del
          </button>
        </div>
      ))}
    </div>
  );
}

// ── Change Password Prompt ──

function ChangePasswordPrompt({
  vaultId,
  vaultName,
  onDone,
  onCancel,
}: {
  vaultId: string;
  vaultName: string;
  onDone: () => void;
  onCancel: () => void;
}) {
  const [oldPassword, setOldPassword] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [error, setError] = useState("");
  const [changing, setChanging] = useState(false);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError("");
    if (newPassword.length < 8) {
      setError("New password must be at least 8 characters.");
      return;
    }
    if (newPassword !== confirmPassword) {
      setError("Passwords do not match.");
      return;
    }
    setChanging(true);
    try {
      await tauriInvoke("change_vault_password", {
        vaultId,
        oldPassword,
        newPassword,
      });
      onDone();
    } catch (err) {
      setError(String(err));
    } finally {
      setChanging(false);
    }
  };

  return (
    <form onSubmit={handleSubmit} className="p-3 space-y-2 border-b border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-800/50">
      <p className="text-xs text-gray-500 dark:text-gray-400">
        Change password for <strong>{vaultName}</strong>
      </p>
      <input
        type="password"
        value={oldPassword}
        onChange={(e) => setOldPassword(e.target.value)}
        placeholder="Current password"
        required
        className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
      />
      <div className="grid grid-cols-2 gap-2">
        <input
          type="password"
          value={newPassword}
          onChange={(e) => setNewPassword(e.target.value)}
          placeholder="New password (min 8)"
          required
          minLength={8}
          className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
        />
        <input
          type="password"
          value={confirmPassword}
          onChange={(e) => setConfirmPassword(e.target.value)}
          placeholder="Confirm new password"
          required
          minLength={8}
          className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
        />
      </div>
      {error && <p className="text-xs text-red-500">{error}</p>}
      <div className="flex justify-end gap-2">
        <button
          type="button"
          onClick={onCancel}
          className="text-xs px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 hover:bg-gray-100 dark:hover:bg-gray-800"
        >
          Cancel
        </button>
        <button
          type="submit"
          disabled={changing}
          className="text-xs px-3 py-1.5 rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-50"
        >
          {changing ? "Changing..." : "Change Password"}
        </button>
      </div>
    </form>
  );
}

// ── Main Vault Panel ──

export function VaultPanel() {
  const [vaults, setVaults] = useState<VaultInfo[]>([]);
  const [unlockedCount, setUnlockedCount] = useState(0);
  const [showCreateForm, setShowCreateForm] = useState(false);
  const [togglingIds, setTogglingIds] = useState<Set<string>>(new Set());
  const [unlockTarget, setUnlockTarget] = useState<VaultInfo | null>(null);
  const [selectedVault, setSelectedVault] = useState<VaultInfo | null>(null);
  const [changingPasswordVault, setChangingPasswordVault] = useState<VaultInfo | null>(null);
  const [activeSection, setActiveSection] = useState<"files" | "transport" | "policies" | "cloud">("files");

  const refresh = useCallback(async () => {
    try {
      const [vaultList, count] = await Promise.all([
        tauriInvoke<VaultInfo[]>("list_vaults", { scanPaths: [] }, []),
        tauriInvoke<number>("get_unlocked_vault_count", undefined, 0),
      ]);
      setVaults(vaultList);
      setUnlockedCount(count);
    } catch (err) {
      console.error("Failed to load vaults:", err);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const handleToggleLock = useCallback(
    async (vault: VaultInfo) => {
      if (vault.is_unlocked) {
        // Lock the vault
        setTogglingIds((prev) => new Set([...prev, vault.id]));
        try {
          await tauriInvoke("lock_vault", { vaultId: vault.id });
          refresh();
        } catch (err) {
          console.error("Failed to lock vault:", err);
        } finally {
          setTogglingIds((prev) => {
            const next = new Set(prev);
            next.delete(vault.id);
            return next;
          });
        }
      } else {
        // Show unlock prompt
        setUnlockTarget(vault);
      }
    },
    [refresh]
  );

  const handleUnlock = useCallback(
    async (password: string) => {
      if (!unlockTarget) return;
      const vault = unlockTarget;
      setUnlockTarget(null);
      setTogglingIds((prev) => new Set([...prev, vault.id]));
      try {
        await tauriInvoke<VaultInfo>("unlock_vault", {
          path: vault.path,
          password,
        });
        refresh();
      } catch (err) {
        console.error("Failed to unlock vault:", err);
      } finally {
        setTogglingIds((prev) => {
          const next = new Set(prev);
          next.delete(vault.id);
          return next;
        });
      }
    },
    [unlockTarget, refresh]
  );

  const handleSelect = useCallback((vault: VaultInfo) => {
    setSelectedVault((prev) => (prev?.id === vault.id ? null : vault));
  }, []);

  return (
    <div
      className="flex flex-col h-full"
      role="region"
      aria-label="Vault manager"
    >
      {/* Header */}
      <div className="flex items-center justify-between px-3 py-2 border-b border-gray-200 dark:border-gray-700">
        <div className="flex items-center gap-2">
          <h2 className="text-sm font-semibold">Vault Manager</h2>
          <span className="text-xs px-1.5 py-0.5 rounded-full bg-gray-100 dark:bg-gray-800 text-gray-500 dark:text-gray-400">
            {unlockedCount} unlocked
          </span>
        </div>
        <button
          onClick={() => {
            setShowCreateForm(!showCreateForm);
          }}
          className="text-xs px-2 py-1 rounded bg-blue-500 text-white hover:bg-blue-600"
          type="button"
        >
          {showCreateForm ? "Cancel" : "+ New Vault"}
        </button>
      </div>

      {/* Create Vault Form */}
      {showCreateForm && (
        <div className="border-b border-gray-200 dark:border-gray-700">
          <CreateVaultForm
            onCreated={() => {
              setShowCreateForm(false);
              refresh();
            }}
            onCancel={() => setShowCreateForm(false)}
          />
        </div>
      )}

      {/* Unlock Prompt */}
      {unlockTarget && (
        <UnlockPrompt
          vaultName={unlockTarget.name}
          onUnlock={handleUnlock}
          onCancel={() => setUnlockTarget(null)}
        />
      )}

      {/* Change Password Prompt */}
      {changingPasswordVault && (
        <ChangePasswordPrompt
          vaultId={changingPasswordVault.id}
          vaultName={changingPasswordVault.name}
          onDone={() => {
            setChangingPasswordVault(null);
          }}
          onCancel={() => setChangingPasswordVault(null)}
        />
      )}

      {/* Vault List */}
      <div className="flex-1 overflow-y-auto">
        {vaults.length === 0 && !showCreateForm && (
          <div className="flex flex-col items-center justify-center h-32 text-sm text-gray-400 dark:text-gray-500">
            <p>No vaults found</p>
            <button
              onClick={() => setShowCreateForm(true)}
              className="mt-2 text-blue-500 hover:text-blue-600"
              type="button"
            >
              Create your first vault
            </button>
          </div>
        )}

        {vaults.map((vault) => (
          <div key={vault.id}>
            <VaultItem
              vault={vault}
              onToggleLock={handleToggleLock}
              onSelect={handleSelect}
              toggling={togglingIds.has(vault.id)}
              selected={selectedVault?.id === vault.id}
            />
            {/* Expanded vault actions */}
            {selectedVault?.id === vault.id && (
              <div className="px-3 py-2 border-b border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-800/30">
                <div className="flex items-center gap-2 mb-2">
                  <span className="text-xs text-gray-500 dark:text-gray-400">
                    Created: {new Date(vault.created_at).toLocaleDateString()}
                  </span>
                  <button
                    onClick={() => setChangingPasswordVault(vault)}
                    className="text-xs px-2 py-1 rounded hover:bg-gray-200 dark:hover:bg-gray-700"
                    type="button"
                  >
                    Change Password
                  </button>
                </div>
              </div>
            )}
          </div>
        ))}
      </div>

      {/* Bottom sections (shown when a vault is selected and unlocked) */}
      {selectedVault && selectedVault.is_unlocked && (
        <div className="border-t border-gray-200 dark:border-gray-700">
          {/* Section Tabs */}
          <div className="flex border-b border-gray-200 dark:border-gray-700">
            <button
              onClick={() => setActiveSection("files")}
              className={`flex-1 text-xs py-2 ${
                activeSection === "files"
                  ? "border-b-2 border-blue-500 text-blue-500 font-medium"
                  : "text-gray-500 dark:text-gray-400 hover:bg-gray-50 dark:hover:bg-gray-800/50"
              }`}
              type="button"
            >
              Files
            </button>
            <button
              onClick={() => setActiveSection("cloud")}
              className={`flex-1 text-xs py-2 ${
                activeSection === "cloud"
                  ? "border-b-2 border-blue-500 text-blue-500 font-medium"
                  : "text-gray-500 dark:text-gray-400 hover:bg-gray-50 dark:hover:bg-gray-800/50"
              }`}
              type="button"
            >
              Cloud
            </button>
            <button
              onClick={() => setActiveSection("transport")}
              className={`flex-1 text-xs py-2 ${
                activeSection === "transport"
                  ? "border-b-2 border-blue-500 text-blue-500 font-medium"
                  : "text-gray-500 dark:text-gray-400 hover:bg-gray-50 dark:hover:bg-gray-800/50"
              }`}
              type="button"
            >
              Transport
            </button>
            <button
              onClick={() => setActiveSection("policies")}
              className={`flex-1 text-xs py-2 ${
                activeSection === "policies"
                  ? "border-b-2 border-blue-500 text-blue-500 font-medium"
                  : "text-gray-500 dark:text-gray-400 hover:bg-gray-50 dark:hover:bg-gray-800/50"
              }`}
              type="button"
            >
              Policies
            </button>
          </div>

          {/* Active Section Content */}
          {activeSection === "files" && (
            <FileOperations vaultId={selectedVault.id} />
          )}
          {activeSection === "cloud" && (
            <div>
              <CloudEncrypt vaultId={selectedVault.id} />
              <CloudDecrypt vaultId={selectedVault.id} />
            </div>
          )}
          {activeSection === "transport" && <TransportSecurity />}
          {activeSection === "policies" && <EncryptionPolicies />}
        </div>
      )}

      {/* Transport & Policies always accessible when no unlocked vault selected */}
      {(!selectedVault || !selectedVault.is_unlocked) && (
        <div className="border-t border-gray-200 dark:border-gray-700">
          <div className="flex border-b border-gray-200 dark:border-gray-700">
            <button
              onClick={() => setActiveSection("transport")}
              className={`flex-1 text-xs py-2 ${
                activeSection === "transport"
                  ? "border-b-2 border-blue-500 text-blue-500 font-medium"
                  : "text-gray-500 dark:text-gray-400 hover:bg-gray-50 dark:hover:bg-gray-800/50"
              }`}
              type="button"
            >
              Transport
            </button>
            <button
              onClick={() => setActiveSection("policies")}
              className={`flex-1 text-xs py-2 ${
                activeSection === "policies"
                  ? "border-b-2 border-blue-500 text-blue-500 font-medium"
                  : "text-gray-500 dark:text-gray-400 hover:bg-gray-50 dark:hover:bg-gray-800/50"
              }`}
              type="button"
            >
              Policies
            </button>
          </div>
          {activeSection === "transport" && <TransportSecurity />}
          {activeSection === "policies" && <EncryptionPolicies />}
        </div>
      )}
    </div>
  );
}

export default VaultPanel;
