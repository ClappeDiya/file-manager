import { describe, it, expect } from "vitest";
import {
  findMountForPath,
  formatDriveSpace,
  formatDriveSpaceDetail,
  type DriveInfoLike,
} from "@/lib/drive-space";

function drive(
  mount: string,
  free: number,
  total: number,
  name = mount,
): DriveInfoLike {
  return {
    name,
    mount_point: mount,
    free_bytes: free,
    total_bytes: total,
  };
}

describe("findMountForPath", () => {
  it("returns null for an empty drives list", () => {
    expect(findMountForPath([], "/foo")).toBeNull();
  });

  it("matches a single drive at root", () => {
    const root = drive("/", 100, 1000, "Macintosh HD");
    expect(findMountForPath([root], "/Users/md")).toBe(root);
  });

  it("returns null when no drive matches an empty path", () => {
    const root = drive("/", 100, 1000);
    expect(findMountForPath([root], "")).toBeNull();
  });

  it("picks the longest-prefix mount over a shorter ancestor", () => {
    const root = drive("/", 100, 1000, "Macintosh HD");
    const usb = drive("/Volumes/USB", 50, 200, "USB Stick");
    expect(findMountForPath([root, usb], "/Volumes/USB/photos")).toBe(usb);
    expect(findMountForPath([root, usb], "/Users/md")).toBe(root);
  });

  it("treats a trailing slash on the mount equivalently", () => {
    const usb = drive("/Volumes/USB/", 50, 200);
    expect(findMountForPath([usb], "/Volumes/USB/photos")).toBe(usb);
  });

  it("treats an exact-match path as inside the mount", () => {
    const usb = drive("/Volumes/USB", 50, 200);
    expect(findMountForPath([usb], "/Volumes/USB")).toBe(usb);
  });

  it("does NOT match when the path only shares a common prefix string", () => {
    // /Volumes/USB-Backup is NOT under /Volumes/USB
    const usb = drive("/Volumes/USB", 50, 200);
    expect(findMountForPath([usb], "/Volumes/USB-Backup/foo")).toBeNull();
  });

  it("returns null when no drive matches at all", () => {
    const usb = drive("/Volumes/USB", 50, 200);
    expect(findMountForPath([usb], "/Users/md/Documents")).toBeNull();
  });

  it("handles nested USB mounts (longest-prefix wins)", () => {
    const usb = drive("/Volumes/USB", 50, 200);
    const sub = drive("/Volumes/USB/Subvol", 25, 100);
    expect(findMountForPath([usb, sub], "/Volumes/USB/Subvol/file.txt")).toBe(sub);
    expect(findMountForPath([usb, sub], "/Volumes/USB/other.txt")).toBe(usb);
  });
});

describe("formatDriveSpace", () => {
  it("formats free space as a terse string", () => {
    const d = drive("/", 256 * 1024 * 1024, 1024 * 1024 * 1024);
    expect(formatDriveSpace(d)).toMatch(/^256(\.0)? MB free$/);
  });

  it("returns null when total_bytes is 0 (non-accounting mount)", () => {
    expect(formatDriveSpace(drive("/", 0, 0))).toBeNull();
  });

  it("formats large drives in GB or TB", () => {
    const d = drive("/", 245 * 1024 * 1024 * 1024, 500 * 1024 * 1024 * 1024);
    const out = formatDriveSpace(d);
    expect(out).toMatch(/free$/);
    expect(out).toMatch(/(GB|GiB)/);
  });
});

describe("formatDriveSpaceDetail", () => {
  it("includes name, free, and total", () => {
    const d = drive("/", 100 * 1024 * 1024, 500 * 1024 * 1024, "Macintosh HD");
    const out = formatDriveSpaceDetail(d);
    expect(out).toContain("Macintosh HD");
    expect(out).toContain("free");
    expect(out).toContain("of");
  });

  it("returns null when total_bytes is 0", () => {
    expect(formatDriveSpaceDetail(drive("/", 0, 0, "Special"))).toBeNull();
  });
});

// ──────────────────────────────────────────────
// Iter 17: `removable` flag flows through DriveInfoLike. The sidebar
// Devices section needs the flag to decide whether to render the eject
// affordance; matching / formatting must stay shape-agnostic so adding
// the flag never changes status-bar behaviour (regression guard).
// ──────────────────────────────────────────────
describe("DriveInfoLike.removable (iter 17)", () => {
  it("findMountForPath ignores the removable flag", () => {
    const fixed: DriveInfoLike = {
      name: "Macintosh HD",
      mount_point: "/",
      free_bytes: 100,
      total_bytes: 1000,
      removable: false,
    };
    const usb: DriveInfoLike = {
      name: "USB Stick",
      mount_point: "/Volumes/USB",
      free_bytes: 50,
      total_bytes: 200,
      removable: true,
    };
    // Longest-prefix still wins regardless of removable.
    expect(findMountForPath([fixed, usb], "/Volumes/USB/photos")).toBe(usb);
    expect(findMountForPath([fixed, usb], "/Users/md")).toBe(fixed);
  });

  it("formatDriveSpace returns the same output with or without removable", () => {
    const without: DriveInfoLike = drive("/", 100, 1000);
    const withFlag: DriveInfoLike = { ...without, removable: true };
    expect(formatDriveSpace(withFlag)).toBe(formatDriveSpace(without));
  });

  it("formatDriveSpaceDetail returns the same output with or without removable", () => {
    const without: DriveInfoLike = drive("/", 100, 1000, "Macintosh HD");
    const withFlag: DriveInfoLike = { ...without, removable: true };
    expect(formatDriveSpaceDetail(withFlag)).toBe(
      formatDriveSpaceDetail(without),
    );
  });
});
