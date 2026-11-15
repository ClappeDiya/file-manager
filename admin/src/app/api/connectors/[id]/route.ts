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
  deleteConnector,
  getConnector,
  saveConnector,
} from "@/lib/db/repos/provisioning";
import { orgErrorResponse } from "@/lib/route-helpers";
import { probeConnector } from "@/lib/connector-probe";
type Params = { params: Promise<{ id: string }> };
export async function GET(req: NextRequest, p: Params) {
  const ctx = await requirePermission(req, "connectors.read");
  if (!isContext(ctx)) return ctx;
  const c = getConnector(ctx.orgId, (await p.params).id);
  return c
    ? NextResponse.json({ connector: c })
    : badRequest("Connector not found", 404);
}
export async function PATCH(req: NextRequest, p: Params) {
  const ctx = await requireStepUp(req, "connectors.write");
  if (!isContext(ctx)) return ctx;
  const body = await readJson(req);
  if (!body) return badRequest("Invalid JSON");
  try {
    const c = saveConnector(ctx.orgId, body, (await p.params).id);
    audit(ctx, {
      eventType: "connector.update",
      description: `Updated connector "${c.name}"`,
      resourceType: "connector",
      resourceId: c.id,
    });
    return NextResponse.json({ connector: c });
  } catch (e) {
    return orgErrorResponse(e);
  }
}
export async function DELETE(req: NextRequest, p: Params) {
  const ctx = await requireStepUp(req, "connectors.write");
  if (!isContext(ctx)) return ctx;
  const { id } = await p.params;
  try {
    if (!deleteConnector(ctx.orgId, id))
      return badRequest("Connector not found", 404);
    audit(ctx, {
      eventType: "connector.delete",
      description: "Removed connector and direct desktop grants",
      resourceType: "connector",
      resourceId: id,
    });
    return NextResponse.json({ ok: true });
  } catch (e) {
    return orgErrorResponse(e);
  }
}
export async function POST(req: NextRequest, p: Params) {
  const ctx = await requireStepUp(req, "connectors.write");
  if (!isContext(ctx)) return ctx;
  const b = await readJson(req);
  if (b?.action !== "test") return badRequest("Unknown action");
  const c = getConnector(ctx.orgId, (await p.params).id);
  if (!c) return badRequest("Connector not found", 404);
  const result = await probeConnector(c);
  audit(ctx, {
    eventType: "connector.probe",
    description: `TCP reachability ${result.reachable ? "succeeded" : "failed"}; authentication was not tested`,
    resourceType: "connector",
    resourceId: c.id,
  });
  return NextResponse.json({ ...result, authenticationTested: false });
}
