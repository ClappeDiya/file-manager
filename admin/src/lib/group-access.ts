/** Who may manage a group: holders of groups.write in its organization, or the group's owners. */
import type { AuthContext } from './session';
import { canIn } from './session';
import { getGroup, isGroupOwner, type Group } from './db/repos/groups';

export function manageableGroup(ctx: AuthContext, groupId: string): { group: Group; asOwner: boolean } | null {
  const g = getGroup(groupId);
  if (!g) return null;
  if (canIn(ctx, g.orgId, 'groups.write')) return { group: g, asOwner: false };
  if (isGroupOwner(g.id, ctx.user.id)) return { group: g, asOwner: true };
  return null;
}
