/** Rule template search: pure matching plus the keyboard-accessible combobox. */
import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { PresetCombobox } from "@/components/preset-combobox";
import { PRESETS, searchPresets } from "@/lib/automation-rule-model";

describe("searchPresets", () => {
  const ids = (q: string) => searchPresets(PRESETS, q).map((p) => p.id);

  it("lists everything for an empty query, in order", () => {
    expect(searchPresets(PRESETS, "  ")).toEqual(PRESETS);
  });

  it("matches the name, description and tags, case-insensitively", () => {
    expect(ids("MALWARE")).toEqual(["malware-scan"]);
    expect(ids("year")).toContain("file-by-date"); // description / tag
    expect(ids("clamav")).toEqual(["malware-scan"]); // tag only
    expect(ids("csv")).toContain("weekly-listing");
  });

  it("needs every word to match, and ranks name matches first", () => {
    expect(ids("empty folders")).toEqual(["remove-empty-folders"]);
    expect(ids("zip nothing")).toEqual([]);
    const r = ids("folder");
    expect(r[0]).toBe("remove-empty-folders"); // a name match sorts ahead of description-only hits
    expect(r.length).toBeGreaterThan(1);
  });

  it("every template has searchable tags", () => {
    for (const p of PRESETS) expect(p.tags.length, p.id).toBeGreaterThan(0);
  });
});

describe("PresetCombobox", () => {
  it("filters as you type and picks with the keyboard", () => {
    const onPick = vi.fn();
    render(<PresetCombobox presets={PRESETS} onPick={onPick} />);
    const box = screen.getByRole("combobox", { name: "Start from a template" });
    expect(box.getAttribute("aria-expanded")).toBe("false");
    fireEvent.focus(box);
    expect(screen.getAllByRole("option")).toHaveLength(PRESETS.length);
    fireEvent.change(box, { target: { value: "antivirus" } });
    expect(screen.getAllByRole("option").map((o) => o.textContent)).toEqual([expect.stringContaining("Scan arrivals for malware")]);
    fireEvent.keyDown(box, { key: "Enter" });
    expect(onPick).toHaveBeenCalledWith(PRESETS.find((p) => p.id === "malware-scan"));
    // Picking resets and closes the list.
    expect((box as HTMLInputElement).value).toBe("");
    expect(screen.queryByRole("listbox")).toBeNull();
  });

  it("moves with the arrow keys, stops at the ends, and closes on Escape", () => {
    const onPick = vi.fn();
    render(<PresetCombobox presets={PRESETS} onPick={onPick} />);
    const box = screen.getByRole("combobox");
    fireEvent.keyDown(box, { key: "ArrowDown" });
    const options = () => screen.getAllByRole("option");
    expect(options()[0].getAttribute("aria-selected")).toBe("true");
    fireEvent.keyDown(box, { key: "ArrowDown" });
    expect(options()[1].getAttribute("aria-selected")).toBe("true");
    expect(box.getAttribute("aria-activedescendant")).toBe(options()[1].id);
    fireEvent.keyDown(box, { key: "End" });
    expect(options()[PRESETS.length - 1].getAttribute("aria-selected")).toBe("true");
    fireEvent.keyDown(box, { key: "ArrowDown" }); // stays on the last
    expect(options()[PRESETS.length - 1].getAttribute("aria-selected")).toBe("true");
    fireEvent.keyDown(box, { key: "Home" });
    fireEvent.keyDown(box, { key: "ArrowUp" });
    expect(options()[0].getAttribute("aria-selected")).toBe("true");
    fireEvent.keyDown(box, { key: "Escape" });
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(onPick).not.toHaveBeenCalled();
  });

  it("picks with the mouse and says when nothing matches", () => {
    const onPick = vi.fn();
    render(<PresetCombobox presets={PRESETS} onPick={onPick} />);
    const box = screen.getByRole("combobox");
    fireEvent.change(box, { target: { value: "zzzz" } });
    expect(screen.getByText(/No template matches/)).toBeTruthy();
    fireEvent.keyDown(box, { key: "Enter" });
    expect(onPick).not.toHaveBeenCalled();
    fireEvent.change(box, { target: { value: "unzip" } });
    fireEvent.click(screen.getByRole("option", { name: /Unzip arriving archives/ }));
    expect(onPick).toHaveBeenCalledWith(PRESETS.find((p) => p.id === "unzip-arrivals"));
  });
});
