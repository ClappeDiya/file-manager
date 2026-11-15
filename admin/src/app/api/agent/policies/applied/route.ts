import { recordWorkspaceReceipt } from "@/lib/db/repos/provisioning";
import { z } from "zod";
import { NextRequest, NextResponse } from "next/server";
import { readJson } from "@/lib/session";
import { agentAuth, isMachine } from "@/lib/agent-auth";
import { setAppliedPolicies } from "@/lib/db/repos/machines";
import { appendAudit } from "@/lib/db/repos/audit";

/** POST /api/agent/policies/applied {applied: {policyId: version}, errors: {policyId: message}} */
export async function POST(request: NextRequest) {
  const m = await agentAuth(request);
  if (!isMachine(m)) return m;
  const parsed = z
    .object({
      applied: z.record(z.string(), z.unknown()).default({}),
      errors: z.record(z.string(), z.string().max(2000)).default({}),
    })
    .strict()
    .safeParse(await readJson(request));
  if (
    !parsed.success ||
    Object.keys(parsed.data.applied).length > 2000 ||
    Object.keys(parsed.data.errors).length > 2000
  )
    return NextResponse.json(
      { error: "Invalid acknowledgement" },
      { status: 400 },
    );
  const b = parsed.data;
  const applied = Object.fromEntries(
    Object.entries(b.applied).flatMap(([k, v]) =>
      typeof v === "number" && Number.isSafeInteger(v) && v > 0
        ? [[k, v] as const]
        : [],
    ),
  );
  setAppliedPolicies(m.id, applied);
  recordWorkspaceReceipt(m, b?.errors ?? {});
  for (const [pid, msg] of Object.entries(b?.errors ?? {}))
    appendAudit({
      eventType: "policy.apply_failed",
      severity: "warning",
      orgId: m.orgId,
      description: `"${m.name}" could not apply policy ${pid}: ${String(msg).slice(0, 300)}`,
      resourceType: "machine",
      resourceId: m.id,
    });
  return NextResponse.json({ ok: true });
}
