import { useEffect } from "react";
import { useNotifyStore } from "@/stores/backup-notify";


/** The saved profiles and which of the `chosen` ids no longer exist (shared by every picker). */
export function useProfileChoices(chosen: string[]) {
  const { profiles, loadProfiles } = useNotifyStore();
  useEffect(() => {
    void loadProfiles();
  }, [loadProfiles]);
  const missing = chosen.filter((id) => !profiles.some((p) => p.id === id));
  return { profiles, missing };
}
