/** WS-C: edit / duplicate / export / import sync pairs, filter rules and presets, global defaults. */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { SyncPanel } from "@/components/sync-panel";
import { DEFAULT_COPY_OPTIONS } from "@/lib/copy-options";
import { changedOptionKeys, emptyPairForm, filterFromForm, formFromPair, layerOptions, pairFromForm } from "@/lib/sync-pair-form";
import { useUIStore } from "@/stores/ui-store";
import type { SyncPair } from "@/stores/sync-store";

const pair: SyncPair = {
  id: "p1",
  name: "Photos",
  source_path: "/home/me/Pictures",
  dest_path: "/mnt/nas/photos",
  mode: "one_way",
  enabled: true,
  last_run: null,
  trigger: { scheduled: { cron_expr: "0 0 2 * * * *" } },
  filter: {
    include_patterns: [],
    exclude_patterns: ["*.tmp"],
    include_regex: ["^keep/"],
    exclude_regex: [],
    min_size: 0,
    max_size: 0,
    file_types: [],
    exclude_file_types: [],
    rules: [{ action: "exclude", kind: "group", pattern: "video" }],
  },
  conflict_policy: "create_conflict_copy",
  verify_mode: "fast",
  checksum_enabled: false,
  created_at: "2026-10-01T00:00:00Z",
  time_offset_secs: null,
  copy_options: { ...DEFAULT_COPY_OPTIONS, threads: 4 },
  run_options: { settle_secs: 0, completion_marker: null, version_template: null },
  option_overrides: ["threads"],
};

const PRESET = { id: "u1", name: "No archives", rules: [{ action: "exclude", kind: "group", pattern: "archives" }], created_at: "2026-10-01T00:00:00Z" };

function backend(extra: Record<string, (a: Record<string, unknown>) => unknown> = {}) {
  const handlers: Record<string, (a: Record<string, unknown>) => unknown> = {
    list_sync_pairs: () => [pair],
    get_sync_health: () => "green",
    get_sync_default_options: () => ({ ...DEFAULT_COPY_OPTIONS, retries: 2 }),
    list_user_filter_presets: () => [PRESET],
    get_filter_groups: () => [{ name: "video", description: "Video files", patterns: ["*.mp4"] }],
    update_sync_pair: (a) => JSON.parse(a.pairJson as string),
    create_sync_pair: () => ({ ...pair, id: "p2" }),
    // A scheduled pair's copy keeps its cron expression and starts disabled.
    duplicate_sync_pair: () => ({ ...pair, id: "p3", name: "Photos (copy)", enabled: false }),
    export_sync_pair: () => '{"format":"ufop-sync-pair","version":1}',
    import_sync_pair: () => ({ pair: { ...pair, id: "p4", enabled: false }, warnings: ["The source folder /home/me/Pictures does not exist on this computer."] }),
    schedules_list: () => [],
    calendars_list: () => [],
    parse_filter_rules: () => [{ action: "include", kind: "rclone", pattern: "/Photos/**" }],
    ...extra,
  };
  mockInvoke.mockImplementation(async (cmd: string, args: Record<string, unknown>) => {
    const h = handlers[cmd];
    return h ? h(args ?? {}) : cmd.startsWith("list") || cmd.startsWith("get_sync_conflicts") ? [] : null;
  });
}

const called = (cmd: string) => mockInvoke.mock.calls.filter(([c]) => c === cmd).map(([, a]) => a);

beforeEach(() => {
  mockInvoke.mockReset();
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  useUIStore.setState({ appMode: "simple" });
});

describe("sync pair form mapping", () => {
  it("layers the pair's own options over the global defaults", () => {
    const defaults = { ...DEFAULT_COPY_OPTIONS, retries: 2, threads: 8 };
    expect(layerOptions(defaults, pair.copy_options!, ["threads"])).toMatchObject({ threads: 4, retries: 2 });
    // Pairs from before defaults keep all their own options.
    expect(layerOptions(defaults, pair.copy_options!, null)).toMatchObject({ threads: 4, retries: 0 });
    expect(changedOptionKeys({ ...defaults, exclude_hidden: true }, defaults)).toEqual(["exclude_hidden"]);
  });

  it("round-trips a pair through the form", () => {
    const defaults = { ...DEFAULT_COPY_OPTIONS, retries: 2 };
    const form = formFromPair(pair, defaults);
    expect(form.trigger).toBe("scheduled");
    expect(form.cronExpr).toBe("0 0 2 * * * *");
    expect(form.copyOptions).toMatchObject({ threads: 4, retries: 2 });
    const saved = pairFromForm({ ...form, name: "Photos nightly" }, pair, defaults);
    expect(saved.id).toBe("p1");
    expect(saved.name).toBe("Photos nightly");
    expect(saved.filter.include_regex).toEqual(["^keep/"]);
    expect(saved.filter.rules).toEqual(pair.filter.rules);
    expect(saved.option_overrides).toEqual(["threads"]);
    expect(saved.enabled).toBe(true);
    expect(pairFromForm({ ...form, enabled: false }, pair, defaults).enabled).toBe(false);
    expect(pairFromForm(form, { ...pair, option_overrides: null }, defaults).option_overrides).toBeNull();
    expect(filterFromForm(emptyPairForm()).rules).toEqual([]);
  });
});

