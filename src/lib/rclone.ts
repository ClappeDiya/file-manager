/**
 * rclone: binary/config settings, the remote manager and crypt tools.
 * Types mirror `src-tauri/src/rclone_engine/*` and
 * `commands/rclone_commands.rs` (serde output: snake_case, `T | null`).
 */
import { tauriInvoke } from "@/hooks/use-tauri";

// ── Binary and config (Settings → rclone) ──

/** `RcloneSettings` */
export interface RcloneSettings {
  binary_path: string | null;
  config_path: string | null;
}

/** `BinarySource`: where the binary came from. */
export type RcloneBinarySource = "settings" | "env" | "path" | "none";

/** `RcloneStatus` */
export interface RcloneStatus {
  path: string;
  source: RcloneBinarySource;
  found: boolean;
  version: string | null;
  meets_minimum: boolean | null;
  min_version: string;
  config_path: string | null;
  managed: boolean;
  error: string | null;
}

/** `RcloneSettingsView` */
export interface RcloneSettingsView {
  settings: RcloneSettings;
  status: RcloneStatus;
  default_config_path: string | null;
  managed_dir: string;
  pinned_version: string;
  managed_version: string | null;
  update_available: boolean;
  platform_supported: boolean;
}

/** `Candidate`: a binary found by Detect. */
export interface RcloneCandidate {
  path: string;
  origin: "env" | "path" | "managed" | "common" | string;
  version: string | null;
  meets_minimum: boolean | null;
  error: string | null;
}

/** `RcloneInstallOutcome` */
export interface RcloneInstallOutcome {
  result: { path: string; version: string; archive: string; sha256: string };
  view: RcloneSettingsView;
}

export const SOURCE_LABEL: Record<RcloneBinarySource, string> = {
  settings: "chosen in Settings",
  env: "from UFOP_RCLONE",
  path: "found on PATH",
  none: "not found",
};

/** One line for About / Settings: "v1.69.1 (/usr/bin/rclone)" or "not installed". */
export function describeStatus(s: RcloneStatus): string {
  if (!s.version) return s.source === "none" || !s.found ? "not installed" : `not working (${s.error ?? "no version"})`;
  return `${s.version} (${s.path})`;
}

export const rcloneStatus = () => tauriInvoke<RcloneStatus>("rclone_status");
export const getRcloneSettings = () => tauriInvoke<RcloneSettingsView>("rclone_settings_get");
export const setRcloneSettings = (settings: RcloneSettings) => tauriInvoke<RcloneSettingsView>("rclone_settings_set", { settings });
export const detectRclone = () => tauriInvoke<RcloneCandidate[]>("rclone_detect", undefined, []);
export const installManagedRclone = () => tauriInvoke<RcloneInstallOutcome>("rclone_install_managed");

// ── Remote manager ──

/** `OptionExample` */
export interface RcloneOptionExample {
  value: string;
  help: string;
  provider: string;
}

/** `OptionSchema` */
export interface RcloneOption {
  name: string;
  help: string;
  /** "" = all providers; "AWS,Ceph" = those; "!AWS,Ceph" = all but those. */
  provider: string;
  default: string;
  examples: RcloneOptionExample[];
  required: boolean;
  is_password: boolean;
  sensitive: boolean;
  advanced: boolean;
  exclusive: boolean;
  hidden: boolean;
  type: string;
}

/** `ProviderSchema` */
export interface RcloneProvider {
  name: string;
  description: string;
  prefix: string;
  options: RcloneOption[];
  hidden: boolean;
}

/** `ConfigStep` */
export interface RcloneConfigStep {
  state: string;
  option: RcloneOption | null;
  error: string;
  result: string;
  done: boolean;
}

/** `RcloneRemote` (`rclone listremotes --long`) */
export interface RcloneRemote {
  name: string;
  backend: string;
}

/** `RemoteDetails` (secrets withheld) */
export interface RcloneRemoteDetails {
  name: string;
  type: string;
  values: Record<string, string>;
  secrets_set: string[];
}

