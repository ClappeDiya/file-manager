import { describe, expect, it } from "vitest";
import { decodeBase64Bytes } from "@/lib/base64";

describe("decodeBase64Bytes", () => {
  it("round-trips arbitrary bytes, including a split multi-byte sequence", () => {
    const euro = new TextEncoder().encode("€"); // 3 bytes
    const half = euro.slice(0, 2);
    const b64 = btoa(String.fromCharCode(...half));
    expect(Array.from(decodeBase64Bytes(b64))).toEqual(Array.from(half));
  });
  it("handles empty input", () => {
    expect(decodeBase64Bytes("").length).toBe(0);
  });
});
