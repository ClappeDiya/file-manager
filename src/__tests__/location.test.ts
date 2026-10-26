import { describe, it, expect, beforeEach } from "vitest";
import {
  clearRemoteNames,
  formatAddress,
  isRemotePath,
  isTopFolder,
  joinLocation,
  locationLabel,
  locationRoot,
  locationSegments,
  normalizeRemoteFolder,
  parentLocation,
  parseAddressInput,
  parseLocation,
  rememberRemoteNames,
  remoteLocation,
  sameFolder,
  sameLocationRoot,
  splitAddressForCompletion,
} from "@/lib/location";

const ID = "6f1c2b0a-1111-4222-8333-944455556666";

describe("pane locations", () => {
  beforeEach(() => clearRemoteNames());

  it("leaves every local path unchanged", () => {
    for (const p of ["/", "/home/me", "C:\\Users\\me", "\\\\nas\\share"]) {
      expect(parseLocation(p)).toEqual({ kind: "local", path: p });
      expect(isRemotePath(p)).toBe(false);
    }
    expect(parentLocation("/home/me")).toBe("/home");
    expect(parentLocation("/home")).toBe("/");
    expect(joinLocation("/", "a")).toBe("/a");
    expect(joinLocation("/home", "a")).toBe("/home/a");
    expect(joinLocation("C:\\Users", "a")).toBe("C:\\Users\\a");
    expect(locationRoot("/home/me")).toBe("/");
  });

  it("parses and builds conn:// locations", () => {
    const p = remoteLocation(ID, "/drive/root:/Docs");
    expect(p).toBe(`conn://${ID}/drive/root:/Docs`);
    expect(parseLocation(p)).toEqual({ kind: "remote", connectionId: ID, path: "/drive/root:/Docs" });
    expect(parseLocation(`conn://${ID}`)).toEqual({ kind: "remote", connectionId: ID, path: "/" });
    expect(remoteLocation(ID, "bucket/x")).toBe(`conn://${ID}/bucket/x`);
    expect(joinLocation(remoteLocation(ID, "/"), "a.txt")).toBe(`conn://${ID}/a.txt`);
    expect(joinLocation(p, "a.txt")).toBe(`conn://${ID}/drive/root:/Docs/a.txt`);
    expect(parentLocation(`conn://${ID}/a/b`)).toBe(`conn://${ID}/a`);
    expect(parentLocation(`conn://${ID}/a`)).toBe(`conn://${ID}/`);
    expect(parentLocation(`conn://${ID}/`)).toBe(`conn://${ID}/`);
    expect(locationRoot(p)).toBe(`conn://${ID}/`);
  });

  it("knows which locations share a root", () => {
    const other = "0b5d8c3e-aaaa-4bbb-8ccc-dddddddddddd";
    expect(sameLocationRoot("/a", "/b")).toBe(true);
    expect(sameLocationRoot(`conn://${ID}/a`, `conn://${ID}/b`)).toBe(true);
    expect(sameLocationRoot(`conn://${ID}/a`, `conn://${other}/a`)).toBe(false);
    expect(sameLocationRoot("/a", `conn://${ID}/a`)).toBe(false);
  });

  it("treats OneDrive's Graph root prefixes as the connection root when comparing folders", () => {
    // Regression: `/drive/root:/a.txt` listed in a pane opened at `/` has
    // the parent `/drive/root:`, which did not equal `/`, so cutting and
    // pasting it into its own folder renamed it to `a (copy).txt`.
    expect(normalizeRemoteFolder("/drive/root:")).toBe("/");
    expect(normalizeRemoteFolder("/drive/root:/Docs/")).toBe("/Docs");
    expect(normalizeRemoteFolder("/drives/LIB1/root:/Q3")).toBe("/Q3");
    expect(normalizeRemoteFolder("/drive/rootx:/Docs")).toBe("/drive/rootx:/Docs");
    expect(sameFolder(parentLocation(`conn://${ID}/drive/root:/a.txt`), `conn://${ID}/`)).toBe(true);
    expect(sameFolder(`conn://${ID}/drives/LIB1/root:/Q3`, `conn://${ID}/Q3/`)).toBe(true);
    expect(sameFolder(`conn://${ID}/drive/root:/Docs`, `conn://${ID}/`)).toBe(false);
    expect(sameFolder(`conn://${ID}/`, "conn://0b5d8c3e-aaaa-4bbb-8ccc-dddddddddddd/")).toBe(false);
    expect(sameFolder("/home", `conn://${ID}/home`)).toBe(false);
    expect(sameFolder("/home", "/home")).toBe(true);
  });

  it("labels and breadcrumbs use listed names (Drive paths are ids)", () => {
    rememberRemoteNames([
      { path: `conn://${ID}/f1`, name: "Reports" },
      { path: `conn://${ID}/f1/f2`, name: "2026" },
    ]);
    expect(locationLabel(`conn://${ID}/f1/f2`)).toBe("2026");
    expect(locationLabel(`conn://${ID}/`, () => "My Drive")).toBe("My Drive");
    expect(locationLabel("/")).toBe("Root");
    expect(locationLabel("/home/me")).toBe("me");
    expect(locationSegments(`conn://${ID}/f1/f2`)).toEqual([
      { label: "Reports", path: `conn://${ID}/f1` },
      { label: "2026", path: `conn://${ID}/f1/f2` },
    ]);
    // OneDrive's Graph prefix is not a folder of its own.
    expect(locationSegments(`conn://${ID}/drive/root:/Docs`).map((s) => s.label)).toEqual(["Docs"]);
    expect(locationSegments("/home/me")).toEqual([
      { label: "home", path: "/home" },
      { label: "me", path: "/home/me" },
    ]);
  });

  it("parses typed addresses", () => {
    const conns = [
      { id: ID, name: "Work Drive" },
      { id: "0b5d8c3e-aaaa-4bbb-8ccc-dddddddddddd", name: "D" },
    ];
    expect(parseAddressInput("/home/me/", conns)).toEqual({ ok: true, path: "/home/me" });
    expect(parseAddressInput("/", conns)).toEqual({ ok: true, path: "/" });
    expect(parseAddressInput("Work Drive:/Docs", conns)).toEqual({ ok: true, path: `conn://${ID}/Docs` });
    expect(parseAddressInput("work drive:Docs", conns)).toEqual({ ok: true, path: `conn://${ID}/Docs` });
    expect(parseAddressInput("Work Drive:", conns)).toEqual({ ok: true, path: `conn://${ID}/` });
    expect(parseAddressInput(`conn://${ID}/x`, conns)).toEqual({ ok: true, path: `conn://${ID}/x` });
    expect(parseAddressInput("conn://00000000-0000-4000-8000-000000000000/x", conns).ok).toBe(false);
    // Windows drives stay local unless a connection is named exactly that.
    expect(parseAddressInput("C:\\Users", conns)).toEqual({ ok: true, path: "C:\\Users" });
    expect(parseAddressInput("C:", conns)).toEqual({ ok: true, path: "C:\\" });
    expect(parseAddressInput("D:/x", conns)).toEqual({ ok: true, path: "conn://0b5d8c3e-aaaa-4bbb-8ccc-dddddddddddd/x" });
    expect(parseAddressInput("Nope:/x", conns)).toEqual({ ok: false, error: 'No saved connection is named "Nope"' });
    expect(parseAddressInput("relative/path", conns).ok).toBe(false);
    expect(parseAddressInput("  ", conns).ok).toBe(false);
  });

  it("splits typed text for completion and formats addresses", () => {
    expect(splitAddressForCompletion("/home/us")).toEqual({ parent: "/home", partial: "us" });
    expect(splitAddressForCompletion("/ho")).toEqual({ parent: "/", partial: "ho" });
    expect(splitAddressForCompletion("Drive:/Docs/Re")).toEqual({ parent: "Drive:/Docs", partial: "Re" });
    expect(splitAddressForCompletion("Drive:/Re")).toEqual({ parent: "Drive:/", partial: "Re" });
    expect(formatAddress(`conn://${ID}/Docs`, () => "Work Drive")).toBe("Work Drive:/Docs");
    expect(formatAddress("/home")).toBe("/home");
  });

  // Regression: the parent of `C:\Users` was `C:` (the drive's current
  // folder, not its root), and the parent of OneDrive's `/drive/root:` was
  // `/drive`, a path the connector cannot list.
  it("goes up to drive, share and connection roots and stops there", () => {
    expect(parentLocation("C:\\Users")).toBe("C:\\");
    expect(parentLocation("C:\\Users\\me")).toBe("C:\\Users");
    expect(parentLocation("C:\\")).toBe("C:\\");
    expect(parentLocation("C:/Users")).toBe("C:/");
    expect(parentLocation("\\\\nas\\share\\docs")).toBe("\\\\nas\\share");
    expect(parentLocation("\\\\nas\\share")).toBe("\\\\nas\\share");
    expect(parentLocation("/")).toBe("/");
    expect(parentLocation(`conn://${ID}/drive/root:/Docs`)).toBe(`conn://${ID}/drive/root:`);
    expect(parentLocation(`conn://${ID}/drive/root:`)).toBe(`conn://${ID}/drive/root:`);
    expect(parentLocation(`conn://${ID}/drives/b!x/root:`)).toBe(`conn://${ID}/drives/b!x/root:`);
  });

  it("knows the top folder of a location", () => {
    expect(isTopFolder("/")).toBe(true);
    expect(isTopFolder("/home")).toBe(false);
    expect(isTopFolder("C:\\")).toBe(true);
    expect(isTopFolder("D:")).toBe(true);
    expect(isTopFolder("C:\\Users")).toBe(false);
    expect(isTopFolder("\\\\nas\\share")).toBe(true);
    expect(isTopFolder(`conn://${ID}/`)).toBe(true);
    expect(isTopFolder(`conn://${ID}`)).toBe(true);
    expect(isTopFolder(`conn://${ID}/drive/root:`)).toBe(true);
    expect(isTopFolder(`conn://${ID}/a`)).toBe(false);
  });

  // Regression (WS-Z2): a local path with ":" in a name was read as
  // "<connection name>:path" and refused.
  it("takes full local paths with a colon in a name as local", () => {
    const conns = [{ id: ID, name: "Work Drive" }];
    expect(parseAddressInput("/home/me/notes:2024", conns)).toEqual({ ok: true, path: "/home/me/notes:2024" });
    expect(parseAddressInput("/Volumes/Backup 10:30/", conns)).toEqual({ ok: true, path: "/Volumes/Backup 10:30" });
    expect(parseAddressInput("\\\\nas\\share\\a:b", conns)).toEqual({ ok: true, path: "\\\\nas\\share\\a:b" });
    expect(parseAddressInput("Work Drive:/Docs", conns)).toEqual({ ok: true, path: `conn://${ID}/Docs` });
  });
});
