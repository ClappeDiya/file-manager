import { test } from "node:test";
import assert from "node:assert/strict";
import { mismatches } from "./check-tauri-resolved-versions.mjs";

test("rejects the SDK drift that prevented native packaging despite matching majors", () => {
  assert.equal(mismatches({tauri:"2.12.5", "tauri-plugin-dialog":"2.8.4"}, {"@tauri-apps/api":"2.10.1", "@tauri-apps/cli":"2.10.1", "@tauri-apps/plugin-dialog":"2.6.0"}).length, 3);
});
test("accepts independently released patches and rejects a missing Rust plugin", () => {
  assert.deepEqual(mismatches({tauri:"2.12.5", "tauri-plugin-dialog":"2.8.4"}, {"@tauri-apps/api":"2.12.4", "@tauri-apps/cli":"2.12.4", "@tauri-apps/plugin-dialog":"2.8.4"}), []);
  assert.equal(mismatches({tauri:"2.12.5"}, {"@tauri-apps/plugin-dialog":"2.8.4"}).length, 1);
});
