/**
 * Key-value metadata editor and search: argument shapes match the Rust
 * commands and edits round-trip through them.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { MetadataEditor, MetadataSearch } from "@/components/metadata-editor";

const info = (metadata: Record<string, string>) => ({ path: "/d/a.pdf", tags: [], labels: [], metadata });

beforeEach(() => {
  mockInvoke.mockReset();
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});

describe("metadata editor", () => {
  it("shows, adds, edits and removes values", async () => {
    let current: Record<string, string> = { client: "Acme" };
    mockInvoke.mockImplementation(async (cmd: string, args: Record<string, string>) => {
      if (cmd === "integrity_metadata_keys") return ["client", "invoice"];
      if (cmd === "metadata_schema_for") return null;
      if (cmd === "integrity_set_metadata") current = { ...current, [args.key]: args.value };
      if (cmd === "integrity_remove_metadata") {
        const { [args.key]: _gone, ...rest } = current;
        current = rest;
      }
      return info(current);
    });
    render(<MetadataEditor path="/d/a.pdf" />);
    expect(await screen.findByDisplayValue("Acme")).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("New metadata key"), { target: { value: "invoice" } });
    fireEvent.change(screen.getByLabelText("New metadata value"), { target: { value: "INV-7" } });
    fireEvent.click(screen.getByText("Set"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("integrity_set_metadata", { path: "/d/a.pdf", key: "invoice", value: "INV-7" }),
    );
    expect(await screen.findByDisplayValue("INV-7")).toBeInTheDocument();

    const client = screen.getByLabelText("Value for client");
    fireEvent.change(client, { target: { value: "Acme Inc" } });
    fireEvent.blur(client);
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("integrity_set_metadata", { path: "/d/a.pdf", key: "client", value: "Acme Inc" }),
    );

    fireEvent.click(screen.getByLabelText("Remove invoice"));
    await waitFor(() => expect(screen.queryByDisplayValue("INV-7")).not.toBeInTheDocument());
  });

  it("surfaces validation errors from Rust", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "integrity_set_metadata") throw new Error("Metadata keys must be 1–128 characters");
      if (cmd === "integrity_metadata_keys") return [];
      if (cmd === "metadata_schema_for") return null;
      return info({});
    });
    render(<MetadataEditor path="/d/a.pdf" />);
    fireEvent.change(await screen.findByLabelText("New metadata key"), { target: { value: "x" } });
    fireEvent.click(screen.getByText("Set"));
    expect(await screen.findByText(/1–128 characters/)).toBeInTheDocument();
  });
});

describe("metadata search", () => {
  it("searches by key and value, optionally in this folder", async () => {
    mockInvoke.mockImplementation(async (cmd: string) =>
      cmd === "integrity_metadata_keys" ? ["client"] : [{ path: "/p/a.txt", key: "client", value: "Acme" }],
    );
    const onOpen = vi.fn();
    render(<MetadataSearch under="/p" onOpen={onOpen} />);
    await screen.findByRole("option", { name: "client" });
    fireEvent.change(screen.getByLabelText("Metadata key"), { target: { value: "client" } });
    fireEvent.change(screen.getByLabelText("Metadata value contains"), { target: { value: "acme" } });
    fireEvent.click(screen.getByText("Search"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("integrity_search_metadata", {
        key: "client", value: "acme", contains: true, under: "/p", limit: 500,
      }),
    );
    fireEvent.click(await screen.findByText("/p/a.txt"));
    expect(onOpen).toHaveBeenCalledWith("/p/a.txt");
  });
});
