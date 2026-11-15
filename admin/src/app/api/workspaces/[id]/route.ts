import { NextRequest, NextResponse } from "next/server";
import {
  audit,
  badRequest,
  isContext,
  readJson,
  requireStepUp,
} from "@/lib/session";
import { deleteWorkspace, saveWorkspace } from "@/lib/db/repos/provisioning";
import { orgErrorResponse } from "@/lib/route-helpers";
type Params = { params: Promise<{ id: string }> };
export async function PATCH(req: NextRequest, p: Params) {
  const ctx = await requireStepUp(req, "workspaces.write");
  if (!isContext(ctx)) return ctx;
  const b = await readJson(req);
  if (!b) return badRequest("Invalid JSON");
  try {
    const workspace = saveWorkspace(ctx.orgId, b, (await p.params).id);
    audit(ctx, {
      eventType: "workspace.update",
      description: `Updated workspace "${workspace.name}" and its desktop grants`,
      resourceType: "workspace",
      resourceId: workspace.id,
    });
    return NextResponse.json({ workspace });
  } catch (e) {
    return orgErrorResponse(e);
  }
}
export async function DELETE(req: NextRequest, p: Params) {
  const ctx = await requireStepUp(req, "workspaces.write");
  if (!isContext(ctx)) return ctx;
  const { id } = await p.params;
  if (!deleteWorkspace(ctx.orgId, id))
    return badRequest("Workspace not found", 404);
  audit(ctx, {
    eventType: "workspace.delete",
    description:
      "Removed workspace; desktops remove its managed connections on their next successful sync",
    resourceType: "workspace",
    resourceId: id,
  });
  return NextResponse.json({ ok: true });
}
