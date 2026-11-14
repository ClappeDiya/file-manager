import { formatBytes } from "@/lib/format-bytes";
import type { RemoteQuotaInfo } from "@/lib/location-provider";

/** Status-bar text of a connection's storage: "X free of Y" (or "X used"). */
export function formatQuota(q: RemoteQuotaInfo): { short: string; detail: string } | null {
  const parts: string[] = [];
  if (q.used != null) parts.push(`${formatBytes(q.used)} used`);
  if (q.free != null) parts.push(`${formatBytes(q.free)} free`);
  if (q.total != null) parts.push(`${formatBytes(q.total)} total`);
  if (q.trashed != null && q.trashed > 0) parts.push(`${formatBytes(q.trashed)} in trash`);
  if (parts.length === 0) return null;
  const short =
    q.free != null && q.total != null
      ? `${formatBytes(q.free)} free of ${formatBytes(q.total)}`
      : q.free != null
        ? `${formatBytes(q.free)} free`
        : q.used != null
          ? `${formatBytes(q.used)} used`
          : `${formatBytes(q.total ?? 0)} total`;
  return { short, detail: `Connection storage: ${parts.join(", ")}` };
}
