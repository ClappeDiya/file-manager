import { cn } from "@ufop/ui-components";
import type { SpaceStatus } from "@/stores/spaces-store";

interface SpaceStatusDotProps {
  status: SpaceStatus | undefined;
  className?: string;
}

const STATUS_CONFIG: Record<
  string,
  { color: string; label: string; pulse?: boolean }
> = {
  local_only: { color: "bg-zinc-400", label: "Local only" },
  offline: { color: "bg-red-500", label: "Offline" },
  connected: { color: "bg-green-500", label: "Connected" },
  syncing: { color: "bg-blue-500", label: "Syncing...", pulse: true },
  synced: { color: "bg-green-500", label: "Synced" },
  conflicts: { color: "bg-amber-500", label: "Conflicts" },
  error: { color: "bg-red-500", label: "Error" },
};

export function SpaceStatusDot({ status, className }: SpaceStatusDotProps) {
  const type = status?.type ?? "local_only";
  const config = STATUS_CONFIG[type] ?? STATUS_CONFIG.local_only;

  let label = config.label;
  if (type === "synced" && "last_sync_at" in (status ?? {})) {
    label = `Synced`;
  }
  if (type === "conflicts" && "count" in (status ?? {})) {
    const count = (status as { type: "conflicts"; count: number }).count;
    label = `${count} conflict${count !== 1 ? "s" : ""}`;
  }
  if (type === "error" && "message" in (status ?? {})) {
    label = (status as { type: "error"; message: string }).message;
  }

  return (
    <span
      className={cn(
        "inline-block h-2 w-2 shrink-0 rounded-full",
        config.color,
        config.pulse && "animate-pulse",
        className,
      )}
      title={label}
      aria-label={label}
    />
  );
}
