/** Filename policies settings: edit, save in Rust's shape, test a path. */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { FilenamePoliciesSection, type FilenameRule } from "@/components/filename-policies-section";

let stored: FilenameRule[] = [];
beforeEach(() => {
  stored = [];
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  mockInvoke.mockReset().mockImplementation(async (cmd: string, a: { rules: FilenameRule[]; paths: string[] }) => {
    if (cmd === "filename_policies_list") return stored;
    if (cmd === "filename_policies_save") {
      if (a.rules.some((r) => r.required_pattern === "(")) throw "Policy 'x': invalid pattern";
      stored = a.rules.map((r, i) => ({ ...r, id: r.id || `id${i}` }));
      return stored;
    }
    if (cmd === "filename_policy_check")
      return a.paths[0].endsWith(".exe") ? [{ rule_id: "id0", rule_name: "No programs", path: a.paths[0], reason: ".exe files are not allowed here" }] : [];
    return null;
  });
});

describe("FilenamePoliciesSection", () => {
  it("adds a rule and saves it in the engine's shape", async () => {
    render(<FilenamePoliciesSection />);
    await screen.findByText(/No rules/);
    fireEvent.click(screen.getByText("Add rule"));
    fireEvent.change(screen.getByLabelText("Rule 1 name"), { target: { value: "No programs" } });
    fireEvent.change(screen.getByLabelText("Blocked extensions"), { target: { value: ".EXE, bat" } });
    fireEvent.change(screen.getByLabelText(/Max name length/), { target: { value: "120" } });
    fireEvent.click(screen.getByText("Save rules"));
    await screen.findByText("Saved");
    expect(stored).toEqual([
      expect.objectContaining({ id: "id0", name: "No programs", scope: "", blocked_extensions: ["EXE", "bat"], allowed_extensions: null, max_name_bytes: 120, required_pattern: null, enabled: true }),
    ]);
  });

  it("shows the engine's validation error and keeps the edits", async () => {
    render(<FilenamePoliciesSection />);
    await screen.findByText(/No rules/);
    fireEvent.click(screen.getByText("Add rule"));
    fireEvent.change(screen.getByLabelText("Names must match (regex)"), { target: { value: "(" } });
    fireEvent.click(screen.getByText("Save rules"));
    expect((await screen.findByRole("alert")).textContent).toContain("invalid pattern");
    expect(screen.getByLabelText("Rule 1 name")).toBeTruthy();
  });

  it("tests a path against the saved rules", async () => {
    render(<FilenamePoliciesSection />);
    fireEvent.change(screen.getByLabelText(/Test a path/), { target: { value: "/tmp/setup.exe" } });
    fireEvent.click(screen.getByText("Test"));
    await waitFor(() => expect(screen.getByTestId("policy-test-result").textContent).toBe("Blocked: .exe files are not allowed here (No programs)"));
    fireEvent.change(screen.getByLabelText(/Test a path/), { target: { value: "/tmp/a.txt" } });
    fireEvent.click(screen.getByText("Test"));
    await waitFor(() => expect(screen.getByTestId("policy-test-result").textContent).toBe("Allowed"));
  });
});
