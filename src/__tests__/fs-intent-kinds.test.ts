/**
 * Pins the frontend half of the intent-kind contract.
 *
 * `OperationIntent.kind` is used verbatim by the Rust Safety Interlock as its
 * ledger baseline lookup key (`WHERE kind = ?`), so these strings must equal
 * the kinds the Rust FS engine writes in
 * `src-tauri/src/commands/file_ops_commands.rs`.
 *
 * The Rust half is pinned by `safety::tests::fs_delete_baseline_sees_what_the
 * _engine_actually_wrote`, which drives the real engine and asserts the same
 * literals. Together the two tests fail if either side is renamed alone.
 *
 * The regression: the UI sent "delete"/"purge" while the engine recorded
 * "delete_trash"/"delete_permanent", so the delete baseline matched zero rows
 * forever — the interlock claimed "First-ever delete of this kind on this
 * device" on every delete of 25+ files and never once ran its ratio logic.
 */
import { describe, it, expect } from "vitest";
import { FS_INTENT_KIND } from "@/lib/safety";

describe("FS_INTENT_KIND", () => {
  it("matches the kinds the Rust FS engine writes to the ledger", () => {
    // Keep in lockstep with file_ops_commands.rs record sites.
    expect(FS_INTENT_KIND.move).toBe("move");
    expect(FS_INTENT_KIND.deleteTrash).toBe("delete_trash");
    expect(FS_INTENT_KIND.deletePermanent).toBe("delete_permanent");
  });

  it("does not reintroduce the kinds the engine never records", () => {
    // "delete" and "purge" are what the UI used to send. The engine has never
    // written either, so an intent carrying one silently gets an empty baseline.
    const sent = Object.values(FS_INTENT_KIND) as string[];
    expect(sent).not.toContain("delete");
    expect(sent).not.toContain("purge");
  });
});
