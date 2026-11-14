/**
 * Portable mode state (K131): Rust `paths::PortableStatus` from
 * `get_portable_status` (null without a backend).
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

