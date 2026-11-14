import { describe, it, expect, vi, beforeEach } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { AddressBar, ADDRESS_SUGGEST_DEBOUNCE_MS } from "@/components/address-bar";
import { BreadcrumbBar } from "@/components/breadcrumb-bar";
import { clearRemoteNames, rememberRemoteNames } from "@/lib/location";

const ID = "6f1c2b0a-1111-4222-8333-944455556666";
const connections = [{ id: ID, name: "Work Drive" }];
const dir = (path: string, name: string) => ({ path, name, is_dir: true });

function setup(over: Partial<React.ComponentProps<typeof AddressBar>> = {}) {
  const props = {
    path: "/home/me",
    connections,
    listFolders: vi.fn(async (p: string) => (p === "/home" ? [dir("/home/me", "me"), dir("/home/music", "music"), { path: "/home/x.txt", name: "x.txt", is_dir: false }] : [])),
    validate: vi.fn(async () => null as string | null),
    history: ["/var/log", `conn://${ID}/Docs`],
    onNavigate: vi.fn(),
    onCancel: vi.fn(),
    ...over,
  };
  render(<AddressBar {...props} />);
  return { props, input: screen.getByTestId("address-bar-input") as HTMLInputElement };
}

describe("AddressBar", () => {
  beforeEach(() => clearRemoteNames());

  it("starts with the current folder, Enter navigates, Esc cancels", async () => {
    const { props, input } = setup();
    expect(input.value).toBe("/home/me");
    fireEvent.change(input, { target: { value: "/var/log" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(props.onNavigate).toHaveBeenCalledWith("/var/log"));
    fireEvent.keyDown(input, { key: "Escape" });
    expect(props.onCancel).toHaveBeenCalled();
  });

  it("shows a connection folder as <name>:path and accepts that form", async () => {
    const { props, input } = setup({ path: `conn://${ID}/Docs` });
    expect(input.value).toBe("Work Drive:/Docs");
    fireEvent.change(input, { target: { value: "work drive:/Archive" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(props.onNavigate).toHaveBeenCalledWith(`conn://${ID}/Archive`));
  });

  it("shows an inline error for addresses that do not parse or do not exist", async () => {
    const validate = vi.fn(async (p: string) => (p === "/nope" ? "That folder does not exist" : null));
    const { props, input } = setup({ validate });
    fireEvent.change(input, { target: { value: "Nowhere:/x" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(await screen.findByTestId("address-bar-error")).toHaveTextContent('No saved connection is named "Nowhere"');
    expect(input).toHaveAttribute("aria-invalid", "true");
    fireEvent.change(input, { target: { value: "/nope" } });
    expect(screen.queryByTestId("address-bar-error")).not.toBeInTheDocument();
    fireEvent.keyDown(input, { key: "Enter" });
    expect(await screen.findByTestId("address-bar-error")).toHaveTextContent("That folder does not exist");
    expect(props.onNavigate).not.toHaveBeenCalled();
  });

  it("goes to the path validation resolved a typed address to (Google Drive names → ids)", async () => {
    const validate = vi.fn(async () => ({ path: `conn://${ID}/r1` }));
    const { props, input } = setup({ validate });
    fireEvent.change(input, { target: { value: "Work Drive:/Reports" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(props.onNavigate).toHaveBeenCalledWith(`conn://${ID}/r1`));
    expect(validate).toHaveBeenCalledWith(`conn://${ID}/Reports`);
  });

  it("suggests the typed parent's folders (debounced) and recent places", async () => {
    vi.useFakeTimers();
    try {
      const { props, input } = setup();
      fireEvent.change(input, { target: { value: "/home/mu" } });
      expect(props.listFolders).not.toHaveBeenCalledWith("/home");
      await act(async () => {
        vi.advanceTimersByTime(ADDRESS_SUGGEST_DEBOUNCE_MS + 10);
      });
      expect(props.listFolders).toHaveBeenCalledWith("/home");
      const list = screen.getByTestId("address-bar-suggestions");
      expect(list).toHaveTextContent("music");
      expect(list).not.toHaveTextContent("x.txt");
      expect(list).not.toHaveTextContent(/^me$/);
      // Tab completes the first suggestion as a folder.
      fireEvent.keyDown(input, { key: "Tab" });
      expect(input.value).toBe("/home/music/");
      // Recent places match anywhere in the address.
      fireEvent.change(input, { target: { value: "log" } });
      expect(screen.getByTestId("address-bar-suggestions")).toHaveTextContent("/var/log");
      fireEvent.change(input, { target: { value: "Work" } });
      expect(screen.getByTestId("address-bar-suggestions")).toHaveTextContent("Work Drive:/Docs");
    } finally {
      vi.useRealTimers();
    }
  });

  it("arrow keys pick a suggestion and Enter goes there", async () => {
    const { props, input } = setup({ history: ["/var/log"] });
    fireEvent.change(input, { target: { value: "/var" } });
    fireEvent.keyDown(input, { key: "ArrowDown" });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(props.onNavigate).toHaveBeenCalledWith("/var/log"));
  });
});

describe("BreadcrumbBar with connections and editing", () => {
  it("starts a connection path at the connection, with listed names", () => {
    rememberRemoteNames([{ path: `conn://${ID}/f1`, name: "Reports" }]);
    const onNavigate = vi.fn();
    render(<BreadcrumbBar path={`conn://${ID}/f1`} onNavigate={onNavigate} connectionName={() => "Work Drive"} />);
    expect(screen.getByTestId("breadcrumb-connection")).toHaveTextContent("Work Drive");
    expect(screen.getByTestId("breadcrumb-segment-0")).toHaveTextContent("Reports");
    fireEvent.click(screen.getByTestId("breadcrumb-root"));
    expect(onNavigate).toHaveBeenCalledWith(`conn://${ID}/`);
  });

  it("turns into the address field when the empty part is clicked", () => {
    const onEditingChange = vi.fn();
    const address = { connections, listFolders: async () => [], validate: async () => null, history: [] };
    const { rerender } = render(<BreadcrumbBar path="/home" onNavigate={vi.fn()} address={address} editing={false} onEditingChange={onEditingChange} />);
    expect(screen.queryByTestId("address-bar")).not.toBeInTheDocument();
    fireEvent.click(screen.getByTestId("breadcrumb-edit"));
    expect(onEditingChange).toHaveBeenCalledWith(true);
    rerender(<BreadcrumbBar path="/home" onNavigate={vi.fn()} address={address} editing onEditingChange={onEditingChange} />);
    expect(screen.getByTestId("address-bar-input")).toHaveValue("/home");
    expect(screen.queryByTestId("breadcrumb-segment-0")).not.toBeInTheDocument();
  });
});
