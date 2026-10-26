import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import SettingsPanel from "@/components/settings-panel";

// Outside Tauri every section loads its fallback; the rclone section shows its error/loading line.
const scrollIntoView = vi.fn();

beforeEach(() => {
  scrollIntoView.mockReset();
  Element.prototype.scrollIntoView = scrollIntoView;
});

describe("Settings focused on a section", () => {
  it("opens and scrolls to the rclone section when asked", async () => {
    render(<SettingsPanel focusSection="rclone" />);
    const section = screen.getByTestId("settings-section-rclone");
    expect(section).toHaveTextContent("collapse");
    expect(scrollIntoView).toHaveBeenCalled();
    expect(scrollIntoView.mock.contexts[0]).toBe(section);
  });

  it("leaves every section collapsed when opened plainly", () => {
    render(<SettingsPanel />);
    expect(screen.getByTestId("settings-section-rclone")).toHaveTextContent("expand");
    expect(scrollIntoView).not.toHaveBeenCalled();
  });
});
