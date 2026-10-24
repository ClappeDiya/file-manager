import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { ALL_TIME, formatLocalTime, rangeArgs, rangeBounds, rangeInverted, type HistoryRange } from "@/lib/history-range";
import { HistoryRangeChips } from "@/components/history-range-chips";

const NOW = new Date(2026, 9, 24, 15, 30, 0); // 24 Oct 2026, 15:30 local
const r = (preset: HistoryRange["preset"], from = "", to = ""): HistoryRange => ({ preset, from, to });
const local = (iso: string | null) => (iso ? new Date(iso) : null);

describe("rangeBounds", () => {
  it("is open for all time, so calls look as they did before ranges existed", () => {
    expect(rangeBounds(ALL_TIME, NOW)).toEqual({ from: null, to: null });
    expect(rangeArgs(ALL_TIME, NOW)).toEqual({});
  });

  it("starts Today at local midnight", () => {
    const { from, to } = rangeBounds(r("today"), NOW);
    expect(local(from)).toEqual(new Date(2026, 9, 24, 0, 0, 0, 0));
    expect(to).toBeNull();
  });

  it("covers exactly yesterday, in local days", () => {
    const { from, to } = rangeBounds(r("yesterday"), NOW);
    expect(local(from)).toEqual(new Date(2026, 9, 23, 0, 0, 0, 0));
    expect(local(to)).toEqual(new Date(2026, 9, 23, 23, 59, 59, 999));
  });

  it("counts the last 7 / 30 days as today and the days before it", () => {
    expect(local(rangeBounds(r("7d"), NOW).from)).toEqual(new Date(2026, 9, 18, 0, 0, 0, 0));
    expect(local(rangeBounds(r("30d"), NOW).from)).toEqual(new Date(2026, 8, 25, 0, 0, 0, 0));
  });

  it("turns a custom range into whole local days, both ends included", () => {
    const { from, to } = rangeBounds(r("custom", "2026-10-01", "2026-10-03"), NOW);
    expect(local(from)).toEqual(new Date(2026, 9, 1, 0, 0, 0, 0));
    expect(local(to)).toEqual(new Date(2026, 9, 3, 23, 59, 59, 999));
  });

  it("leaves an unfilled custom side open and ignores garbage", () => {
    expect(rangeArgs(r("custom", "2026-10-01", ""), NOW)).toEqual({ dateFrom: local(rangeBounds(r("custom", "2026-10-01"), NOW).from)!.toISOString() });
    expect(rangeArgs(r("custom", "nonsense", "2026-13-45"), NOW).dateFrom).toBeUndefined();
  });

  it("spots a range that ends before it starts", () => {
    expect(rangeInverted(r("custom", "2026-10-05", "2026-10-01"))).toBe(true);
    expect(rangeInverted(r("custom", "2026-10-01", "2026-10-05"))).toBe(false);
    expect(rangeInverted(r("today"))).toBe(false);
  });
});

describe("formatLocalTime", () => {
  it("formats an instant in the computer's time zone and tolerates non-times", () => {
    expect(formatLocalTime("2026-10-01T12:00:00Z")).toBe(new Date("2026-10-01T12:00:00Z").toLocaleString());
    expect(formatLocalTime(null)).toBe("—");
    expect(formatLocalTime("soon")).toBe("soon");
  });
});

describe("HistoryRangeChips", () => {
  it("offers the five chips and reports the choice", () => {
    const onChange = vi.fn();
    render(<HistoryRangeChips value={ALL_TIME} onChange={onChange} />);
    for (const name of ["Today", "Yesterday", "Last 7 days", "Last 30 days", "Custom"]) {
      expect(screen.getByRole("button", { name })).toBeInTheDocument();
    }
    fireEvent.click(screen.getByRole("button", { name: "Last 7 days" }));
    expect(onChange).toHaveBeenCalledWith({ preset: "7d", from: "", to: "" });
  });

  it("clears the active chip when it is pressed again", () => {
    const onChange = vi.fn();
    render(<HistoryRangeChips value={r("today")} onChange={onChange} />);
    expect(screen.getByRole("button", { name: "Today" })).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(screen.getByRole("button", { name: "Today" }));
    expect(onChange).toHaveBeenCalledWith(ALL_TIME);
  });

  it("shows the two dates for Custom and warns about an inverted range", () => {
    const onChange = vi.fn();
    const { rerender } = render(<HistoryRangeChips value={r("custom")} onChange={onChange} />);
    fireEvent.change(screen.getByLabelText("Date range from"), { target: { value: "2026-10-05" } });
    expect(onChange).toHaveBeenCalledWith({ preset: "custom", from: "2026-10-05", to: "" });
    expect(screen.queryByRole("alert")).toBeNull();
    rerender(<HistoryRangeChips value={r("custom", "2026-10-05", "2026-10-01")} onChange={onChange} />);
    expect(screen.getByRole("alert")).toHaveTextContent("before the start");
  });
});
