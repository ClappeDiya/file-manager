/**
 * Tests for the file-action keyboard resolver.
 *
 * The command palette and the keyboard cheat sheet have always
 * advertised `F2` (Rename) and `⌘/Ctrl+Backspace` (Move to Trash),
 * but no key handler ever made those keystrokes do anything — the
 * shortcut hints were a promise the app silently broke. This resolver
 * is the missing piece: a pure predicate that maps a keystroke to a
 * file action, with the safety logic that a destructive shortcut
 * demands.
 *
 * The branches that matter most are the *negative* ones — the
 * keystroke must NOT trigger a delete or rename while the user is
 * typing in a search / filter / rename / AI field, and bare
 * Backspace (the most-pressed key in any text field) must never
 * trash files. Those are exactly the cases that turn a convenience
 * shortcut into a data-loss bug, so they are tested first.
 *
 * Pure unit tests — no React, no DOM, no Zustand. The resolver takes
 * a structural subset of `KeyboardEvent` plus a "is a text field
 * focused" boolean, so the real handler can pass the live event and
 * the tests can pass plain objects.
 */
import { describe, it, expect } from "vitest";
import { resolveFileActionKey } from "../lib/file-action-keys";

type Gesture = Parameters<typeof resolveFileActionKey>[0];

/** Build a keystroke with every modifier off unless overridden. */
function key(over: Partial<Gesture> & { key: string }): Gesture {
  return { metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, ...over };
}

describe("resolveFileActionKey — never hijacks typing", () => {
  it("returns null for F2 while a text field is focused", () => {
    expect(resolveFileActionKey(key({ key: "F2" }), true)).toBeNull();
  });

  it("returns null for Delete while a text field is focused", () => {
    expect(resolveFileActionKey(key({ key: "Delete" }), true)).toBeNull();
  });

  it("returns null for ⌘+Backspace while a text field is focused (preserves delete-to-line-start)", () => {
    expect(resolveFileActionKey(key({ key: "Backspace", metaKey: true }), true)).toBeNull();
  });
});

describe("resolveFileActionKey — Rename (F2)", () => {
  it("maps unmodified F2 to rename", () => {
    expect(resolveFileActionKey(key({ key: "F2" }), false)).toBe("rename");
  });

  it("does not map F2 with a modifier", () => {
    expect(resolveFileActionKey(key({ key: "F2", metaKey: true }), false)).toBeNull();
    expect(resolveFileActionKey(key({ key: "F2", shiftKey: true }), false)).toBeNull();
    expect(resolveFileActionKey(key({ key: "F2", altKey: true }), false)).toBeNull();
  });
});

describe("resolveFileActionKey — Move to Trash", () => {
  it("maps the bare Delete key to trash (Windows/Linux convention)", () => {
    expect(resolveFileActionKey(key({ key: "Delete" }), false)).toBe("trash");
  });

  it("maps ⌘+Backspace to trash (macOS convention, as advertised)", () => {
    expect(resolveFileActionKey(key({ key: "Backspace", metaKey: true }), false)).toBe("trash");
  });

  it("maps Ctrl+Backspace to trash (Windows/Linux convention)", () => {
    expect(resolveFileActionKey(key({ key: "Backspace", ctrlKey: true }), false)).toBe("trash");
  });

  it("does NOT trash on a bare Backspace (the everyday text-edit key)", () => {
    expect(resolveFileActionKey(key({ key: "Backspace" }), false)).toBeNull();
  });

  it("does NOT trash on Alt+Backspace (reserved for delete-word)", () => {
    expect(resolveFileActionKey(key({ key: "Backspace", altKey: true }), false)).toBeNull();
  });

  it("does NOT trash on Shift+Delete (permanent-delete is intentionally out of scope)", () => {
    expect(resolveFileActionKey(key({ key: "Delete", shiftKey: true }), false)).toBeNull();
  });

  it("does NOT trash on ⌘+Shift+Backspace", () => {
    expect(resolveFileActionKey(key({ key: "Backspace", metaKey: true, shiftKey: true }), false)).toBeNull();
  });
});

describe("resolveFileActionKey — unrelated keys", () => {
  it("returns null for ordinary keys", () => {
    expect(resolveFileActionKey(key({ key: "a" }), false)).toBeNull();
    expect(resolveFileActionKey(key({ key: "Enter" }), false)).toBeNull();
    expect(resolveFileActionKey(key({ key: "ArrowDown" }), false)).toBeNull();
  });
});
