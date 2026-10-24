/**
 * "Notify when a run finishes" picker shared by sync pairs and automation
 * rules: profile choice, per-result filter, Advanced size threshold, and the
 * pieces that put it in the sync and automation editors.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { RunNotifyPicker } from "@/components/run-notify-picker";
import { SyncRunOptionsSection } from "@/components/sync-run-options";
import { bytesToMb, mbToBytes, type RunNotify } from "@/lib/run-notify";
import { useNotifyStore } from "@/stores/backup-notify";
import { DEFAULT_COPY_OPTIONS } from "@/lib/copy-options";
import { DEFAULT_RUN_OPTIONS, type SyncRunOptions } from "@/stores/sync-store";
import { useUIStore } from "@/stores/ui-store";

const PROFILES = [
  { id: "p1", name: "Ops Slack", description: "", notification: { type: "slack", credential_id: "c", send_on: [], template: "", cert: {} }, min_severity: null, managed: false, created_at: "", updated_at: "" },
  { id: "p2", name: "Mail", description: "", notification: { type: "email", credential_id: "c", send_on: [], template: "", cert: {} }, min_severity: null, managed: false, created_at: "", updated_at: "" },
];

beforeEach(() => {
  mockInvoke.mockReset();
  mockInvoke.mockImplementation(async (cmd: string) => (cmd === "backup_list_notification_profiles" ? PROFILES : null));
  useNotifyStore.setState({ profiles: [], error: null });
  useUIStore.setState({ appMode: "simple" });
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});

function Harness({ initial = {}, advanced = false, seen }: { initial?: RunNotify; advanced?: boolean; seen: (v: RunNotify) => void }) {
  const [v, setV] = useState<RunNotify>(initial);
  return <RunNotifyPicker value={v} advanced={advanced} onChange={(p) => { const n = { ...v, ...p }; seen(n); setV(n); }} />;
}

describe("size helpers", () => {
  it("converts MB text to bytes and back; blank or invalid means no threshold", () => {
    expect(mbToBytes("1")).toBe(1_048_576);
    expect(mbToBytes("0.5")).toBe(524_288);
    expect(mbToBytes("")).toBeNull();
    expect(mbToBytes("-3")).toBeNull();
    expect(mbToBytes("abc")).toBeNull();
    expect(bytesToMb(1_048_576)).toBe("1");
    expect(bytesToMb(null)).toBe("");
  });
});

describe("RunNotifyPicker", () => {
  it("lists the shared profiles and reports the choice", async () => {
    const seen = vi.fn();
    render(<Harness seen={seen} />);
    const select = await screen.findByTestId("run-notify-profile");
    await screen.findByText("Ops Slack (Slack)");
    fireEvent.change(select, { target: { value: "p1" } });
    expect(seen).toHaveBeenLastCalledWith({ notify_profile_id: "p1" });
    // Results can be limited once a profile is chosen.
    fireEvent.click(screen.getByLabelText("Errors"));
    fireEvent.click(screen.getByLabelText("Warnings"));
    expect(seen).toHaveBeenLastCalledWith({ notify_profile_id: "p1", notify_on: ["error", "warning"] });
    fireEvent.click(screen.getByLabelText("Errors"));
    expect(seen).toHaveBeenLastCalledWith({ notify_profile_id: "p1", notify_on: ["warning"] });
    // Back to nobody.
    fireEvent.change(select, { target: { value: "" } });
    expect(seen).toHaveBeenLastCalledWith({ notify_profile_id: null, notify_on: ["warning"] });
  });

  it("shows the size threshold only in Advanced, in MB, keeping partial typing", async () => {
    const seen = vi.fn();
    const { rerender } = render(<Harness initial={{ notify_profile_id: "p1" }} seen={seen} />);
    await screen.findByText("Mail (Email)");
    expect(screen.queryByLabelText(/Don’t report successful runs below/)).toBeNull();
    rerender(<Harness initial={{ notify_profile_id: "p1" }} advanced seen={seen} />);
    const input = screen.getByLabelText(/Don’t report successful runs below/) as HTMLInputElement;
    fireEvent.change(input, { target: { value: "0." } });
    expect(input.value).toBe("0.");
    fireEvent.change(input, { target: { value: "2" } });
    expect(seen).toHaveBeenLastCalledWith({ notify_profile_id: "p1", min_bytes: 2_097_152 });
    fireEvent.change(input, { target: { value: "" } });
    expect(seen).toHaveBeenLastCalledWith({ notify_profile_id: "p1", min_bytes: null });
  });

  it("warns when the chosen profile was deleted", async () => {
    render(<Harness initial={{ notify_profile_id: "gone" }} seen={vi.fn()} />);
    await screen.findByText("Ops Slack (Slack)");
    expect(screen.getByText(/no longer exists/)).toBeTruthy();
  });

  it("explains how to create a first profile", async () => {
    mockInvoke.mockResolvedValue([]);
    render(<Harness seen={vi.fn()} />);
    expect(await screen.findByText(/No profiles yet/)).toBeTruthy();
  });
});

describe("sync pair form", () => {
  it("stores the notification settings in the pair's run options", async () => {
    const changes = vi.fn<(r: SyncRunOptions) => void>();
    function Form() {
      const [r, setR] = useState(DEFAULT_RUN_OPTIONS);
      return <SyncRunOptionsSection mode="one_way" runOptions={r} copyOptions={DEFAULT_COPY_OPTIONS} onChange={(a) => { changes(a); setR(a); }} />;
    }
    render(<Form />);
    await screen.findByText("Ops Slack (Slack)");
    fireEvent.change(screen.getByTestId("run-notify-profile"), { target: { value: "p2" } });
    await waitFor(() => expect(changes).toHaveBeenCalled());
    expect(changes.mock.calls.at(-1)![0]).toEqual({ settle_secs: 0, completion_marker: null, version_template: null, notify_profile_id: "p2" });
  });
});
