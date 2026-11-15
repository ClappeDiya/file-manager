import { useEffect, useState } from "react";
import { tauriInvokeSafe } from "@/hooks/use-tauri";
import type { FilterGroupInfo, FilterRule } from "@/stores/backup-store";


/** The groups for the "Group" kind, from the shared engine. */
export function useFilterGroups(): FilterGroupInfo[] {
  const [groups, setGroups] = useState<FilterGroupInfo[]>([]);
  useEffect(() => {
    let live = true;
    void tauriInvokeSafe<FilterGroupInfo[]>("get_filter_groups", undefined, []).then((g) => live && setGroups(Array.isArray(g) ? g : []));
    return () => {
      live = false;
    };
  }, []);
  return groups;
}


/** Rules not already in `rules`, appended in order. */
export function appendRules(rules: FilterRule[], extra: FilterRule[]): FilterRule[] {
  const key = (r: FilterRule) => `${r.action}|${r.kind}|${r.pattern}`;
  const have = new Set(rules.map(key));
  return [...rules, ...extra.filter((r) => !have.has(key(r)))];
}
