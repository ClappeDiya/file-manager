/**
 * Chat-bot remote control (Settings → Advanced). Off by default. When on, a
 * Telegram or Slack bot answers only the people on its allow-list, with a
 * fixed set of commands (/jobs, /run <job>, /stop <job>, /status, /help)
 * that act on the app's own backup, sync and automation jobs. Every accepted
 * command is written to the audit log. Bot tokens go to the credential store
 * and are never shown again.
 * Mirrors Rust `chat_control::{ChatControlSettings, service::ChatControlStatus}`
 * (`chat_control_get`, `chat_control_save`, `chat_control_status`).
 */
import { useEffect, useState } from "react";
import { errorText } from "@/lib/error-text";
import { isTauriAvailable, tauriInvoke } from "@/hooks/use-tauri";

export interface ChatChannelSettings {
  enabled: boolean;
  allowed_user_ids: string[];
  api_base: string;
}

export interface ChatControlSettings {
  enabled: boolean;
  telegram: ChatChannelSettings;
  slack: ChatChannelSettings;
  rate_limit_per_min: number;
}

export interface ChatChannelState {
  connected: boolean;
  handled: number;
  last_error: string | null;
}

export interface ChatControlStatus {
  running: boolean;
  telegram: ChatChannelState;
  slack: ChatChannelState;
  has_telegram_token: boolean;
  has_slack_bot_token: boolean;
  has_slack_app_token: boolean;
}

export interface ChatControlView {
  settings: ChatControlSettings;
  status: ChatControlStatus;
}

/** Ids as typed: separated by commas, spaces or new lines. */
export function parseIds(text: string): string[] {
  return [...new Set(text.split(/[\s,;]+/).map((s) => s.trim()).filter(Boolean))];
}

const field = "w-full rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900 px-2 py-1 text-sm";

function ChannelLine({ name, enabled, state }: { name: string; enabled: boolean; state: ChatChannelState }) {
  if (!enabled) return null;
  return (
    <p className="text-xs" data-testid={`chat-status-${name.toLowerCase()}`}>
      <span className="font-medium">{name}:</span>{" "}
      {state.last_error ? (
        <span className="text-red-600">{state.last_error}</span>
      ) : state.connected ? (
        <span className="text-green-600 dark:text-green-400">connected · {state.handled} command{state.handled === 1 ? "" : "s"} handled</span>
      ) : (
        <span className="text-gray-500">connecting…</span>
      )}
    </p>
  );
}

type Secrets = { telegram: string; slackBot: string; slackApp: string };
const NO_SECRETS: Secrets = { telegram: "", slackBot: "", slackApp: "" };

