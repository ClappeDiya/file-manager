const UNITS_SI = ["B", "KB", "MB", "GB", "TB"];
const UNITS_BINARY = ["B", "KiB", "MiB", "GiB", "TiB"];

export interface FormatBytesOptions {
  decimals?: number;
  binary?: boolean;
}

export function formatBytes(
  bytes: number,
  options?: FormatBytesOptions,
): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const decimals = options?.decimals ?? 1;
  const units = options?.binary ? UNITS_BINARY : UNITS_SI;
  const k = 1024;
  const i = Math.min(
    Math.floor(Math.log(bytes) / Math.log(k)),
    units.length - 1,
  );
  const value = bytes / Math.pow(k, i);
  const fixed = i === 0 ? 0 : decimals;
  return `${value.toFixed(fixed)} ${units[i]}`;
}

/**
 * Format a byte count as a tooltip/headline suffix: returns `"{separator}{formatted}"`
 * when bytes is a positive finite number, otherwise returns an empty string.
 *
 * Centralises the suppression rule the ledger UIs share — first-run
 * sentinel (`null`), older backend payloads (`undefined`), and
 * metadata-only windows (`0`) all collapse to "" so renames and folder
 * creates never surface a noisy `0 B` next to the op count.
 */
export function formatBytesSuffix(
  bytes: number | null | undefined,
  separator: string = ", ",
): string {
  if (bytes === null || bytes === undefined || bytes <= 0) return "";
  return `${separator}${formatBytes(bytes)}`;
}
