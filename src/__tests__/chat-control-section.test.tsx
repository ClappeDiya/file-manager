/** Chat-bot remote control settings: off by default, allow-lists, token handling, errors, Advanced gate. */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { ChatControlSection, parseIds, type ChatControlSettings, type ChatControlView } from "@/components/chat-control-section";
import { AdvancedOnly } from "@/components/backup/advanced-only";
import { useUIStore } from "@/stores/ui-store";

const idle = { connected: false, handled: 0, last_error: null };
const OFF: ChatControlView = {
  settings: { enabled: false, telegram: { enabled: false, allowed_user_ids: [], api_base: "" }, slack: { enabled: false, allowed_user_ids: [], api_base: "" }, rate_limit_per_min: 10 },
  status: { running: false, telegram: idle, slack: idle, has_telegram_token: false, has_slack_bot_token: false, has_slack_app_token: false },
};
let current: ChatControlView = OFF;
type SaveArgs = { settings: ChatControlSettings; telegramToken: string | null; slackBotToken: string | null; slackAppToken: string | null };

beforeEach(() => {
  current = structuredClone(OFF);
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  mockInvoke.mockReset().mockImplementation(async (cmd: string, a?: SaveArgs) => {
    if (cmd === "chat_control_get") return current;
    if (cmd === "chat_control_status") return current.status;
    if (cmd === "chat_control_save") {
      if (a!.settings.telegram.enabled && a!.settings.telegram.allowed_user_ids.length === 0) throw { message: "Add at least one Telegram user id that may command the bot", advice: "" };
      current = {
        settings: a!.settings,
        status: {
          ...current.status,
          running: a!.settings.enabled,
          has_telegram_token: a!.telegramToken === null ? current.status.has_telegram_token : a!.telegramToken !== "",
          telegram: a!.settings.enabled ? { connected: true, handled: 3, last_error: null } : idle,
        },
      };
      return current;
    }
    return null;
  });
});

describe("parseIds", () => {
  it("splits on commas, spaces and new lines and drops blanks and repeats", () => {
    expect(parseIds("123, 456\n789  123;")).toEqual(["123", "456", "789"]);
    expect(parseIds("  ")).toEqual([]);
  });
});

describe("ChatControlSection", () => {
  it("is off by default and shows nothing to configure until switched on", async () => {
    render(<ChatControlSection />);
    const master = (await screen.findByLabelText(/Allow remote control from chat/)) as HTMLInputElement;
    expect(master.checked).toBe(false);
    expect(screen.queryByLabelText(/Allowed Telegram user ids/)).toBeNull();
    expect(screen.getByText(/ignored without a reply/)).toBeTruthy();
  });

  it("saves a Telegram bot with its token and allow-list, then reports its status", async () => {
    render(<ChatControlSection />);
    fireEvent.click(await screen.findByLabelText(/Allow remote control from chat/));
    fireEvent.click(screen.getByLabelText("Use a Telegram bot"));
    fireEvent.change(screen.getByLabelText(/Bot token \(from @BotFather\)/), { target: { value: " 123456:ABCDEFGHIJ " } });
    fireEvent.change(screen.getByLabelText("Allowed Telegram user ids"), { target: { value: "111, 222" } });
    fireEvent.click(screen.getByText("Save"));
    await screen.findByText("Saved");
    expect(mockInvoke).toHaveBeenCalledWith("chat_control_save", {
      settings: { enabled: true, telegram: { enabled: true, allowed_user_ids: ["111", "222"], api_base: "" }, slack: OFF.settings.slack, rate_limit_per_min: 10 },
      telegramToken: "123456:ABCDEFGHIJ",
      slackBotToken: null,
      slackAppToken: null,
    });
    expect((await screen.findByTestId("chat-status-telegram")).textContent).toContain("connected · 3 commands handled");
    // The token is never shown again; the box now says one is stored.
    const box = screen.getByLabelText(/Bot token \(from @BotFather\)/) as HTMLInputElement;
    expect(box.value).toBe("");
    expect(box.placeholder).toMatch(/A token is stored/);
  });

  it("keeps a stored token unless asked to remove it", async () => {
    current.settings = { ...current.settings, enabled: true, telegram: { enabled: true, allowed_user_ids: ["111"], api_base: "" } };
    current.status.has_telegram_token = true;
    render(<ChatControlSection />);
    await screen.findByLabelText("Allowed Telegram user ids");
    fireEvent.click(screen.getByText("Save"));
    await screen.findByText("Saved");
    expect((mockInvoke.mock.calls.find(([c]) => c === "chat_control_save")![1] as SaveArgs).telegramToken).toBeNull();
    mockInvoke.mockClear();
    fireEvent.click(screen.getByLabelText("Remove"));
    fireEvent.click(screen.getByText("Save"));
    await waitFor(() => expect((mockInvoke.mock.calls.find(([c]) => c === "chat_control_save")![1] as SaveArgs).telegramToken).toBe(""));
  });

  it("shows the backend's refusal, such as an empty allow-list", async () => {
    render(<ChatControlSection />);
    fireEvent.click(await screen.findByLabelText(/Allow remote control from chat/));
    fireEvent.click(screen.getByLabelText("Use a Telegram bot"));
    fireEvent.click(screen.getByText("Save"));
    expect((await screen.findByRole("alert")).textContent).toContain("Add at least one Telegram user id");
    expect(screen.queryByText("Saved")).toBeNull();
  });

  it("configures Slack with both tokens", async () => {
    render(<ChatControlSection />);
    fireEvent.click(await screen.findByLabelText(/Allow remote control from chat/));
    fireEvent.click(screen.getByLabelText(/Use a Slack app/));
    fireEvent.change(screen.getByLabelText(/Bot token \(xoxb/), { target: { value: "xoxb-1" } });
    fireEvent.change(screen.getByLabelText(/App-level token/), { target: { value: "xapp-1" } });
    fireEvent.change(screen.getByLabelText("Allowed Slack member ids"), { target: { value: "U012ABCDEF" } });
    fireEvent.click(screen.getByText("Save"));
    await screen.findByText("Saved");
    const args = mockInvoke.mock.calls.find(([c]) => c === "chat_control_save")![1] as SaveArgs;
    expect([args.slackBotToken, args.slackAppToken, args.settings.slack.allowed_user_ids]).toEqual(["xoxb-1", "xapp-1", ["U012ABCDEF"]]);
  });

  it("is only offered in Advanced mode", () => {
    useUIStore.setState({ appMode: "simple", backupAdvanced: null });
    const { unmount } = render(<AdvancedOnly><ChatControlSection /></AdvancedOnly>);
    expect(screen.queryByLabelText(/Allow remote control from chat/)).toBeNull();
    unmount();
    useUIStore.setState({ appMode: "advanced" });
    render(<AdvancedOnly><ChatControlSection /></AdvancedOnly>);
    return screen.findByLabelText(/Allow remote control from chat/);
  });
});
