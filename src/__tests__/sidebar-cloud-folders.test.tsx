import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { SidebarNav, type CloudFolder } from "@/components/sidebar-nav";

const FOLDERS: CloudFolder[] = [
  { provider: "dropbox", label: "Dropbox", path: "/home/me/Dropbox" },
  { provider: "onedrive", label: "OneDrive — Contoso", path: "/home/me/OneDrive - Contoso" },
];

let folders: CloudFolder[] = [];

beforeEach(() => {
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  folders = FOLDERS;
  mockInvoke.mockReset().mockImplementation(async (cmd: string) => (cmd === "detect_cloud_folders" ? folders : []));
});

describe("sidebar: cloud folders", () => {
  it("lists the detected folders and opens one in the pane", async () => {
    const onNavigate = vi.fn();
    render(<SidebarNav onNavigate={onNavigate} />);
    const section = await screen.findByTestId("sidebar-cloud-folders");
    expect(section).toHaveTextContent("Cloud folders");
    expect(section).toHaveTextContent("OneDrive — Contoso");
    fireEvent.click(screen.getByTestId("sidebar-cloud-folder-dropbox"));
    expect(onNavigate).toHaveBeenCalledWith("/home/me/Dropbox");
    expect(screen.getByTestId("sidebar-cloud-folder-onedrive")).toHaveAttribute("title", "/home/me/OneDrive - Contoso");
  });

  it("shows no section when nothing is installed or outside the desktop app", async () => {
    folders = [];
    const { unmount } = render(<SidebarNav onNavigate={() => {}} />);
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("detect_cloud_folders", undefined));
    expect(screen.queryByTestId("sidebar-cloud-folders")).toBeNull();
    unmount();
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    render(<SidebarNav onNavigate={() => {}} />);
    await new Promise((r) => setTimeout(r, 20));
    expect(screen.queryByTestId("sidebar-cloud-folders")).toBeNull();
  });
});
