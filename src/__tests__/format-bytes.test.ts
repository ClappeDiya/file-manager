import { describe, it, expect } from "vitest";
import { formatBytes, formatBytesSuffix } from "@/lib/format-bytes";

describe("formatBytes", () => {
  it("returns '0 B' for zero", () => {
    expect(formatBytes(0)).toBe("0 B");
  });

  it("returns '0 B' for negative values", () => {
    expect(formatBytes(-100)).toBe("0 B");
  });

  it("returns '0 B' for NaN", () => {
    expect(formatBytes(NaN)).toBe("0 B");
  });

  it("returns '0 B' for Infinity", () => {
    expect(formatBytes(Infinity)).toBe("0 B");
  });

  it("formats bytes below 1 KB without decimals", () => {
    expect(formatBytes(500)).toBe("500 B");
  });

  it("formats kilobytes with 1 decimal", () => {
    expect(formatBytes(1536)).toBe("1.5 KB");
  });

  it("formats megabytes with 1 decimal", () => {
    expect(formatBytes(1048576)).toBe("1.0 MB");
  });

  it("formats gigabytes with 1 decimal", () => {
    expect(formatBytes(1073741824)).toBe("1.0 GB");
  });

  it("formats terabytes with 1 decimal", () => {
    expect(formatBytes(1099511627776)).toBe("1.0 TB");
  });

  it("clamps to TB for very large values", () => {
    const petabyte = 1024 * 1099511627776;
    expect(formatBytes(petabyte)).toContain("TB");
  });

  it("respects custom decimals option", () => {
    expect(formatBytes(1536, { decimals: 2 })).toBe("1.50 KB");
  });

  it("respects zero decimals option", () => {
    expect(formatBytes(1536, { decimals: 0 })).toBe("2 KB");
  });

  it("uses binary units when binary option is true", () => {
    expect(formatBytes(1048576, { binary: true })).toBe("1.0 MiB");
  });

  it("uses binary unit labels at every tier", () => {
    expect(formatBytes(1024, { binary: true })).toBe("1.0 KiB");
    expect(formatBytes(1073741824, { binary: true })).toBe("1.0 GiB");
    expect(formatBytes(1099511627776, { binary: true })).toBe("1.0 TiB");
  });

  it("combines binary and custom decimals", () => {
    expect(formatBytes(1536, { binary: true, decimals: 2 })).toBe("1.50 KiB");
  });

  it("handles exact 1 KB boundary", () => {
    expect(formatBytes(1024)).toBe("1.0 KB");
  });

  it("handles exact 1 byte", () => {
    expect(formatBytes(1)).toBe("1 B");
  });

  it("defaults to SI units when no options provided", () => {
    const result = formatBytes(2048);
    expect(result).toBe("2.0 KB");
    expect(result).not.toContain("KiB");
  });
});

describe("formatBytesSuffix", () => {
  it("returns empty string for null (first-run sentinel)", () => {
    expect(formatBytesSuffix(null)).toBe("");
  });

  it("returns empty string for undefined (older backend payload)", () => {
    expect(formatBytesSuffix(undefined)).toBe("");
  });

  it("returns empty string for zero (metadata-only window)", () => {
    expect(formatBytesSuffix(0)).toBe("");
  });

  it("returns empty string for negative values", () => {
    expect(formatBytesSuffix(-1)).toBe("");
  });

  it("prepends default separator ', ' for positive values", () => {
    expect(formatBytesSuffix(1536)).toBe(", 1.5 KB");
  });

  it("respects a custom separator", () => {
    expect(formatBytesSuffix(1_048_576, " · ")).toBe(" · 1.0 MB");
  });

  it("uses formatBytes for the numeric portion", () => {
    expect(formatBytesSuffix(1024)).toBe(", 1.0 KB");
    expect(formatBytesSuffix(1_073_741_824)).toBe(", 1.0 GB");
  });
});
