/**
 * op-error-toast — locks the contract that destructive operations
 * surface a structured toast on failure rather than silently
 * console.error-ing into the void.
 *
 * Covers:
 *   - the `what` line carries the operation name + " failed"
 *   - the `why` line extracts an Error.message when available
 *   - the `why` line falls back to the string repr for non-Error
 *     throwables (Tauri sometimes throws plain strings)
 *   - the `why` line uses a generic fallback for empty / null / unknown
 *   - the default `appDid` / `userAction` are populated
 *   - caller-supplied `appDid` / `userAction` override the defaults
 *   - console.error is preserved alongside the toast for dev visibility
 */
import { describe, it, expect, beforeEach, vi } from "vitest";
import { reportOperationFailure } from "@/lib/op-error-toast";
import { useUIStore } from "@/stores/ui-store";

beforeEach(() => {
  // Each test inspects the most-recently-added error. Reset the
  // error list so tests don't bleed into each other.
  useUIStore.setState({ structuredErrors: [] });
  // Quiet the spy so tests don't pollute the test runner output.
  vi.spyOn(console, "error").mockImplementation(() => {});
});

function lastError() {
  // ui-store prepends new errors to the front of the array
  // (`[newError, ...state.structuredErrors]`), so the most-recently-
  // added one is at index 0, not at .length - 1. Using [0] here
  // means a future test that fires multiple errors per case still
  // inspects the right one without rewriting the helper.
  const errors = useUIStore.getState().structuredErrors;
  return errors[0];
}

describe("reportOperationFailure", () => {
  it('appends " failed" to the operation name in the `what` field', () => {
    reportOperationFailure("Delete", new Error("EACCES"));
    expect(lastError().what).toBe("Delete failed");
  });

  it("extracts Error.message into the `why` field", () => {
    reportOperationFailure("Copy", new Error("permission denied"));
    expect(lastError().why).toBe("permission denied");
  });

  it("uses the string repr for plain-string throwables (Tauri pattern)", () => {
    reportOperationFailure("Move", "Source does not exist");
    expect(lastError().why).toBe("Source does not exist");
  });

  it("falls back to a generic message for empty Error.message", () => {
    reportOperationFailure("Rename", new Error(""));
    expect(lastError().why).toBe("The operation returned an error");
  });

  it("falls back to a generic message for empty string throwables", () => {
    reportOperationFailure("Duplicate", "");
    expect(lastError().why).toBe("The operation returned an error");
  });

  it("falls back to a generic message for non-string, non-Error throwables", () => {
    reportOperationFailure("Create folder", { weird: "object" });
    expect(lastError().why).toBe("The operation returned an error");
  });

  it("falls back to a generic message for null", () => {
    reportOperationFailure("Permanent delete", null);
    expect(lastError().why).toBe("The operation returned an error");
  });

  it("populates default appDid and userAction when no context is passed", () => {
    reportOperationFailure("Delete", new Error("boom"));
    const e = lastError();
    expect(e.appDid).toBe("Did not change anything on disk");
    expect(e.userAction).toMatch(/permission/);
  });

  it("honours a caller-supplied appDid override", () => {
    reportOperationFailure("Permanent delete", new Error("locked"), {
      appDid: "Files were NOT removed",
    });
    expect(lastError().appDid).toBe("Files were NOT removed");
  });

  it("honours a caller-supplied userAction override", () => {
    reportOperationFailure("Rename", new Error("exists"), {
      userAction: "Pick a different name",
    });
    expect(lastError().userAction).toBe("Pick a different name");
  });

  it("preserves console.error for dev-mode visibility", () => {
    const spy = vi.spyOn(console, "error");
    reportOperationFailure("Copy", new Error("EPERM"));
    expect(spy).toHaveBeenCalledWith("Copy failed:", expect.any(Error));
  });
});
