/** Sync run options section: move-after-copy, settle window, marker, version names. */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { SyncRunOptionsSection } from "@/components/sync-run-options";
import { DEFAULT_COPY_OPTIONS, type CopyOptions } from "@/lib/copy-options";
import { DEFAULT_RUN_OPTIONS, type SyncRunOptions } from "@/stores/sync-store";

const changes = vi.fn<(r: SyncRunOptions, c: CopyOptions) => void>();
function Harness({ mode }: { mode: string }) {
  const [r, setR] = useState(DEFAULT_RUN_OPTIONS);
  const [c, setC] = useState(DEFAULT_COPY_OPTIONS);
  return <SyncRunOptionsSection mode={mode} runOptions={r} copyOptions={c} onChange={(a, b) => { changes(a, b); setR(a); setC(b); }} />;
}
const last = () => changes.mock.calls[changes.mock.calls.length - 1];

beforeEach(() => {
  changes.mockReset();
  mockInvoke.mockReset().mockResolvedValue("report~1.pdf");
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});

describe("SyncRunOptionsSection", () => {
  it("maps the choices onto run and copy options", () => {
    render(<Harness mode="one_way" />);
    fireEvent.change(screen.getByLabelText("Source files"), { target: { value: "move" } });
    expect(last()[1].move_files).toBe(true);
    fireEvent.change(screen.getByLabelText(/Skip files changed/), { target: { value: "45" } });
    expect(last()[0].settle_secs).toBe(45);
    fireEvent.change(screen.getByLabelText(/Completion marker/), { target: { value: "_DONE" } });
    expect(last()[0]).toEqual({ settle_secs: 45, completion_marker: "_DONE", version_template: null });
    fireEvent.change(screen.getByLabelText(/Completion marker/), { target: { value: "  " } });
    expect(last()[0].completion_marker).toBeNull();
    expect(screen.queryByLabelText("Version names")).toBeNull();
  });

  it("offers no move option for two-way sync", () => {
    render(<Harness mode="two_way" />);
    expect(screen.queryByLabelText("Source files")).toBeNull();
  });

  it("previews version names with Rust's renderer in versioned mode", async () => {
    render(<Harness mode="versioned_backup" />);
    fireEvent.change(screen.getByLabelText("Version names"), { target: { value: "{name}~{num}.{ext}" } });
    expect((await screen.findByTestId("version-preview", {}, { timeout: 2000 })).textContent).toBe("report.pdf → report~1.pdf");
    expect(mockInvoke).toHaveBeenCalledWith("naming_preview", { template: "{name}~{num}.{ext}", samplePath: "report.pdf", matchRegex: null, asPath: false });
  });
});
