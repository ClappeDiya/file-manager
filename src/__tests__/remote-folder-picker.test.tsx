import { describe, it, expect, vi, beforeEach, beforeAll } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));
vi.mock("@/lib/prompt-dialog", () => ({ promptText: vi.fn(async () => "New Stuff"), confirmText: vi.fn(async () => true) }));
vi.mock("@/hooks/use-drive-space", () => ({ useAllDrives: () => [{ name: "Disk", mount_point: "/", total_bytes: 0, free_bytes: 0, removable: false }] }));

import { FolderField, RemoteFolderPicker } from "@/components/remote-folder-picker";
import { useConnectionsStore } from "@/stores/connections-store";
import { relativeToRoot } from "@/lib/location";

const ID = "6f1c2b0a-1111-4222-8333-944455556666";
const dir = (path: string, name: string) => ({ name, path, is_dir: true, is_symlink: false, size: 0, modified: null, created: null, is_hidden: false, extension: null, permissions: null });
const calls = (cmd: string) => mockInvoke.mock.calls.filter((c) => c[0] === cmd).map((c) => c[1]);

beforeAll(() => {
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});

beforeEach(() => {
  useConnectionsStore.setState({ connections: [{ id: ID, name: "Work S3", protocol: "s3", remote_path: "/bucket" }], statuses: {}, loaded: true });
  mockInvoke.mockReset().mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    if (cmd === "list_directory") return args?.path === "/" ? [dir("/home", "home")] : args?.path === "/home" ? [dir("/home/me", "me")] : [];
    if (cmd === "connector_list_remote") return args?.path === "/bucket" ? [dir("/bucket/photos", "photos")] : [];
    return null;
  });
});

describe("RemoteFolderPicker", () => {
  it("browses this computer lazily and returns the chosen folder", async () => {
    const onPick = vi.fn();
    render(<RemoteFolderPicker title="Pick" onPick={onPick} onCancel={vi.fn()} />);
    expect(await screen.findByText("home")).toBeInTheDocument();
    expect(calls("list_directory")).toEqual([{ path: "/" }]);
    fireEvent.click(screen.getByLabelText("Expand home"));
    expect(await screen.findByText("me")).toBeInTheDocument();
    fireEvent.click(screen.getByText("me"));
    expect(screen.getByTestId("picker-selected")).toHaveTextContent("/home/me");
    fireEvent.click(screen.getByTestId("picker-choose"));
    expect(onPick).toHaveBeenCalledWith("/home/me");
  });

  it("browses a connection through the connector and creates a folder there", async () => {
    const onPick = vi.fn();
    render(<RemoteFolderPicker title="Pick" onPick={onPick} onCancel={vi.fn()} />);
    fireEvent.click(await screen.findByTestId(`picker-root-${ID}`));
    expect(await screen.findByText("photos")).toBeInTheDocument();
    expect(calls("connector_list_remote")).toContainEqual({ protocol: "s3", path: "/bucket", connectionId: ID });
    fireEvent.click(screen.getByTestId("picker-new-folder"));
    await waitFor(() => expect(calls("connector_make_directory")).toEqual([{ protocol: "s3", remotePath: "/bucket/New Stuff", connectionId: ID }]));
    await waitFor(() => expect(screen.getByTestId("picker-selected")).toHaveTextContent("Work S3:/bucket/New Stuff"));
    fireEvent.click(screen.getByTestId("picker-choose"));
    expect(onPick).toHaveBeenCalledWith(`conn://${ID}/bucket/New Stuff`);
  });

  it("can be limited to this computer, or locked to one connection folder", async () => {
    const { unmount } = render(<RemoteFolderPicker title="Pick" allowConnections={false} onPick={vi.fn()} onCancel={vi.fn()} />);
    await screen.findByText("home");
    expect(screen.queryByTestId(`picker-root-${ID}`)).not.toBeInTheDocument();
    unmount();
    render(<RemoteFolderPicker title="Pick" lockRoot={`conn://${ID}/bucket`} onPick={vi.fn()} onCancel={vi.fn()} />);
    expect(await screen.findByText("photos")).toBeInTheDocument();
    expect(screen.queryByText("home")).not.toBeInTheDocument();
  });

  it("FolderField keeps the native Browse… and adds Pick…, converting values", async () => {
    const onChange = vi.fn();
    render(
      <FolderField
        value="photos"
        onChange={onChange}
        testId="sub"
        lockRoot={`conn://${ID}/bucket`}
        toValue={(p) => relativeToRoot("/bucket", p.replace(`conn://${ID}`, ""))}
        fromValue={(v) => `conn://${ID}/bucket/${v}`}
      />,
    );
    // Locked to a connection: no native (local) picker.
    expect(screen.queryByText("Browse…")).not.toBeInTheDocument();
    fireEvent.click(screen.getByTestId("sub-pick"));
    fireEvent.click(await screen.findByText("photos"));
    fireEvent.click(screen.getByTestId("picker-choose"));
    expect(onChange).toHaveBeenCalledWith("photos");
    expect(relativeToRoot("/", "/a/b")).toBe("a/b");
    expect(relativeToRoot("/bucket", "/bucket")).toBe("");
  });
});
