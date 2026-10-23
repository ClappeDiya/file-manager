const UNITS_SI = ["B", "KB", "MB", "GB", "TB"];
const UNITS_BINARY = ["B", "KiB", "MiB", "GiB", "TiB"];

/**
 * Size units: "decimal" (1 KB = 1,000 bytes, like macOS and most storage
 * vendors) or "binary" (1 KiB = 1,024 bytes).
 */
export type ByteUnits = "binary" | "decimal";

/** The app-wide default (Settings → Appearance → Size units, K123). The UI
 *  store sets it when the setting changes or is restored, so every
 *  `formatBytes` caller follows the setting without passing it. */
let defaultUnits: ByteUnits = "decimal";

export function setDefaultByteUnits(units: ByteUnits): void {
  defaultUnits = units === "binary" ? "binary" : "decimal";
}

export function getDefaultByteUnits(): ByteUnits {
  return defaultUnits;
}

export interface FormatBytesOptions {
  decimals?: number;
  /** true = binary (KiB), false = decimal (KB); wins over `units`. */
  binary?: boolean;
  /** Units for this call; default: the app-wide setting. */
  units?: ByteUnits;
}

/** A byte count scaled to its largest unit: the value and the unit label. */
export function scaleBytes(bytes: number, options?: Pick<FormatBytesOptions, "binary" | "units">): { value: number; unit: string } {
  const binary = options?.binary ?? (options?.units ?? defaultUnits) === "binary";
  const units = binary ? UNITS_BINARY : UNITS_SI;
  if (!Number.isFinite(bytes) || bytes <= 0) return { value: 0, unit: units[0] };
  const k = binary ? 1024 : 1000;
  const i = Math.max(0, Math.min(Math.floor(Math.log(bytes) / Math.log(k)), units.length - 1));
  return { value: bytes / Math.pow(k, i), unit: units[i] };
}

export function formatBytes(
  bytes: number,
  options?: FormatBytesOptions,
): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const decimals = options?.decimals ?? 1;
  const { value, unit } = scaleBytes(bytes, options);
  const fixed = unit === "B" ? 0 : decimals;
  return `${value.toFixed(fixed)} ${unit}`;
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
