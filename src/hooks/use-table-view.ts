import { useState } from "react";
import { useUIStore, type TableView } from "@/stores/ui-store";

const DEFAULT_VIEW: TableView = { pageSize: 50, sort: null };

/** Page size and sort: remembered in the UI store when `prefsKey` is set,
 *  otherwise local to the component. */
export function useTableView(prefsKey: string | undefined, defaults?: Partial<TableView>): [TableView, (patch: Partial<TableView>) => void] {
  const base = { ...DEFAULT_VIEW, ...defaults };
  const stored = useUIStore((s) => (prefsKey ? s.tableViews[prefsKey] : undefined));
  const setStored = useUIStore((s) => s.setTableView);
  const [local, setLocal] = useState<TableView>(base);
  if (!prefsKey) return [local, (patch) => setLocal((v) => ({ ...v, ...patch }))];
  return [stored ?? base, (patch) => setStored(prefsKey, { ...(stored ?? base), ...patch })];
}
