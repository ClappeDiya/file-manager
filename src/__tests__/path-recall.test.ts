/**
 * path-recall — pure helper that translates a FileLineage into a
 * single actionable hint for the FilePane's "path not found" banner.
 *
 * The tests cover:
 *   - "no useful info" cases (empty lineage, no matching events)
 *   - relocation cases (move, rename) → alternateLocation populated
 *   - deletion cases (delete_trash, delete_permanent) → wasDeleted=true,
 *     alt=null, phrased by recoverability
 *   - copy-from cases → alt=null (the file is still where it was)
 *   - failed events excluded (they're history, not state)
 *   - the most-recent event wins when multiple touch the path
 *   - case-insensitive path matching (macOS/Windows defensiveness)
 *   - describePathRecall produces a sensible phrase per kind
 */
import { describe, it, expect } from "vitest";
import { inferPathRecall, describePathRecall } from "@/lib/path-recall";
import { FS_INTENT_KIND } from "@/lib/fs-kinds";
import type { FileLineage, LineageEvent } from "@/stores/lineage-store";

function makeEvent(overrides: Partial<LineageEvent> = {}): LineageEvent {
  return {
    id: "ev-" + Math.random().toString(36).slice(2, 8),
    occurred_at: "2026-05-14T10:00:00Z",
    engine: "fs",
    kind: "move",
    status: "ok",
    subject_path: "/Users/me/Downloads/report.pdf",
    target_path: "/Users/me/Documents/report.pdf",
    bytes: null,
    correlation_id: "cor-1",
    summary: "moved report.pdf",
    details_json: "{}",
    undo_token: null,
    ...overrides,
  };
}

function makeLineage(events: LineageEvent[]): FileLineage {
  return {
    root_path: events[0]?.subject_path ?? "/",
    aliases: [],
    events,
    correlation_ids: [],
    truncated: false,
  };
}

describe("inferPathRecall", () => {
  const original = "/Users/me/Downloads/report.pdf";

  it("returns null for null lineage", () => {
    expect(inferPathRecall(null, original)).toBeNull();
  });

  it("returns null for empty lineage", () => {
    expect(inferPathRecall(makeLineage([]), original)).toBeNull();
  });

  it("returns null when no event touches the original path", () => {
    const lineage = makeLineage([
      makeEvent({
        subject_path: "/elsewhere/a.txt",
        target_path: "/elsewhere/b.txt",
      }),
    ]);
    expect(inferPathRecall(lineage, original)).toBeNull();
  });

  it("returns alternateLocation for a move OUT of the original path", () => {
    const lineage = makeLineage([
      makeEvent({
        kind: "move",
        subject_path: original,
        target_path: "/Users/me/Documents/report.pdf",
      }),
    ]);
    const info = inferPathRecall(lineage, original);
    expect(info).not.toBeNull();
    expect(info!.alternateLocation).toBe("/Users/me/Documents/report.pdf");
    expect(info!.wasDeleted).toBe(false);
  });

  it("returns alternateLocation for a rename of the original path", () => {
    const lineage = makeLineage([
      makeEvent({
        kind: "rename",
        subject_path: original,
        target_path: "/Users/me/Downloads/report-final.pdf",
      }),
    ]);
    const info = inferPathRecall(lineage, original);
    expect(info!.alternateLocation).toBe("/Users/me/Downloads/report-final.pdf");
  });

  it("sets wasDeleted and clears alternateLocation for a trash delete", () => {
    const lineage = makeLineage([
      makeEvent({
        kind: FS_INTENT_KIND.deleteTrash,
        subject_path: original,
        target_path: null,
      }),
    ]);
    const info = inferPathRecall(lineage, original);
    expect(info!.wasDeleted).toBe(true);
    expect(info!.alternateLocation).toBeNull();
  });

  it("sets wasDeleted=true for a permanent delete of the original path", () => {
    const lineage = makeLineage([
      makeEvent({
        kind: FS_INTENT_KIND.deletePermanent,
        subject_path: original,
        target_path: null,
      }),
    ]);
    expect(inferPathRecall(lineage, original)!.wasDeleted).toBe(true);
  });

  it("leaves alternateLocation null for a copy FROM the original path", () => {
    // copy is informational — the original file is still where it was
    // (unless a later event moved it). Don't suggest an alternate that
    // the user already had.
    const lineage = makeLineage([
      makeEvent({
        kind: "copy",
        subject_path: original,
        target_path: "/some/backup/report.pdf",
      }),
    ]);
    const info = inferPathRecall(lineage, original);
    expect(info!.alternateLocation).toBeNull();
    expect(info!.wasDeleted).toBe(false);
  });

  it("ignores failed events when picking the most-recent state change", () => {
    const lineage = makeLineage([
      makeEvent({
        kind: "move",
        status: "failed",
        subject_path: original,
        target_path: "/wrong/place.pdf",
        occurred_at: "2026-05-14T12:00:00Z",
      }),
      makeEvent({
        kind: "move",
        status: "ok",
        subject_path: original,
        target_path: "/correct/place.pdf",
        occurred_at: "2026-05-14T11:00:00Z",
      }),
    ]);
    const info = inferPathRecall(lineage, original);
    expect(info!.alternateLocation).toBe("/correct/place.pdf");
  });

  it("picks the most-recent matching event when multiple touch the path", () => {
    const lineage = makeLineage([
      makeEvent({
        kind: "rename",
        subject_path: original,
        target_path: "/Users/me/Downloads/v3.pdf",
        occurred_at: "2026-05-14T14:00:00Z",
      }),
      makeEvent({
        kind: "rename",
        subject_path: original,
        target_path: "/Users/me/Downloads/v2.pdf",
        occurred_at: "2026-05-14T10:00:00Z",
      }),
    ]);
    expect(inferPathRecall(lineage, original)!.alternateLocation).toBe(
      "/Users/me/Downloads/v3.pdf",
    );
  });

  it("matches path case-insensitively (macOS / Windows defensiveness)", () => {
    const lineage = makeLineage([
      makeEvent({
        kind: "move",
        subject_path: "/Users/Me/Downloads/Report.PDF",
        target_path: "/elsewhere/r.pdf",
      }),
    ]);
    const info = inferPathRecall(lineage, "/users/me/downloads/report.pdf");
    expect(info).not.toBeNull();
    expect(info!.alternateLocation).toBe("/elsewhere/r.pdf");
  });

  it("returns null when the only relevant event is the path being a copy TARGET that no longer exists", () => {
    // Copy event TO the original path — the path was a copy
    // destination but might have been deleted later. Without a
    // subsequent delete event, we have nothing useful to say
    // beyond "we saw it here once" — surface the event but no alt.
    const lineage = makeLineage([
      makeEvent({
        kind: "copy",
        subject_path: "/source/report.pdf",
        target_path: original,
      }),
    ]);
    const info = inferPathRecall(lineage, original);
    expect(info).not.toBeNull();
    expect(info!.alternateLocation).toBeNull();
    expect(info!.wasDeleted).toBe(false);
  });
});

