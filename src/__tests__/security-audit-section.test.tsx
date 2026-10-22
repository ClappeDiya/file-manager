/**
 * Security audit log section: status, "Verify integrity" (pass and fail),
 * checkpoint signing and JSON-lines export over the Rust IPC contract.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { SecurityAuditSection } from "@/components/security-audit-section";
import { describeIpcError, type AuditLogStatus, type AuditVerification } from "@/lib/audit-log";

const status: AuditLogStatus = {
  entries: 12,
  last_sequence: 12,
  last_hash: "ab".repeat(32),
  anchors: 2,
  last_anchor: null,
  unanchored_entries: 3,
  append_only_enforced: true,
  key_id: "0123456789abcdef",
  public_key: "cd".repeat(32),
  key_source: "os_keychain",
};

const okResult: AuditVerification = {
  ok: true,
  entries_checked: 12,
  first_bad_sequence: null,
  first_bad_anchor: null,
  reason: null,
  anchors_verified: 2,
  anchors_total: 2,
  last_sequence: 12,
  last_hash: "ab".repeat(32),
  append_only_enforced: true,
  key_id: "0123456789abcdef",
  verified_at: "2026-10-22T10:00:00.000000Z",
};

describe("SecurityAuditSection", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
    (window as unknown as Record<string, unknown>).__TAURI__ = {};
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  });

  afterEach(() => {
    delete (window as unknown as Record<string, unknown>).__TAURI__;
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  });

  it("shows status and a passing verification", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "get_audit_log_status") return status;
      if (cmd === "verify_audit_log") return okResult;
      throw new Error(`unexpected ${cmd}`);
    });
    render(<SecurityAuditSection />);
    const st = await screen.findByTestId("audit-status");
    expect(st.textContent).toContain("12");
    expect(st.textContent).toContain("3 newer entries not yet checkpointed");
    expect(st.textContent).toContain("0123456789abcdef");
    expect(st.textContent).toContain("OS keychain");

    fireEvent.click(screen.getByTestId("audit-verify"));
    const res = await screen.findByTestId("audit-verify-result");
    expect(mockInvoke).toHaveBeenCalledWith("verify_audit_log", undefined);
    expect(res.dataset.ok).toBe("true");
    expect(res.textContent).toContain("Audit log intact");
    expect(res.textContent).toContain("12 entries verified");
    expect(res.textContent).toContain("2 of 2 checkpoints");
  });

  it("pinpoints the first bad entry when verification fails", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "get_audit_log_status") return { ...status, append_only_enforced: false };
      if (cmd === "verify_audit_log")
        return {
          ...okResult,
          ok: false,
          entries_checked: 3,
          first_bad_sequence: 4,
          first_bad_anchor: 1,
          anchors_verified: 0,
          reason: "entry 4 content does not match its hash (altered)",
          append_only_enforced: false,
        } satisfies AuditVerification;
      throw new Error(`unexpected ${cmd}`);
    });
    render(<SecurityAuditSection />);
    expect((await screen.findByTestId("audit-status")).textContent).toContain("Append-only triggers are missing");
    fireEvent.click(screen.getByTestId("audit-verify"));
    const res = await screen.findByTestId("audit-verify-result");
    expect(res.dataset.ok).toBe("false");
    expect(res.textContent).toContain("FAILED");
    expect(res.textContent).toContain("First bad entry: #4");
    expect(res.textContent).toContain("First bad checkpoint: #1");
    expect(res.textContent).toContain("entry 4 content does not match its hash");
  });

  it("surfaces backend errors with their advice", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "get_audit_log_status") return status;
      if (cmd === "create_audit_anchor")
        throw { error_type: "state", message: "Refusing to sign an audit checkpoint.", advice: "Run Verify integrity." };
      throw new Error(`unexpected ${cmd}`);
    });
    render(<SecurityAuditSection />);
    await screen.findByTestId("audit-status");
    fireEvent.click(screen.getByTestId("audit-anchor"));
    expect((await screen.findByTestId("audit-error")).textContent).toBe(
      "Refusing to sign an audit checkpoint. Run Verify integrity.",
    );
  });

  it("signs a checkpoint and exports JSON lines", async () => {
    const jsonl = ['{"type":"header"}', '{"type":"entry"}', '{"type":"anchor"}', ""].join("\n");
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "get_audit_log_status") return status;
      if (cmd === "create_audit_anchor")
        return { anchor_seq: 3, last_sequence: 12, last_hash: "ab", count: 12, created_at: "", key_id: "k", algorithm: "ed25519", signature: "s" };
      if (cmd === "export_audit_log") return jsonl;
      throw new Error(`unexpected ${cmd}`);
    });
    const createUrl = vi.fn(() => "blob:audit");
    const revokeUrl = vi.fn();
    Object.assign(URL, { createObjectURL: createUrl, revokeObjectURL: revokeUrl });
    const click = vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});

    render(<SecurityAuditSection />);
    await screen.findByTestId("audit-status");
    fireEvent.click(screen.getByTestId("audit-anchor"));
    expect(await screen.findByText("Checkpoint #3 signed over 12 entries")).toBeTruthy();

    fireEvent.click(screen.getByTestId("audit-export"));
    await waitFor(() => expect(click).toHaveBeenCalled());
    expect(mockInvoke).toHaveBeenCalledWith("export_audit_log", undefined);
    expect(createUrl).toHaveBeenCalledTimes(1);
    expect(revokeUrl).toHaveBeenCalledWith("blob:audit");
    expect(await screen.findByText("Exported 2 records (entries and checkpoints)")).toBeTruthy();
    click.mockRestore();
  });

  it("formats IPC errors", () => {
    expect(describeIpcError({ message: "m", advice: "a" })).toBe("m a");
    expect(describeIpcError(new Error("boom"))).toBe("boom");
    expect(describeIpcError("plain")).toBe("plain");
  });

  it("renders without Tauri (no status, actions still present)", async () => {
    delete (window as unknown as Record<string, unknown>).__TAURI__;
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    render(<SecurityAuditSection />);
    expect(screen.getByTestId("audit-verify")).toBeTruthy();
    await waitFor(() => expect(screen.queryByTestId("audit-status")).toBeNull());
    expect(mockInvoke).not.toHaveBeenCalled();
  });
});