describe("SyncPanel: edit, duplicate, export, import", () => {
  it("edits a pair with the create form and saves it with update_sync_pair", async () => {
    backend();
    render(<SyncPanel />);
    fireEvent.click(await screen.findByLabelText("Edit Photos"));
    expect(screen.getByText('Edit "Photos"')).toBeInTheDocument();
    const name = screen.getByDisplayValue("Photos");
    fireEvent.change(name, { target: { value: "Photos nightly" } });
    fireEvent.click(screen.getByRole("button", { name: "Save Changes" }));
    await waitFor(() => expect(called("update_sync_pair").length).toBe(1));
    const saved = JSON.parse(called("update_sync_pair")[0].pairJson as string);
    expect(saved).toMatchObject({ id: "p1", name: "Photos nightly", trigger: { scheduled: { cron_expr: "0 0 2 * * * *" } } });
    expect(saved.filter.exclude_patterns).toEqual(["*.tmp"]);
    expect(called("create_sync_pair")).toHaveLength(0);
  });

  it("duplicates a pair", async () => {
    backend();
    render(<SyncPanel />);
    fireEvent.click(await screen.findByLabelText("Duplicate Photos"));
    await waitFor(() => expect(called("duplicate_sync_pair")).toEqual([{ pairId: "p1" }]));
    expect(await screen.findByText(/Created "Photos \(copy\)". It is disabled/)).toBeInTheDocument();
  });

  it("keeps a disabled copy's schedule and turns it on from Edit", async () => {
    const copy = { ...pair, id: "p3", name: "Photos (copy)", enabled: false };
    backend({ list_sync_pairs: () => [copy] });
    render(<SyncPanel />);
    expect(await screen.findByTestId("sync-pair-disabled")).toBeInTheDocument();
    fireEvent.click(screen.getByLabelText("Edit Photos (copy)"));
    expect(screen.getByDisplayValue("0 0 2 * * * *")).toBeInTheDocument();
    const toggle = screen.getByLabelText(/Enabled \(runs on its trigger/);
    expect(toggle).not.toBeChecked();
    fireEvent.click(toggle);
    fireEvent.click(screen.getByRole("button", { name: "Save Changes" }));
    await waitFor(() => expect(called("update_sync_pair").length).toBe(1));
    const saved = JSON.parse(called("update_sync_pair")[0].pairJson as string);
    expect(saved).toMatchObject({ id: "p3", enabled: true, trigger: { scheduled: { cron_expr: "0 0 2 * * * *" } } });
  });

  it("warns that a two-way pair with blind upload never deletes", async () => {
    backend({ list_sync_pairs: () => [{ ...pair, mode: "two_way", dest_state: "blind_upload" }] });
    render(<SyncPanel />);
    fireEvent.click(await screen.findByLabelText("Edit Photos"));
    expect(screen.getByTestId("sync-blind-two-way-hint")).toHaveTextContent(/nothing is\s+deleted/);
  });

  it("notes that a two-way pair scans the destination despite a trusted inventory", async () => {
    backend({ list_sync_pairs: () => [{ ...pair, mode: "two_way", dest_state: "trusted_inventory" }] });
    render(<SyncPanel />);
    fireEvent.click(await screen.findByLabelText("Edit Photos"));
    expect(screen.getByTestId("sync-inventory-two-way-hint")).toHaveTextContent(/scan the destination every run/);
    expect(screen.queryByTestId("sync-blind-two-way-hint")).toBeNull();
  });

  it("exports and imports pairs in the Advanced view only", async () => {
    backend();
    const { unmount } = render(<SyncPanel />);
    await screen.findByLabelText("Edit Photos");
    expect(screen.queryByLabelText("Export Photos")).toBeNull();
    expect(screen.queryByLabelText("Import a sync pair")).toBeNull();
    unmount();

    useUIStore.setState({ appMode: "advanced" });
    URL.createObjectURL = vi.fn(() => "blob:x");
    URL.revokeObjectURL = vi.fn();
    render(<SyncPanel />);
    fireEvent.click(await screen.findByLabelText("Export Photos"));
    await waitFor(() => expect(called("export_sync_pair")).toEqual([{ pairId: "p1" }]));
    const file = new File(['{"format":"ufop-sync-pair"}'], "pair.json", { type: "application/json" });
    fireEvent.change(screen.getByLabelText("Import a sync pair"), { target: { files: [file] } });
    await waitFor(() => expect(called("import_sync_pair")).toEqual([{ json: '{"format":"ufop-sync-pair"}' }]));
    expect(await screen.findByText(/does not exist on this computer/)).toBeInTheDocument();
  });
});

describe("SyncPanel: filter rules and presets", () => {
  it("applies a saved preset in the Standard view and sends the rules", async () => {
    backend();
    render(<SyncPanel />);
    fireEvent.click(await screen.findByLabelText("Create new sync pair"));
    const rules = screen.getByTestId("sync-filter-rules");
    // Standard view: presets yes, rule editor no.
    expect(within(rules).queryByRole("button", { name: "Add rule" })).toBeNull();
    const picker = await within(rules).findByLabelText("Apply a saved filter preset");
    await within(rules).findByText("No archives (1 rule)");
    fireEvent.change(picker, { target: { value: "u1" } });
    expect(within(rules).getByText(/\(1, first match wins\)/)).toBeInTheDocument();
    fireEvent.change(screen.getByPlaceholderText("My Backup"), { target: { value: "New" } });
    fireEvent.change(screen.getByPlaceholderText("/home/user/documents"), { target: { value: "/a" } });
    fireEvent.change(screen.getByPlaceholderText("/backup/documents"), { target: { value: "/b" } });
    fireEvent.click(screen.getByRole("button", { name: "Create Sync Pair" }));
    await waitFor(() => expect(called("create_sync_pair").length).toBe(1));
    const args = called("create_sync_pair")[0];
    expect(JSON.parse(args.filterJson as string).rules).toEqual(PRESET.rules);
    // Layered on the global defaults: only what differs is the pair's own.
    expect(args.optionOverrides).toEqual([]);
  });

  // Regression: in the Standard view a preset's rules were added to the pair
  // but only the Advanced-only editor showed them, so they could not be undone.
  it("lists applied rules read-only in the Standard view, with remove and clear", async () => {
    backend({ list_user_filter_presets: () => [{ ...PRESET, rules: [...PRESET.rules, { action: "include", kind: "rclone", pattern: "/Photos/**" }] }] });
    render(<SyncPanel />);
    fireEvent.click(await screen.findByLabelText("Create new sync pair"));
    const rules = screen.getByTestId("sync-filter-rules");
    const picker = await within(rules).findByLabelText("Apply a saved filter preset");
    await within(rules).findByText("No archives (2 rules)");
    fireEvent.change(picker, { target: { value: "u1" } });
    const list = within(rules).getByTestId("filter-rules-summary");
    expect(within(list).getByText("archives")).toBeInTheDocument();
    expect(within(list).getByText("/Photos/**")).toBeInTheDocument();
    expect(within(list).queryByRole("textbox")).toBeNull();
    fireEvent.click(within(list).getByRole("button", { name: "Remove rule archives" }));
    expect(within(list).queryByText("archives")).toBeNull();
    expect(within(rules).getByText(/\(1, first match wins\)/)).toBeInTheDocument();
    fireEvent.click(within(list).getByRole("button", { name: "Clear all rules" }));
    expect(within(rules).queryByTestId("filter-rules-summary")).toBeNull();
    fireEvent.change(screen.getByPlaceholderText("My Backup"), { target: { value: "New" } });
    fireEvent.change(screen.getByPlaceholderText("/home/user/documents"), { target: { value: "/a" } });
    fireEvent.change(screen.getByPlaceholderText("/backup/documents"), { target: { value: "/b" } });
    fireEvent.click(screen.getByRole("button", { name: "Create Sync Pair" }));
    await waitFor(() => expect(called("create_sync_pair").length).toBe(1));
    expect(JSON.parse(called("create_sync_pair")[0].filterJson as string).rules).toEqual([]);
  });

  it("edits ordered rules and imports rclone filter files in the Advanced view", async () => {
    backend();
    useUIStore.setState({ appMode: "advanced" });
    render(<SyncPanel />);
    fireEvent.click(await screen.findByLabelText("Create new sync pair"));
    const rules = screen.getByTestId("sync-filter-rules");
    fireEvent.click(within(rules).getByRole("button", { name: "Add rule" }));
    expect(within(rules).getAllByDisplayValue("Exclude").length).toBe(1);
    const file = new File(["+ /Photos/**\n- *\n"], "rules.txt", { type: "text/plain" });
    fireEvent.change(within(rules).getByTestId("import-filter-rules-input"), { target: { files: [file] } });
    await waitFor(() => expect(called("parse_filter_rules")).toEqual([{ text: "+ /Photos/**\n- *\n" }]));
    expect(await within(rules).findByText(/1 rule imported from rules.txt/)).toBeInTheDocument();
    expect(within(rules).getByDisplayValue("/Photos/**")).toBeInTheDocument();
  });
});
