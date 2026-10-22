/**
 * Machine/backup access scoping. Roles that manage the fleet see every machine
 * in the organization; the `user` role (machine owners) sees only machines its
 * groups grant — directly, or by tag.
 */
import type { AuthContext } from './session';
import { userGrants } from './db/repos/groups';
import { listMachines, type Machine } from './db/repos/machines';

/** `undefined` = no restriction; otherwise the machine ids the user may see. */
export function visibleMachineIds(ctx: AuthContext): string[] | undefined {
  if (ctx.role !== 'user') return undefined;
  const grants = userGrants(ctx.user.id, ctx.orgId);
  const ids = new Set(grants.filter((g) => g.resourceType === 'machine').map((g) => g.resourceId));
  for (const g of grants.filter((x) => x.resourceType === 'backup')) ids.add(g.resourceId.split('/')[0]);
  const tags = grants.filter((g) => g.resourceType === 'tag').map((g) => g.resourceId);
  if (tags.length) for (const m of listMachines({ orgIds: [ctx.orgId] })) if (m.tags.some((t) => tags.includes(t))) ids.add(m.id);
  return [...ids];
}

export function canSeeMachine(ctx: AuthContext, m: Machine | null): m is Machine {
  if (!m || m.orgId !== ctx.orgId) return false;
  const ids = visibleMachineIds(ctx);
  return !ids || ids.includes(m.id);
}
