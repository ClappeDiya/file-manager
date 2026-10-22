import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, isContext, readJson, requirePermission } from '@/lib/session';
import { allTags, setTagColor, tagColors } from '@/lib/db/repos/machines';
import { OrgError } from '@/lib/db/repos/orgs';

/** GET /api/fleet/tags — tags in use with machine counts and colors. */
export async function GET(request: NextRequest) {
  const ctx = await requirePermission(request, 'fleet.read');
  if (!isContext(ctx)) return ctx;
  const colors = tagColors(ctx.orgId);
  return NextResponse.json({ tags: allTags([ctx.orgId]).map((t) => ({ ...t, color: colors[t.tag] ?? null })) });
}

/** PUT /api/fleet/tags {tag, color: "#rrggbb" | null} */
export async function PUT(request: NextRequest) {
  const ctx = await requirePermission(request, 'fleet.write');
  if (!isContext(ctx)) return ctx;
  const b = await readJson<{ tag?: string; color?: string | null }>(request);
  if (!b?.tag) return badRequest('tag is required');
  try {
    setTagColor(ctx.orgId, b.tag, b.color ?? null);
  } catch (e) {
    if (e instanceof OrgError) return badRequest(e.message, e.status);
    throw e;
  }
  audit(ctx, { eventType: 'fleet.tag_color', description: `Set color of tag "${b.tag}"`, metadata: { color: b.color } });
  return NextResponse.json({ colors: tagColors(ctx.orgId) });
}
