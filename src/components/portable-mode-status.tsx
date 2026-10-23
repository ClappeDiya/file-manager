/**
 * Portable mode (K131): where this installation keeps its data. The badge
 * shows in the Settings header when portable mode is on; the details list
 * the folders and where credentials are stored. Mirrors Rust
 * `paths::PortableStatus` (`get_portable_status`).
 */
import { useEffect, useState } from "react";
import { tauriInvokeSafe } from "@/hooks/use-tauri";

export interface PortableStatus {
  enabled: boolean;
  root: string | null;
  source: "env" | "config_file" | "marker" | null;
  data_dir: string;
  config_dir: string;
  cache_dir: string;
  /** `keychain`, `vault_file` (portable) or `memory` (this session only). */
  credential_store: "keychain" | "vault_file" | "memory" | string;
}

const SOURCE_LABEL: Record<string, string> = {
  env: "UFOP_PORTABLE_DIR",
  config_file: "ufop-portable.json next to the app",
  marker: "the “portable” file next to the app",
};

const CREDENTIALS_LABEL: Record<string, string> = {
  keychain: "the system keychain",
  vault_file: "the encrypted vault file in the portable folder",
  memory: "memory only (until the app quits)",
};

export function usePortableStatus(): PortableStatus | null {
  const [status, setStatus] = useState<PortableStatus | null>(null);
  useEffect(() => {
    let live = true;
    void tauriInvokeSafe<PortableStatus | null>("get_portable_status", undefined, null).then((s) => {
      if (live) setStatus(s);
    });
    return () => {
      live = false;
    };
  }, []);
  return status;
}

/** "Portable mode: on (path)" — rendered only when portable mode is on. */
export function PortableModeBadge({ status }: { status: PortableStatus | null }) {
  if (!status?.enabled) return null;
  return (
    <span
      data-testid="portable-mode-badge"
      title={status.source ? `Turned on by ${SOURCE_LABEL[status.source] ?? status.source}` : undefined}
      className="text-[11px] px-2 py-0.5 rounded bg-amber-100 text-amber-800 dark:bg-amber-900/30 dark:text-amber-300 truncate max-w-[60%]"
    >
      Portable mode: on ({status.root})
    </span>
  );
}

export function PortableModeDetails({ status }: { status: PortableStatus | null }) {
  if (!status) return <p className="pt-3 text-xs text-gray-500">Folder information is available in the desktop app.</p>;
  const rows: [string, string][] = [
    ["Data", status.data_dir],
    ["Settings", status.config_dir],
    ["Cache", status.cache_dir],
  ];
  return (
    <div className="pt-3 space-y-2 text-xs" data-testid="portable-mode-details">
      <p>
        {status.enabled ? (
          <>
            <span className="font-medium">Portable mode: on</span> ({status.root}), turned on by {status.source ? (SOURCE_LABEL[status.source] ?? status.source) : "configuration"}. Everything
            this app stores, including logs, caches and temporary files, stays in that folder.
          </>
        ) : (
          <>
            <span className="font-medium">Portable mode: off.</span> To run from a removable drive, put an empty file named <code>portable</code> next to the
            app (or set <code>UFOP_PORTABLE_DIR</code>) and restart it.
          </>
        )}
      </p>
      <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5">
        {rows.map(([k, v]) => (
          <div key={k} className="contents">
            <dt className="text-gray-500">{k}</dt>
            <dd className="break-all font-mono text-[11px]">{v}</dd>
          </div>
        ))}
        <dt className="text-gray-500">Credentials</dt>
        <dd data-testid="portable-credentials">{CREDENTIALS_LABEL[status.credential_store] ?? status.credential_store}</dd>
      </dl>
      {status.enabled && status.credential_store === "vault_file" && (
        <p className="text-gray-500">
          Set <code>UFOP_PORTABLE_PASSPHRASE</code> before the first start to protect the vault with a passphrase; otherwise its key is stored next to
          it and the drive itself should be encrypted.
        </p>
      )}
    </div>
  );
}
