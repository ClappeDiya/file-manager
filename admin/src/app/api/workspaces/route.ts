import { NextRequest, NextResponse } from "next/server";
import {
  audit,
  badRequest,
  isContext,
  readJson,
  requirePermission,
  requireStepUp,
} from "@/lib/session";
import {
  deliveryStatus,
  listDeliveredWorkspaces,
  saveWorkspace,
} from "@/lib/db/repos/provisioning";
import { orgErrorResponse } from "@/lib/route-helpers";
export async function GET(req: NextRequest) {
  const ctx = await requirePermission(req, "workspaces.read");
  if (!isContext(ctx)) return ctx;
  return NextResponse.json(
    {
      workspaces: listDeliveredWorkspaces(ctx.orgId).map((w) => ({
        ...w,
        delivery: deliveryStatus(ctx.orgId, w.id),
      })),
    },
    { headers: { "cache-control": "no-store" } },
  );
}
export async function POST(req: NextRequest) {
  const ctx = await requireStepUp(req, "workspaces.write");
  if (!isContext(ctx)) return ctx;
  const b = await readJson(req);
  if (!b) return badRequest("Invalid JSON");
  try {
    const workspace = saveWorkspace(ctx.orgId, b);
    audit(ctx, {
      eventType: "workspace.create",
      description: `Created workspace "${workspace.name}" with explicit desktop grants`,
      resourceType: "workspace",
      resourceId: workspace.id,
    });
    return NextResponse.json({ workspace }, { status: 201 });
  } catch (e) {
    return orgErrorResponse(e);
  }
}
