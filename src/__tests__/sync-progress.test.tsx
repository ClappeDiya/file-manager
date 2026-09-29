import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { SyncProgressCard } from "@/components/sync-progress-card";
import {
  estimateSecondsLeft,
  formatSecondsLeft,
  phaseLabel,
  progressPercent,
  type SyncProgress,
} from "@/lib/sync-progress";

const base: SyncProgress = {
  pair_id: "p",
  run_id: "r",
  phase: "copying",
  files_done: 2,
  files_total: 4,
  bytes_done: 250,
  bytes_total: 1000,
  current_file: "a/b.txt",
  started_at: new Date(0).toISOString(),
  cancel_requested: false,
};

describe("sync progress helpers", () => {
  it("prefers bytes, falls back to files", () => {
    expect(progressPercent(base)).toBe(25);
    expect(progressPercent({ ...base, bytes_total: 0, bytes_done: 0 })).toBe(50);
    expect(progressPercent({ ...base, files_total: 0, bytes_total: 0, phase: "finishing" })).toBe(100);
  });

  it("estimates time left from elapsed time", () => {
    // 10s elapsed at 25% → 30s left
    expect(estimateSecondsLeft(base, 10_000)).toBe(30);
    expect(estimateSecondsLeft(base, 1_000)).toBeNull(); // too early
    expect(estimateSecondsLeft({ ...base, phase: "planning" }, 10_000)).toBeNull();
    expect(formatSecondsLeft(30)).toBe("about 30s left");
    expect(formatSecondsLeft(600)).toBe("about 10 min left");
    expect(formatSecondsLeft(null)).toBe("");
  });

  it("labels phases in plain language", () => {
    expect(phaseLabel(base)).toBe("Copying 2 of 4");
    expect(phaseLabel({ ...base, phase: "planning" })).toMatch(/Comparing/);
    expect(phaseLabel({ ...base, cancel_requested: true })).toMatch(/Stopping/);
  });
});

describe("SyncProgressCard", () => {
  it("renders a starting state with a Cancel button outside Tauri", () => {
    render(<SyncProgressCard pairId="p" pairName="Photos" />);
    expect(screen.getByText("Syncing Photos")).toBeInTheDocument();
    expect(screen.getByRole("progressbar")).toHaveAttribute("aria-valuenow", "0");
    expect(screen.getByRole("button", { name: "Cancel" })).toBeEnabled();
  });
});
