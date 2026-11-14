/**
 * Notification profiles, previews and webhook presets (Kopia-parity WP9:
 * K101/K104/K105). Wire types mirror `src-tauri/src/backup_engine/notify/`.
 *
 * A profile is a named channel kept once and referenced by jobs (and policy
 * templates) through `notification_profile_ids`; a job sends to its own
 * channels plus its profiles, the same channel only once. Secrets (password,
 * token, webhook URL, secret header values) live in the secure store under the
 * channel's credential ids: `secret` on a save is stored and never returned.
 */
import { create } from "zustand";
import { isBackendAvailable, tauriInvoke } from "@/hooks/use-tauri";
import type { BackupNotification, WebhookPreset } from "./backup-store";

export type Severity = "success" | "warning" | "error";

export interface NotificationProfile {
  id: string;
  name: string;
  description: string;
  notification: BackupNotification;
  /** When set, results at least this severe are sent (instead of `send_on`). */
  min_severity: Severity | null;
  /** Delivered by the organization's console (read-only). */
  managed: boolean;
  created_at: string;
  updated_at: string;
  /** Write-only: the channel secret to store with this save. */
  secret?: string | null;
}

export interface ProfileDeleteOutcome {
  removed_from: string[];
  warnings: string[];
}

export interface NotificationPreview {
  subject: string;
  text: string;
  html: string | null;
  /** Webhooks: request line, headers (secrets masked) and body. */
  request: string | null;
}

export interface PresetParams {
  server: string;
  topic: string;
  user: string;
  bearer: boolean;
}

export const PRESETS: { id: WebhookPreset; label: string; secret: string }[] = [
  { id: "pushover", label: "Pushover", secret: "Application API token" },
  { id: "ntfy", label: "ntfy", secret: "Access token (optional)" },
  { id: "discord", label: "Discord", secret: "Discord webhook URL" },
  { id: "pagerduty", label: "PagerDuty", secret: "Integration (routing) key" },
  { id: "gotify", label: "Gotify", secret: "Application token" },
  { id: "generic", label: "Webhook", secret: "Secret ({{secret}} in the URL, headers or body)" },
];

/** RFC 9110 header-name characters (the engine checks the same). */
export const validHeaderName = (s: string) => /^[A-Za-z0-9!#$%&'*+.^_`|~-]+$/.test(s);

/** Results a profile sends: `min_severity` wins over the channel's `send_on`. */
export function profileSendsOn(p: Pick<NotificationProfile, "min_severity" | "notification">): string[] {
  const all: Severity[] = ["success", "warning", "error"];
  return p.min_severity ? all.slice(all.indexOf(p.min_severity)) : p.notification.send_on;
}

export function channelLabel(n: BackupNotification): string {
  if (n.type === "webhook") return PRESETS.find((p) => p.id === n.preset)?.label ?? "Webhook";
  if (n.type === "xmpp") return "XMPP";
  return n.type.charAt(0).toUpperCase() + n.type.slice(1);
}

export function newProfile(notification: BackupNotification): NotificationProfile {
  return { id: "", name: "", description: "", notification, min_severity: null, managed: false, created_at: "", updated_at: "" };
}

/** A webhook channel for a preset, built by the engine (`backup_webhook_preset`). */
export function webhookPreset(preset: WebhookPreset, params: Partial<PresetParams> = {}): Promise<BackupNotification> {
  return tauriInvoke<BackupNotification>("backup_webhook_preset", { preset, params: { server: "", topic: "", user: "", bearer: false, ...params } });
}

export function previewNotification(notification: BackupNotification, result: Severity = "success"): Promise<NotificationPreview> {
  return tauriInvoke<NotificationPreview>("backup_preview_notification", { notification, result });
}

interface NotifyState {
  profiles: NotificationProfile[];
  error: string | null;
  loadProfiles: () => Promise<void>;
  saveProfile: (p: NotificationProfile) => Promise<NotificationProfile>;
  deleteProfile: (id: string, force?: boolean) => Promise<ProfileDeleteOutcome>;
  testProfile: (id: string) => Promise<void>;
}

export const useNotifyStore = create<NotifyState>((set, get) => ({
  profiles: [],
  error: null,
  loadProfiles: async () => {
    if (!isBackendAvailable()) return;
    try {
      const list = await tauriInvoke<NotificationProfile[] | null>("backup_list_notification_profiles", undefined, []);
      set({ profiles: Array.isArray(list) ? list : [], error: null });
    } catch (e) {
      set({ error: String((e as { message?: string })?.message ?? e) });
    }
  },
  saveProfile: async (profile) => {
    const saved = await tauriInvoke<NotificationProfile>("backup_save_notification_profile", { profile });
    await get().loadProfiles();
    return saved;
  },
  deleteProfile: async (id, force = false) => {
    const r = await tauriInvoke<ProfileDeleteOutcome>("backup_delete_notification_profile", { id, force });
    await get().loadProfiles();
    return r;
  },
  testProfile: (id) => tauriInvoke<null>("backup_test_notification_profile", { id }).then(() => undefined),
}));
