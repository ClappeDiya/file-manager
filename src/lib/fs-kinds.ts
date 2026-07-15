/**
 * The FS engine's ledger vocabulary — the exact `kind` strings
 * `file_ops_commands.rs` writes to the operation ledger.
 *
 * Anything that reads ledger rows back (the Safety Interlock's baseline
 * lookup, path recall's lineage inference) or writes an intent the engine
 * must match has to speak these strings verbatim. A kind the engine never
 * records simply matches zero rows — silently, with no error anywhere.
 *
 * That has already happened twice, both times invisibly:
 *
 *   - The Safety Interlock sent `"delete"` / `"purge"`, so every delete
 *     assessment ran against a permanently empty baseline and its adaptive
 *     ratio logic never once ran for a delete.
 *   - Path recall hardcoded the same wrong pair, so `wasDeleted` was never
 *     true and a user hunting a file they deleted was told
 *     "last delete_trash event recorded" instead of "was moved to the Trash".
 *
 * Both were written from the same wrong mental model, and both type-checked
 * and passed their tests. Hence one home: a new consumer imports these rather
 * than guessing a third time. `fs-intent-kinds.test.ts` pins the strings
 * against the Rust writer.
 */
export const FS_INTENT_KIND = {
  move: "move",
  deleteTrash: "delete_trash",
  deletePermanent: "delete_permanent",
} as const;

export type FsIntentKind = (typeof FS_INTENT_KIND)[keyof typeof FS_INTENT_KIND];
