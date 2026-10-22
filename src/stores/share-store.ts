/** Share links (types mirror connectors::ShareOptions/ShareLink and share_commands::SharingPolicy). */
import { tauriInvoke } from "@/hooks/use-tauri";

export type ShareScope = "anyone" | "organization";

export interface ShareOptions {
  expires_in_secs: number | null;
  password: string | null;
  scope: ShareScope;
  allow_edit: boolean;
}

export interface ShareLink {
  url: string;
  expires_at: string | null;
  scope: ShareScope;
  provider: string;
  note: string | null;
}

export interface SharingPolicy {
  enabled: boolean;
  allow_anyone: boolean;
  require_expiry: boolean;
  max_expiry_days: number | null;
  require_password: boolean;
  allow_edit: boolean;
}

/** Protocols whose connectors implement `create_share_link`. */
export const SHAREABLE_PROTOCOLS = new Set(["s3", "azure_blob", "onedrive", "dropbox", "google_drive"]);

/** What each provider can put on a link (the dialog hides the rest). */
export const PROVIDER_CAPS: Record<string, { expiry: "required" | "optional" | "none"; password: boolean; organization: boolean; edit: boolean; maxDays?: number }> = {
  s3: { expiry: "required", password: false, organization: false, edit: false, maxDays: 7 },
  azure_blob: { expiry: "required", password: false, organization: false, edit: false },
  onedrive: { expiry: "optional", password: true, organization: true, edit: true },
  dropbox: { expiry: "optional", password: true, organization: true, edit: true },
  google_drive: { expiry: "none", password: false, organization: false, edit: true },
};

export const EXPIRY_PRESETS: { label: string; secs: number | null }[] = [
  { label: "1 hour", secs: 3600 },
  { label: "1 day", secs: 86_400 },
  { label: "7 days", secs: 7 * 86_400 },
  { label: "30 days", secs: 30 * 86_400 },
  { label: "Never", secs: null },
];

/** Presets this provider and policy allow. */
export function allowedExpiries(protocol: string, policy: SharingPolicy | null) {
  const caps = PROVIDER_CAPS[protocol];
  return EXPIRY_PRESETS.filter((p) => {
    if (!caps) return true;
    if (caps.expiry === "none") return p.secs === null && !policy?.require_expiry && !policy?.max_expiry_days;
    if (p.secs === null) return caps.expiry === "optional" && !policy?.require_expiry && !policy?.max_expiry_days;
    if (caps.maxDays && p.secs > caps.maxDays * 86_400) return false;
    return !policy?.max_expiry_days || p.secs <= policy.max_expiry_days * 86_400;
  });
}

export const DEFAULT_SHARING_POLICY: SharingPolicy = { enabled: true, allow_anyone: true, require_expiry: false, max_expiry_days: null, require_password: false, allow_edit: true };

export const sharing = {
  policy: () => tauriInvoke<SharingPolicy>("sharing_policy_get"),
  setPolicy: (policy: SharingPolicy) => tauriInvoke<SharingPolicy>("sharing_policy_set", { policy }),
  create: (protocol: string, connectionId: string | null, path: string, options: ShareOptions) =>
    tauriInvoke<ShareLink>("connector_create_share_link", { protocol, connectionId, path, options }),
};
