/** Ransomware-hardening helpers shared by the job editor (`backup-hardening.tsx`). */
import type { HardeningReport } from "@/stores/backup-store";

/** Where the guide is published (the repository's docs folder). */
export const DOCS_BASE =
  "https://github.com/ClappeDiya/file-manager/blob/main/";

export function protectionTone(r: HardeningReport): string {
  if (r.checks.some((c) => c.status === "fail"))
    return "bg-red-100 text-red-800 dark:bg-red-900/40 dark:text-red-200";
  if (r.passed === r.total)
    return "bg-green-100 text-green-800 dark:bg-green-900/40 dark:text-green-200";
  return "bg-amber-100 text-amber-800 dark:bg-amber-900/40 dark:text-amber-200";
}
