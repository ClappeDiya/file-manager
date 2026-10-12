import { useEffect, useRef } from "react";
import { useFileManagerStore } from "@/stores/file-manager-store";

/**
 * A filter typed for one folder must not follow the user into another — it
 * would hide every entry of the new listing ("0 of N"). The first render is
 * skipped so a restored filter survives app launch.
 */
export function useClearFilterOnNavigate(paneIndex: 0 | 1, currentPath: string) {
  const prev = useRef(currentPath);
  useEffect(() => {
    if (prev.current === currentPath) return;
    prev.current = currentPath;
    const st = useFileManagerStore.getState();
    if (st.panes[paneIndex]?.filterText) st.setFilterText(paneIndex, "");
  }, [currentPath, paneIndex]);
}