describe("describePathRecall", () => {
  it("tells the user a trashed file is still recoverable", () => {
    const lineage = makeLineage([
      makeEvent({
        kind: FS_INTENT_KIND.deleteTrash,
        subject_path: "/x/y.txt",
        target_path: null,
      }),
    ]);
    const info = inferPathRecall(lineage, "/x/y.txt")!;
    expect(describePathRecall(info)).toBe("was moved to the Trash");
  });

  it("does not imply a permanently deleted file is recoverable", () => {
    const lineage = makeLineage([
      makeEvent({
        kind: FS_INTENT_KIND.deletePermanent,
        subject_path: "/x/y.txt",
        target_path: null,
      }),
    ]);
    const info = inferPathRecall(lineage, "/x/y.txt")!;
    expect(describePathRecall(info)).toBe("was permanently deleted");
  });

  it("describes moves with the new location", () => {
    const lineage = makeLineage([
      makeEvent({
        kind: "move",
        subject_path: "/a/b.txt",
        target_path: "/c/b.txt",
      }),
    ]);
    const info = inferPathRecall(lineage, "/a/b.txt")!;
    expect(describePathRecall(info)).toBe("was moved to /c/b.txt");
  });

  it("describes renames with the new name", () => {
    const lineage = makeLineage([
      makeEvent({
        kind: "rename",
        subject_path: "/a/old.txt",
        target_path: "/a/new.txt",
      }),
    ]);
    const info = inferPathRecall(lineage, "/a/old.txt")!;
    expect(describePathRecall(info)).toBe("was renamed to /a/new.txt");
  });

  it("describes copy-from events without inventing a destination", () => {
    const lineage = makeLineage([
      makeEvent({
        kind: "copy",
        subject_path: "/a/file.txt",
        target_path: "/b/file.txt",
      }),
    ]);
    const info = inferPathRecall(lineage, "/a/file.txt")!;
    expect(describePathRecall(info)).toBe("was last copied from here");
  });
});

describe("ledger kind coupling", () => {
  // This module hardcoded "delete"/"purge" — a vocabulary the engine never
  // wrote — so wasDeleted was permanently false and a user hunting a deleted
  // file was shown the raw kind ("last delete_trash event recorded"). It
  // type-checked and its tests passed, because the tests invented the same
  // kinds the code did. Pin the real ones against the shared source.
  it("does not treat kinds the engine never writes as deletions", () => {
    for (const invented of ["delete", "purge"]) {
      const lineage = makeLineage([
        makeEvent({ kind: invented, subject_path: "/x/y.txt", target_path: null }),
      ]);
      expect(inferPathRecall(lineage, "/x/y.txt")!.wasDeleted).toBe(false);
    }
  });

  it("phrases every delete kind the engine writes, never its raw identifier", () => {
    for (const kind of [FS_INTENT_KIND.deleteTrash, FS_INTENT_KIND.deletePermanent]) {
      const lineage = makeLineage([
        makeEvent({ kind, subject_path: "/x/y.txt", target_path: null }),
      ]);
      const phrase = describePathRecall(inferPathRecall(lineage, "/x/y.txt")!);
      expect(phrase).not.toContain(kind);
      expect(phrase).toMatch(/Trash|permanently deleted/);
    }
  });
});
