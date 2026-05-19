/**
 * Locks in the symmetry between the Pin and Retry row affordances on the
 * activity timeline. Pin consumes successful fs.copy / fs.move rows;
 * Retry consumes the failed (or cancelled) counterpart. They MUST never
 * both be true for the same row — each ledger event has exactly one
 * status — and they share the same engine + kind + correlation-id
 * preconditions so the visual placement of the two buttons stays a
 * coherent mirror rather than a parallel system.
 */
import { describe, it, expect } from "vitest";
import {
  isPinnableEvent,
  isRetryableEvent,
  isUndoableKindEvent,
} from "@/lib/ledger-event-flags";
import type { LedgerEventWire as LedgerEvent } from "@/lib/ledger-tail-extract";

function makeEvent(overrides: Partial<LedgerEvent> = {}): LedgerEvent {
  return {
    id: "ev-1",
    occurred_at: "2026-05-14T10:00:00Z",
    engine: "fs",
    kind: "copy",
    status: "ok",
    subject_path: "/src/a.txt",
    target_path: "/dst/a.txt",
    bytes: 100,
    correlation_id: "cor-1",
    summary: "copy a.txt",
    details_json: "{}",
    undo_token: null,
    ...overrides,
  };
}

describe("isPinnableEvent", () => {
  it("accepts a successful fs.copy with a correlation id", () => {
    expect(isPinnableEvent(makeEvent())).toBe(true);
  });

  it("accepts a successful fs.move with a correlation id", () => {
    expect(isPinnableEvent(makeEvent({ kind: "move" }))).toBe(true);
  });

  it("rejects rows from non-fs engines", () => {
    expect(isPinnableEvent(makeEvent({ engine: "sync" }))).toBe(false);
  });

  it("rejects failed rows — they are the Retry surface, not Pin", () => {
    expect(isPinnableEvent(makeEvent({ status: "failed" }))).toBe(false);
  });

  it("rejects non-replayable kinds", () => {
    expect(isPinnableEvent(makeEvent({ kind: "rename" }))).toBe(false);
    expect(isPinnableEvent(makeEvent({ kind: "delete" }))).toBe(false);
  });

  it("rejects rows without a correlation id", () => {
    expect(isPinnableEvent(makeEvent({ correlation_id: null }))).toBe(false);
    expect(isPinnableEvent(makeEvent({ correlation_id: "" }))).toBe(false);
  });
});

describe("isRetryableEvent", () => {
  it("accepts a failed fs.copy with a correlation id", () => {
    expect(isRetryableEvent(makeEvent({ status: "failed" }))).toBe(true);
  });

  it("accepts a cancelled fs.move with a correlation id", () => {
    expect(
      isRetryableEvent(makeEvent({ kind: "move", status: "cancelled" })),
    ).toBe(true);
  });

  it("rejects successful rows — they are the Pin surface, not Retry", () => {
    expect(isRetryableEvent(makeEvent({ status: "ok" }))).toBe(false);
  });

  it("rejects skipped rows — skip is an explicit decision, not a failure", () => {
    expect(isRetryableEvent(makeEvent({ status: "skipped" }))).toBe(false);
  });

  it("rejects non-fs engines so transfer/sync failures stay with their own retry paths", () => {
    expect(
      isRetryableEvent(makeEvent({ engine: "transfer", status: "failed" })),
    ).toBe(false);
  });

  it("rejects non-replayable kinds even when failed", () => {
    expect(
      isRetryableEvent(makeEvent({ kind: "rename", status: "failed" })),
    ).toBe(false);
  });

  it("rejects rows without a correlation id", () => {
    expect(
      isRetryableEvent(
        makeEvent({ status: "failed", correlation_id: null }),
      ),
    ).toBe(false);
  });
});

describe("isUndoableKindEvent", () => {
  // Gate predicate for the Undo/Redo per-row affordances. Mirror of
  // `UNDOABLE_FS_KINDS` in Rust — keep them in lock-step.
  it("accepts each of the six fs undoable kinds with a correlation id", () => {
    for (const kind of [
      "copy",
      "duplicate",
      "move",
      "rename",
      "create_folder",
      "create_file",
    ]) {
      expect(
        isUndoableKindEvent(makeEvent({ kind })),
        `kind=${kind}`,
      ).toBe(true);
    }
  });

  it("rejects the fs.undone marker row (same correlation_id as the original op)", () => {
    expect(isUndoableKindEvent(makeEvent({ kind: "fs.undone" }))).toBe(false);
  });

  it("rejects the fs.redone marker row", () => {
    expect(isUndoableKindEvent(makeEvent({ kind: "fs.redone" }))).toBe(false);
  });

  it("rejects non-fs engines", () => {
    expect(isUndoableKindEvent(makeEvent({ engine: "sync" }))).toBe(false);
    expect(isUndoableKindEvent(makeEvent({ engine: "transfer" }))).toBe(false);
  });

  it("rejects events without a correlation id", () => {
    expect(
      isUndoableKindEvent(makeEvent({ correlation_id: null })),
    ).toBe(false);
  });
});

describe("Pin / Retry mutual exclusion", () => {
  // Each ledger event has exactly one status, so the two flags must
  // never agree. This test sweeps every combination this codebase can
  // emit and asserts the invariant — guarding against a future kind
  // or status addition that accidentally lights both buttons.
  const statuses = ["ok", "failed", "cancelled", "skipped"] as const;
  const kinds = ["copy", "move", "rename", "delete", "edit_text"] as const;
  const engines = ["fs", "transfer", "sync", "automation"] as const;

  it("never marks the same row both pinnable and retryable", () => {
    for (const status of statuses) {
      for (const kind of kinds) {
        for (const engine of engines) {
          const ev = makeEvent({ status, kind, engine });
          const pinnable = isPinnableEvent(ev);
          const retryable = isRetryableEvent(ev);
          expect(pinnable && retryable, `${engine}.${kind}/${status}`).toBe(
            false,
          );
        }
      }
    }
  });
});