export function ChatControlSection() {
  const [view, setView] = useState<ChatControlView | null>(null);
  const [settings, setSettings] = useState<ChatControlSettings | null>(null);
  const [tgIds, setTgIds] = useState("");
  const [slIds, setSlIds] = useState("");
  const [secrets, setSecrets] = useState<Secrets>(NO_SECRETS);
  const [remove, setRemove] = useState<Record<keyof Secrets, boolean>>({ telegram: false, slackBot: false, slackApp: false });
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const [busy, setBusy] = useState(false);

  const show = (v: ChatControlView) => {
    setView(v);
    setSettings(v.settings);
    setTgIds(v.settings.telegram.allowed_user_ids.join(", "));
    setSlIds(v.settings.slack.allowed_user_ids.join(", "));
  };

  useEffect(() => {
    if (!isTauriAvailable()) return;
    tauriInvoke<ChatControlView>("chat_control_get")
      .then(show)
      .catch((e) => setError(errorText(e)));
  }, []);

  // Live status while the section is open.
  useEffect(() => {
    if (!isTauriAvailable() || !view?.status.running) return;
    const t = setInterval(() => {
      tauriInvoke<ChatControlStatus>("chat_control_status")
        .then((status) => setView((v) => (v ? { ...v, status } : v)))
        .catch(() => {});
    }, 5000);
    return () => clearInterval(t);
  }, [view?.status.running]);

  if (!isTauriAvailable()) {
    return <p className="pt-3 text-xs text-gray-500">Remote control is available in the desktop app.</p>;
  }
  if (!settings || !view) return error ? <p className="pt-3 text-xs text-red-500">{error}</p> : null;

  const patch = (p: Partial<ChatControlSettings>) => {
    setSaved(false);
    setSettings({ ...settings, ...p });
  };
  const channel = (k: "telegram" | "slack", p: Partial<ChatChannelSettings>) => patch({ [k]: { ...settings[k], ...p } } as Partial<ChatControlSettings>);
  const tokenArg = (k: keyof Secrets): string | null => (remove[k] ? "" : secrets[k].trim() ? secrets[k].trim() : null);

  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      const next: ChatControlSettings = {
        ...settings,
        telegram: { ...settings.telegram, allowed_user_ids: parseIds(tgIds) },
        slack: { ...settings.slack, allowed_user_ids: parseIds(slIds) },
      };
      show(
        await tauriInvoke<ChatControlView>("chat_control_save", {
          settings: next,
          telegramToken: tokenArg("telegram"),
          slackBotToken: tokenArg("slackBot"),
          slackAppToken: tokenArg("slackApp"),
        }),
      );
      setSecrets(NO_SECRETS);
      setRemove({ telegram: false, slackBot: false, slackApp: false });
      setSaved(true);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const secretInput = (k: keyof Secrets, label: string, stored: boolean, placeholder: string) => (
    <div>
      <label className="block text-xs font-medium mb-1" htmlFor={`chat-secret-${k}`}>
        {label}
      </label>
      <div className="flex gap-2">
        <input
          id={`chat-secret-${k}`}
          type="password"
          autoComplete="off"
          className={field}
          value={secrets[k]}
          disabled={remove[k]}
          placeholder={stored ? "A token is stored; type to replace it" : placeholder}
          onChange={(e) => {
            setSaved(false);
            setSecrets({ ...secrets, [k]: e.target.value });
          }}
        />
        {stored && (
          <label className="flex shrink-0 items-center gap-1 text-xs">
            <input type="checkbox" checked={remove[k]} onChange={(e) => setRemove({ ...remove, [k]: e.target.checked })} /> Remove
          </label>
        )}
      </div>
    </div>
  );

  return (
    <div className="space-y-3 pt-3" data-testid="chat-control">
      <p className="text-xs text-gray-500 dark:text-gray-400">
        Start, stop and list your backup, sync and automation jobs from a chat. Only the people you list below can use the bot; everyone else is ignored without a reply. It understands only
        <code> /jobs</code>, <code>/run &lt;job&gt;</code>, <code>/stop &lt;job&gt;</code>, <code>/status</code> and <code>/help</code>, and every command it accepts is written to the audit log.
      </p>
      <label className="flex items-center gap-2 text-sm">
        <input type="checkbox" checked={settings.enabled} onChange={(e) => patch({ enabled: e.target.checked })} />
        Allow remote control from chat (off by default)
      </label>

      {settings.enabled && (
        <>
          <fieldset className="space-y-2 rounded border border-gray-200 dark:border-gray-700 p-3">
            <legend className="px-1 text-xs font-semibold">Telegram</legend>
            <label className="flex items-center gap-2 text-sm">
              <input type="checkbox" checked={settings.telegram.enabled} onChange={(e) => channel("telegram", { enabled: e.target.checked })} />
              Use a Telegram bot
            </label>
            {settings.telegram.enabled && (
              <>
                {secretInput("telegram", "Bot token (from @BotFather)", view.status.has_telegram_token, "123456:ABC…")}
                <div>
                  <label className="block text-xs font-medium mb-1" htmlFor="chat-tg-ids">Allowed Telegram user ids</label>
                  <input id="chat-tg-ids" className={`${field} font-mono`} value={tgIds} placeholder="123456789, 987654321" onChange={(e) => { setSaved(false); setTgIds(e.target.value); }} />
                  <p className="mt-1 text-[11px] text-gray-500">Numeric ids (not @names). Message @userinfobot to find yours.</p>
                </div>
              </>
            )}
          </fieldset>

          <fieldset className="space-y-2 rounded border border-gray-200 dark:border-gray-700 p-3">
            <legend className="px-1 text-xs font-semibold">Slack</legend>
            <label className="flex items-center gap-2 text-sm">
              <input type="checkbox" checked={settings.slack.enabled} onChange={(e) => channel("slack", { enabled: e.target.checked })} />
              Use a Slack app (Socket Mode)
            </label>
            {settings.slack.enabled && (
              <>
                {secretInput("slackBot", "Bot token (xoxb-…)", view.status.has_slack_bot_token, "xoxb-…")}
                {secretInput("slackApp", "App-level token (xapp-…, with connections:write)", view.status.has_slack_app_token, "xapp-…")}
                <div>
                  <label className="block text-xs font-medium mb-1" htmlFor="chat-sl-ids">Allowed Slack member ids</label>
                  <input id="chat-sl-ids" className={`${field} font-mono`} value={slIds} placeholder="U012ABCDEF" onChange={(e) => { setSaved(false); setSlIds(e.target.value); }} />
                  <p className="mt-1 text-[11px] text-gray-500">Member ids start with U (profile → “Copy member ID”).</p>
                </div>
              </>
            )}
          </fieldset>

          <div className="w-56">
            <label className="block text-xs font-medium mb-1" htmlFor="chat-rate">Commands per person per minute</label>
            <input id="chat-rate" type="number" min={1} max={60} className={field} value={settings.rate_limit_per_min} onChange={(e) => patch({ rate_limit_per_min: Number(e.target.value) })} />
          </div>
        </>
      )}

      <ChannelLine name="Telegram" enabled={settings.enabled && settings.telegram.enabled && view.status.running} state={view.status.telegram} />
      <ChannelLine name="Slack" enabled={settings.enabled && settings.slack.enabled && view.status.running} state={view.status.slack} />

      <div className="flex items-center gap-3">
        <button type="button" onClick={save} disabled={busy} className="text-sm px-4 py-1.5 rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-50">
          {busy ? "Saving…" : "Save"}
        </button>
        {saved && <span className="text-xs text-green-600" role="status">Saved</span>}
      </div>
      {error && <p className="text-xs text-red-500" role="alert">{error}</p>}
    </div>
  );
}