/** `RemoteTest` */
export interface RcloneRemoteTest {
  ok: boolean;
  entries: number;
  error: string | null;
  millis: number;
}

/** `ObscureMode` */
export type RcloneObscureMode = "auto" | "obscure" | "already_obscured";

/** `AuthUrlEvent` (event `rclone-auth-url`) */
export interface RcloneAuthUrl {
  name: string;
  url: string;
  opened: boolean;
}

export const RCLONE_AUTH_URL_EVENT = "rclone-auth-url";
/** Fired on `window` after a remote is created, changed or deleted. */
export const RCLONE_REMOTES_CHANGED_EVENT = "ufop:rclone-remotes-changed";

export const listRcloneRemotes = () => tauriInvoke<RcloneRemote[]>("rclone_list_remotes", undefined, []);
export const rcloneProviders = () => tauriInvoke<RcloneProvider[]>("rclone_providers", undefined, []);
export const getRcloneRemote = (name: string) => tauriInvoke<RcloneRemoteDetails>("rclone_remote_get", { name });
export const createRcloneRemote = (name: string, remoteType: string, params: Record<string, string>, obscure: RcloneObscureMode) =>
  tauriInvoke<RcloneConfigStep>("rclone_remote_create", { name, remoteType, params, obscure });
export const updateRcloneRemote = (name: string, params: Record<string, string>, obscure: RcloneObscureMode) =>
  tauriInvoke<RcloneConfigStep>("rclone_remote_update", { name, params, obscure });
export const continueRcloneConfig = (name: string, state: string, result: string, params: Record<string, string>) =>
  tauriInvoke<RcloneConfigStep>("rclone_config_continue", { name, state, result, params });
export const signInRcloneRemote = (name: string, state: string, params: Record<string, string>) =>
  tauriInvoke<RcloneConfigStep>("rclone_config_oauth", { name, state, params, openBrowser: true });
export const cancelRcloneSignIn = (name: string) => tauriInvoke<boolean>("rclone_config_cancel", { name }, false);
export const deleteRcloneRemote = (name: string) => tauriInvoke<void>("rclone_remote_delete", { name });
export const testRcloneRemote = (name: string) => tauriInvoke<RcloneRemoteTest>("rclone_remote_test", { name });

export function notifyRemotesChanged() {
  window.dispatchEvent(new Event(RCLONE_REMOTES_CHANGED_EVENT));
}

/** Listen for sign-in addresses; returns an unsubscribe function. */
export async function onRcloneAuthUrl(cb: (e: RcloneAuthUrl) => void): Promise<() => void> {
  try {
    const { listen } = await import("@tauri-apps/api/event");
    return await listen<RcloneAuthUrl>(RCLONE_AUTH_URL_EVENT, (e) => cb(e.payload));
  } catch {
    return () => {};
  }
}

/** rclone's "Use auto config?" question (OAuth backends). */
export function isOAuthQuestion(step: RcloneConfigStep): boolean {
  return step.option?.name === "config_is_local" || step.state.startsWith("*oauth-islocal");
}

/**
 * Curated "Popular" backends for the picker. Each maps to the provider
 * name (or prefix) in `rclone config providers`; entries the installed
 * rclone lacks are shown as unavailable, never with a hand-made form.
 */
export const POPULAR_RCLONE_BACKENDS: { label: string; type: string }[] = [
  { label: "Box", type: "box" },
  { label: "pCloud", type: "pcloud" },
  { label: "MEGA", type: "mega" },
  { label: "Proton Drive", type: "protondrive" },
  { label: "iCloud Drive", type: "iclouddrive" },
  { label: "Google Photos", type: "gphotos" },
  { label: "Yandex Disk", type: "yandex" },
  { label: "HiDrive", type: "hidrive" },
  { label: "Koofr", type: "koofr" },
  { label: "Oracle Object Storage", type: "oracleobjectstorage" },
  { label: "Gofile", type: "gofile" },
  { label: "HTTP", type: "http" },
  { label: "put.io", type: "putio" },
  { label: "Zoho WorkDrive", type: "zoho" },
  { label: "Citrix ShareFile", type: "sharefile" },
];

