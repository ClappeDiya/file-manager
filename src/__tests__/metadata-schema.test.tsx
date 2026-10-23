/**
 * Metadata schemas: the editor renders schema fields with typed inputs and
 * hides free-form keys under a strict schema; the panel creates a schema with
 * the payload shape Rust expects and shows the compliance report.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { MetadataEditor } from "@/components/metadata-editor";
import { MetadataSchemaPanel } from "@/components/metadata-schema";
import { emptyField, type MetadataSchema } from "@/lib/metadata-schema";

const schema = (over: Partial<MetadataSchema> = {}): MetadataSchema => ({
  id: "s1",
  name: "Invoices",
  folder: "/d",
  fields: [
    { ...emptyField(), key: "status", label: "Status", kind: "choice", required: true, options: ["Draft", "Paid"] },
    { ...emptyField(), key: "due", kind: "date" },
    { ...emptyField(), key: "approved", kind: "boolean" },
  ],
  strict: true,
  recursive: true,
  created_at: "",
  updated_at: "",
  ...over,
});

beforeEach(() => {
  mockInvoke.mockReset();
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});

describe("schema-aware metadata editor", () => {
  it("shows typed schema fields, marks missing required ones and hides free keys when strict", async () => {
    let current: Record<string, string> = { due: "2026-03-01" };
    mockInvoke.mockImplementation(async (cmd: string, args: Record<string, string>) => {
      if (cmd === "metadata_schema_for") return schema();
      if (cmd === "integrity_metadata_keys") return [];
      if (cmd === "integrity_set_metadata") current = { ...current, [args.key]: args.value };
      if (cmd === "integrity_remove_metadata") {
        const { [args.key]: _gone, ...rest } = current;
        current = rest;
      }
      return { path: "/d/a.pdf", tags: [], labels: [], metadata: current };
    });
    render(<MetadataEditor path="/d/a.pdf" />);
    const group = await screen.findByLabelText("Schema Invoices");
    expect(within(group).getByText("Status")).toHaveClass("text-amber-600");
    expect(within(group).getByLabelText("required")).toBeInTheDocument();
    expect(screen.getByLabelText("Value for due")).toHaveAttribute("type", "date");
    expect(screen.queryByLabelText("New metadata key")).not.toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("Value for status"), { target: { value: "Paid" } });
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("integrity_set_metadata", { path: "/d/a.pdf", key: "status", value: "Paid" }));
    await waitFor(() => expect(within(group).getByText("Status")).not.toHaveClass("text-amber-600"));

    fireEvent.click(screen.getByLabelText("Value for approved"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("integrity_set_metadata", { path: "/d/a.pdf", key: "approved", value: "true" }));

    // Clearing a value removes the key rather than storing "".
    const due = screen.getByLabelText("Value for due");
    fireEvent.change(due, { target: { value: "" } });
    fireEvent.blur(due);
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("integrity_remove_metadata", { path: "/d/a.pdf", key: "due" }));
  });

  it("keeps free-form keys under a non-strict schema", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "metadata_schema_for") return schema({ strict: false });
      if (cmd === "integrity_metadata_keys") return [];
      return { path: "/d/a.pdf", tags: [], labels: [], metadata: { note: "hi" } };
    });
    render(<MetadataEditor path="/d/a.pdf" />);
    expect(await screen.findByDisplayValue("hi")).toBeInTheDocument();
    expect(screen.getByLabelText("New metadata key")).toBeInTheDocument();
  });
});

describe("metadata schema panel", () => {
  it("creates a schema for the folder and checks its files", async () => {
    let saved: MetadataSchema | null = null;
    mockInvoke.mockImplementation(async (cmd: string, args: Record<string, unknown>) => {
      if (cmd === "metadata_schema_for") return saved;
      if (cmd === "metadata_save_schema") {
        saved = { ...(args.schema as MetadataSchema), id: "new" };
        return saved;
      }
      if (cmd === "metadata_check_compliance")
        return { schema: saved, files_checked: 3, compliant: 2, truncated: false, issues: [{ path: "/data/inv/b.pdf", key: "status", problem: "missing", message: "Status is required" }] };
      return null;
    });
    render(<MetadataSchemaPanel folder="/data/inv" />);
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("metadata_schema_for", { path: "/data/inv", isDir: true }));
    fireEvent.click(await screen.findByText("Create schema"));
    expect(screen.getByLabelText("Schema name")).toHaveValue("inv");
    fireEvent.change(screen.getByLabelText("Field 1 key"), { target: { value: "status" } });
    fireEvent.change(screen.getByLabelText("Field 1 type"), { target: { value: "choice" } });
    fireEvent.change(screen.getByLabelText("Field 1 options"), { target: { value: "Draft, Paid" } });
    fireEvent.click(screen.getByLabelText("Field 1 required"));
    fireEvent.click(screen.getByText("Only these fields (strict)"));
    fireEvent.click(screen.getByText("Save schema"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("metadata_save_schema", expect.anything()));
    const sent = mockInvoke.mock.calls.find(([c]) => c === "metadata_save_schema")![1].schema as MetadataSchema;
    expect(sent).toMatchObject({ id: "", folder: "/data/inv", strict: true, recursive: true });
    expect(sent.fields[0]).toMatchObject({ key: "status", kind: "choice", required: true, options: ["Draft", "Paid"], pattern: null, min: null, max: null });

    expect(await screen.findByLabelText("Schema fields")).toHaveTextContent("status · Choice · required (Draft, Paid)");
    fireEvent.click(screen.getByText("Check files"));
    const report = await screen.findByLabelText("Compliance report");
    expect(report).toHaveTextContent("2 of 3 files comply");
    expect(report).toHaveTextContent("b.pdf");
    expect(report).toHaveTextContent("Status is required");
  });

  it("shows an inherited schema and offers an override; surfaces save errors", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "metadata_schema_for") return schema({ folder: "/d" });
      if (cmd === "metadata_save_schema") throw new Error("status: a choice field needs options");
      return null;
    });
    render(<MetadataSchemaPanel folder="/d/sub" />);
    expect(await screen.findByText(/inherited from/)).toHaveTextContent("/d");
    expect(screen.queryByText("Delete schema")).not.toBeInTheDocument();
    fireEvent.click(screen.getByText("Override for this folder"));
    fireEvent.change(screen.getByLabelText("Field 1 key"), { target: { value: "x" } });
    fireEvent.click(screen.getByText("Save schema"));
    expect(await screen.findByText(/needs options/)).toBeInTheDocument();
  });
});
