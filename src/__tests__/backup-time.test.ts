import { describe, expect, it } from "vitest";
import { localInputToIso, pickVersionAsOf, toLocalInput } from "@/lib/backup-time";
import type { Fileset } from "@/stores/backup-store";

const fs = (timestamp: string, partial = false): Fileset =>
  ({ id: 0, timestamp, volume_name: "", partial, file_count: 0, total_size: 0, source: "", note: null, tags: {}, pinned: false, pin_reason: null, identity: null, stats: null }) as Fileset;

const versions = [fs("2026-10-03T12:00:00+00:00", true), fs("2026-10-02T12:00:00+00:00"), fs("2026-10-01T12:00:00+00:00", true), fs("2026-10-01T09:00:00+00:00")];

describe("restore as of a time", () => {
  it("picks the newest complete version not after the time", () => {
    expect(pickVersionAsOf(versions, "2026-10-02T00:00:00Z")).toBe(3);
    expect(pickVersionAsOf(versions, "2026-10-02T00:00:00Z", true)).toBe(2);
    expect(pickVersionAsOf(versions, "2026-10-02T12:00:00Z")).toBe(1);
    expect(pickVersionAsOf(versions, "2026-10-02T14:00:00+02:00")).toBe(1);
    expect(pickVersionAsOf(versions, "2030-01-01T00:00:00Z")).toBe(1);
    expect(pickVersionAsOf(versions, "2030-01-01T00:00:00Z", true)).toBe(0);
    expect(pickVersionAsOf(versions, "2020-01-01T00:00:00Z")).toBeNull();
    expect(pickVersionAsOf(versions, "nonsense")).toBeNull();
  });

  it("round-trips local input values", () => {
    const d = new Date(2026, 9, 1, 9, 5);
    expect(toLocalInput(d)).toBe("2026-10-01T09:05");
    expect(Date.parse(localInputToIso("2026-10-01T09:05")!)).toBe(d.getTime());
    expect(localInputToIso("")).toBeNull();
    expect(localInputToIso("not a date")).toBeNull();
  });
});