export function findProvider(providers: RcloneProvider[], type: string): RcloneProvider | null {
  return providers.find((p) => p.name === type || p.prefix === type) ?? null;
}

/** The popular list against the installed rclone's schemas. */
export function popularBackends(providers: RcloneProvider[]): { label: string; type: string; provider: RcloneProvider | null }[] {
  return POPULAR_RCLONE_BACKENDS.map((b) => ({ ...b, provider: findProvider(providers, b.type) }));
}

/** Does a provider filter (`""`, `"AWS,Ceph"`, `"!AWS"`) admit `value`? */
export function providerMatches(filter: string, value: string): boolean {
  const f = filter.trim();
  if (!f) return true;
  const negate = f.startsWith("!");
  const list = (negate ? f.slice(1) : f).split(",").map((s) => s.trim()).filter(Boolean);
  const hit = list.includes(value);
  return negate ? !hit : hit;
}

export const isSecret = (o: RcloneOption) => o.is_password || o.sensitive;

/**
 * Options to show: not hidden from rclone's configurator, matching the
 * chosen `provider` (S3-style backends), and advanced ones only in
 * Advanced mode. One entry per name (the first that applies).
 */
export function visibleOptions(provider: RcloneProvider, values: Record<string, string>, advanced: boolean): RcloneOption[] {
  const providerValue = values.provider ?? provider.options.find((o) => o.name === "provider")?.default ?? "";
  const seen = new Set<string>();
  const out: RcloneOption[] = [];
  for (const o of provider.options) {
    if (o.hidden || seen.has(o.name)) continue;
    if (o.advanced && !advanced) continue;
    if (o.name !== "provider" && !providerMatches(o.provider, providerValue)) continue;
    seen.add(o.name);
    out.push(o);
  }
  return out;
}

/** The value a field starts with: the typed/stored one, else the default (secrets start blank). */
export function fieldValue(o: RcloneOption, values: Record<string, string>): string {
  const v = values[o.name];
  if (v !== undefined) return v;
  return isSecret(o) ? "" : o.default;
}

/** Examples of `o` that apply to the chosen provider. */
export function optionExamples(o: RcloneOption, values: Record<string, string>): RcloneOptionExample[] {
  const pv = values.provider ?? "";
  return o.examples.filter((e) => o.name === "provider" || providerMatches(e.provider, pv));
}

/**
 * The `key=value` parameters to send: on create, the values that differ
 * from the default; on update, those that differ from the stored ones.
 * A blank secret is never sent (blank = keep / unset).
 */
export function changedParams(options: RcloneOption[], values: Record<string, string>, stored?: Record<string, string>): Record<string, string> {
  const out: Record<string, string> = {};
  for (const o of options) {
    const v = values[o.name];
    if (v === undefined) continue;
    if (isSecret(o)) {
      if (v !== "") out[o.name] = v;
      continue;
    }
    const before = stored ? stored[o.name] ?? o.default : o.default;
    if (v !== before) out[o.name] = v;
  }
  return out;
}

/** Required options left empty (a stored secret counts as filled). */
export function missingRequired(options: RcloneOption[], values: Record<string, string>, secretsSet: string[] = []): RcloneOption[] {
  return options.filter((o) => o.required && !(values[o.name] ?? o.default) && !secretsSet.includes(o.name));
}

// ── Crypt ──

/** `CryptInfo` */
export interface RcloneCryptInfo {
  remote: string;
  is_crypt: boolean;
  underlying: string | null;
  underlying_remote: string | null;
  filename_encryption: string | null;
  directory_name_encryption: boolean | null;
}

/** `CryptName` */
export interface RcloneCryptName {
  input: string;
  output: string | null;
  error: string | null;
}

export const rcloneCryptInfo = (remote: string) => tauriInvoke<RcloneCryptInfo>("rclone_crypt_info", { remote });
export const rcloneCryptDecode = (remote: string, names: string[], reverse: boolean) =>
  tauriInvoke<RcloneCryptName[]>("rclone_cryptdecode", { remote, names, reverse });
