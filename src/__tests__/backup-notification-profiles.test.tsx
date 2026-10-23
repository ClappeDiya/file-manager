/**
 * Notifications WP9 (K101/K104/K105): reusable profiles (manager, job
 * picker, forced delete), webhook presets and headers, HTML mail with a
 * sandboxed preview. Shapes sent to Rust match the serde types.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { useUIStore } from "@/stores/ui-store";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { HeadersEditor, NotificationsEditor, newNotification } from "@/components/backup/backup-notifications";
import { NotificationProfilePicker, NotificationProfilesManager } from "@/components/backup/backup-notification-profiles";
import { channelLabel, profileSendsOn, useNotifyStore, validHeaderName, type NotificationProfile } from "@/stores/backup-notify";
import type { BackupNotification, WebhookHeader } from "@/stores/backup-store";
import { SETTING_SECTIONS } from "@/stores/backup-policy";

const PD: BackupNotification = {
  type: "webhook",
  url: "https://events.pagerduty.com/v2/enqueue",
  method: "POST",
  content_type: "application/json",
  headers: [],
  body_template: '{"routing_key":"{{secret}}"}',
  preset: "pagerduty",
  send_on: ["success", "warning", "error"],
  credential_id: "cred-pd",
  template: "",
  cert: { accept_invalid_certs: false, ca_pem: null, pinned_sha256: [] },
};

const profile = (id: string, name: string, extra: Partial<NotificationProfile> = {}): NotificationProfile => ({
  id,
  name,
  description: "",
  notification: PD,
  min_severity: null,
  managed: false,
  created_at: "",
  updated_at: "",
  ...extra,
});

beforeEach(() => {
  useUIStore.setState({ backupAdvanced: true });
  useNotifyStore.setState({ profiles: [], error: null });
  mockInvoke.mockReset();
  mockInvoke.mockResolvedValue(false);
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});

describe("helpers", () => {
  it("severity, labels and header names", () => {
    expect(profileSendsOn(profile("a", "A", { min_severity: "warning" }))).toEqual(["warning", "error"]);
    expect(profileSendsOn(profile("a", "A"))).toEqual(["success", "warning", "error"]);
    expect(channelLabel(PD)).toBe("PagerDuty");
    expect(channelLabel(newNotification("xmpp"))).toBe("XMPP");
    expect(validHeaderName("X-Api-Key")).toBe(true);
    expect(validHeaderName("Bad Name")).toBe(false);
    expect(validHeaderName("X:Y")).toBe(false);
    expect(SETTING_SECTIONS).toContain("notification_profile_ids");
  });

  it("new email channels send HTML; slack and teams carry only a secret", () => {
    expect(newNotification("email")).toMatchObject({ type: "email", html: true, html_template: "" });
    expect(newNotification("slack")).toMatchObject({ type: "slack", send_on: ["warning", "error"] });
  });
});

describe("profiles manager (K101)", () => {
  it("lists, creates from a preset, tests and force-deletes a referenced profile", async () => {
    const list = [profile("p1", "Pager", { min_severity: "error" }), profile("org:x", "Org mail", { managed: true, notification: newNotification("email") })];
    mockInvoke.mockImplementation(async (cmd: string, args: Record<string, unknown>) => {
      if (cmd === "backup_list_notification_profiles") return list;
      if (cmd === "backup_webhook_preset") return { ...PD, url: "https://ntfy.example/backups", preset: "ntfy", headers: [], credential_id: "cred-new" };
      if (cmd === "backup_save_notification_profile") return { ...(args.profile as NotificationProfile), id: "p2" };
      if (cmd === "backup_test_notification_profile") return null;
      if (cmd === "backup_delete_notification_profile") {
        if (!args.force) throw { message: "The notification profile is used by job 'Docs'", advice: "" };
        return { removed_from: ["job 'Docs'"], warnings: ["Job 'Docs' no longer sends to 'Pager'"] };
      }
      return false;
    });
    render(<NotificationProfilesManager />);
    const rows = await screen.findByTestId("profiles-list");
    await waitFor(() => expect(within(rows).getByText("Pager")).toBeInTheDocument());
    expect(within(rows).getByText("error")).toBeInTheDocument();
    expect(within(rows).getByText("Organization")).toBeInTheDocument();

    fireEvent.click(screen.getByTestId("profile-test-p1"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("backup_test_notification_profile", { id: "p1" }));
    expect(await screen.findByText("Test message sent through Pager")).toBeInTheDocument();

    // New profile from the ntfy preset.
    fireEvent.click(screen.getByTestId("profile-new"));
    fireEvent.change(screen.getByTestId("notification-add-preset"), { target: { value: "ntfy" } });
    fireEvent.change(screen.getByTestId("preset-topic"), { target: { value: "backups" } });
    fireEvent.click(screen.getByTestId("preset-add"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("backup_webhook_preset", { preset: "ntfy", params: { server: "", topic: "backups", user: "", bearer: false } }));
    fireEvent.change(await screen.findByTestId("profile-name"), { target: { value: "Phone" } });
    fireEvent.change(screen.getByTestId("profile-severity"), { target: { value: "warning" } });
    fireEvent.click(screen.getByTestId("profile-save"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("backup_save_notification_profile", expect.objectContaining({ profile: expect.objectContaining({ name: "Phone", min_severity: "warning", notification: expect.objectContaining({ type: "webhook", preset: "ntfy" }) }) })));

    // Delete: refused while referenced, then forced.
    fireEvent.click(screen.getByTestId("profile-delete-p1"));
    expect(await screen.findByText(/used by job 'Docs'/)).toBeInTheDocument();
    fireEvent.click(screen.getByTestId("profile-delete-force"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("backup_delete_notification_profile", { id: "p1", force: true }));
    expect(await screen.findByText(/no longer sends to 'Pager'/)).toBeInTheDocument();
  });

  it("the job picker toggles profile ids and flags missing ones", async () => {
    useNotifyStore.setState({ profiles: [profile("p1", "Pager")] });
    mockInvoke.mockImplementation(async (cmd: string) => (cmd === "backup_list_notification_profiles" ? [profile("p1", "Pager")] : false));
    const onChange = vi.fn();
    const { rerender } = render(<NotificationProfilePicker value={[]} onChange={onChange} />);
    fireEvent.click(await screen.findByLabelText("Pager (PagerDuty)"));
    expect(onChange).toHaveBeenLastCalledWith(["p1"]);
    rerender(<NotificationProfilePicker value={["p1", "gone"]} onChange={onChange} />);
    expect(screen.getByText(/1 chosen profile\(s\) no longer exist/)).toBeInTheDocument();
    fireEvent.click(screen.getByText("Remove them"));
    expect(onChange).toHaveBeenLastCalledWith(["p1"]);
  });
});

describe("webhooks and HTML mail (K104/K105)", () => {
  it("edits headers with a secret flag and strips line breaks", () => {
    let value: WebhookHeader[] = [];
    const onChange = vi.fn((v: WebhookHeader[]) => (value = v));
    const { rerender } = render(<HeadersEditor value={value} onChange={onChange} />);
    fireEvent.click(screen.getByTestId("webhook-header-add"));
    rerender(<HeadersEditor value={value} onChange={onChange} />);
    fireEvent.change(screen.getByLabelText("Header name"), { target: { value: "X-Token" } });
    rerender(<HeadersEditor value={value} onChange={onChange} />);
    fireEvent.change(screen.getByLabelText("Header value"), { target: { value: "abc\r\nX-Evil: 1" } });
    rerender(<HeadersEditor value={value} onChange={onChange} />);
    fireEvent.click(screen.getByLabelText("Secret"));
    expect(value[0]).toMatchObject({ name: "X-Token", value: "abcX-Evil: 1", secret: true });
    expect(value[0].credential_id).toMatch(/[0-9a-f-]{36}/);
  });

  it("previews HTML mail in a sandboxed frame and a webhook request", async () => {
    mockInvoke.mockImplementation(async (cmd: string, args: { notification: BackupNotification }) => {
      if (cmd === "backup_preview_notification")
        return args.notification.type === "email"
          ? { subject: "UFOP backup error: Documents", text: "t", html: "<p>Documents</p>", request: null }
          : { subject: "s", text: "t", html: null, request: "POST https://events.pagerduty.com/v2/enqueue\nContent-Type: application/json" };
      return false;
    });
    const email = { ...newNotification("email"), credential_id: "c-mail" };
    render(<NotificationsEditor value={[email, PD]} onChange={() => {}} />);
    const [mailPreview, hookPreview] = screen.getAllByTestId("notification-preview-btn");
    fireEvent.click(mailPreview);
    const frame = await screen.findByTestId("notification-preview-html");
    expect(frame.getAttribute("sandbox")).toBe("");
    expect(frame.getAttribute("srcdoc")).toBe("<p>Documents</p>");
    expect(mockInvoke).toHaveBeenCalledWith("backup_preview_notification", { notification: email, result: "error" });
    fireEvent.click(hookPreview);
    expect((await screen.findByTestId("notification-preview-request")).textContent).toContain("POST https://events.pagerduty.com/v2/enqueue");
    expect(screen.getByTestId("email-html-template")).toBeInTheDocument();
  });

  it("hides HTML template, headers and preview in Standard mode", () => {
    useUIStore.setState({ backupAdvanced: false });
    render(<NotificationsEditor value={[{ ...newNotification("email"), credential_id: "c" }, PD]} onChange={() => {}} />);
    expect(screen.queryByTestId("email-html-template")).toBeNull();
    expect(screen.queryByTestId("webhook-headers")).toBeNull();
    expect(screen.queryByTestId("notification-preview")).toBeNull();
    expect(screen.getByTestId("webhook-url")).toBeInTheDocument();
    expect(screen.getByLabelText("Send as HTML (with a plain-text part)")).toBeChecked();
  });
});
