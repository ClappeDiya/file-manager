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
  listConnectors,
  saveConnector,
} from "@/lib/db/repos/provisioning";
import { orgErrorResponse } from "@/lib/route-helpers";
export async function GET(request: NextRequest) {
  const ctx = await requirePermission(request, "connectors.read");
  if (!isContext(ctx)) return ctx;
  return NextResponse.json(
    {
      connectors: listConnectors(ctx.orgId).map((c) => ({
        ...c,
        delivery: deliveryStatus(ctx.orgId, `connector-${c.id}`),
      })),
    },
    { headers: { "cache-control": "no-store" } },
  );
}
export async function POST(request: NextRequest) {
  const ctx = await requireStepUp(request, "connectors.write");
  if (!isContext(ctx)) return ctx;
  const body = await readJson(request);
  if (!body) return badRequest("Invalid JSON");
  try {
    const connector = saveConnector(ctx.orgId, body);
    audit(ctx, {
      eventType: "connector.create",
      description: `Created connector "${connector.name}"`,
      resourceType: "connector",
      resourceId: connector.id,
    });
    return NextResponse.json({ connector }, { status: 201 });
  } catch (e) {
    return orgErrorResponse(e);
  }
}
