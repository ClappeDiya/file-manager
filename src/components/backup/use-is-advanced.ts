import { selectBackupAdvanced, useUIStore } from "@/stores/ui-store";

/** True when the Backup screens should show their advanced sections. */
export function useIsAdvanced(): boolean {
  return useUIStore(selectBackupAdvanced);
}
