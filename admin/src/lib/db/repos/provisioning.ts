/** Organization-scoped saved connections and explicit desktop grants. No secrets in definitions. */
import { createHash, randomUUID } from "node:crypto";
import { z } from "zod";
import { isIP } from "node:net";
import { db, now, parseJson, type Row } from "../index";
import { OrgError } from "./orgs";
import { getMachine, type Machine } from "./machines";
import { listCredentials } from "./fleet-policies";
import { open } from "../../crypto";

export const CONNECTOR_PROTOCOLS = [
  "sftp",
  "ftp",
  "ftps",
  "webdav",
  "smb",
  "nfs",
  "s3",
  "azure_blob",
  "adls_gen2",
  "azure_files",
  "b2",
] as const;
const ids = z
  .array(z.string().min(1).max(80))
  .max(500)
  .transform((v) => [...new Set(v)]);
const definition = z
  .object({
    name: z.string().trim().min(1).max(120),
    protocol: z.enum(CONNECTOR_PROTOCOLS),
    host: z
      .string()
      .trim()
      .min(1)
      .max(253)
      .refine(
        (h) =>
          isIP(h.replace(/^\[|\]$/g, "")) !== 0 ||
          h
            .split(".")
            .every((label) =>
              /^[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?$/.test(label),
            ),
        "Enter a hostname or IP address without a URL, path or credentials",
      ),
    port: z.number().int().min(1).max(65535).nullable().default(null),
    username: z.string().trim().max(200).default(""),
    remotePath: z.string().min(1).max(2000).default("/"),
    credentialId: z.string().max(80).nullable().default(null),
    machineIds: ids.default([]),
  })
  .strict();
const workspace = z
  .object({
    name: z.string().trim().min(1).max(120),
    description: z.string().max(1000).default(""),
    connectorIds: ids,
    machineIds: ids,
  })
  .strict();
export type ConnectorInput = z.input<typeof definition>;
export type ConnectorDefinition = z.output<typeof definition> & {
  id: string;
  orgId: string;
  version: number;
  updatedAt: string;
};
export type WorkspaceInput = z.input<typeof workspace>;
export type DeliveredWorkspace = z.output<typeof workspace> & {
  id: string;
  orgId: string;
  version: number;
  updatedAt: string;
};
function validate<T>(schema: z.ZodType<T>, raw: unknown): T {
  const r = schema.safeParse(raw);
  if (!r.success)
    throw new OrgError(
      r.error.issues.map((i) => `${i.path.join(".")}: ${i.message}`).join("; "),
    );
  return r.data;
}
function machinesInOrg(orgId: string, machineIds: string[]) {
  for (const id of machineIds) {
    const m = getMachine(id);
    if (
      !m ||
      m.orgId !== orgId ||
      m.status !== "active" ||
      m.agentKind !== "desktop"
    )
      throw new OrgError(
        "Every delivery target must be an active desktop in this organization",
      );
  }
}
const connectorRow = (r: Row): ConnectorDefinition => ({
  ...parseJson<z.output<typeof definition>>(
    r.definition,
    {} as z.output<typeof definition>,
  ),
  id: String(r.id),
  orgId: String(r.org_id),
  version: Number(r.version),
  updatedAt: String(r.updated_at),
});
const workspaceRow = (r: Row): DeliveredWorkspace => ({
  ...parseJson<z.output<typeof workspace>>(
    r.definition,
    {} as z.output<typeof workspace>,
  ),
  id: String(r.id),
  orgId: String(r.org_id),
  version: Number(r.version),
  updatedAt: String(r.updated_at),
});
export function listConnectors(orgId: string) {
  return (
    db()
      .prepare(
        "SELECT * FROM managed_connectors WHERE org_id = ? ORDER BY name",
      )
      .all(orgId) as Row[]
  ).map(connectorRow);
}
export function getConnector(orgId: string, id: string) {
  return listConnectors(orgId).find((c) => c.id === id) ?? null;
}
export function saveConnector(
  orgId: string,
  raw: unknown,
  id?: string,
): ConnectorDefinition {
  const c = validate(definition, raw);
  machinesInOrg(orgId, c.machineIds);
  if (
    c.credentialId &&
    !listCredentials(orgId).some((x) => x.id === c.credentialId)
  )
    throw new OrgError("Credential does not exist in this organization");
  if (id && !getConnector(orgId, id))
    throw new OrgError("Connector not found", 404);
  const key = id ?? randomUUID();
  db()
    .prepare(
      `INSERT INTO managed_connectors (id, org_id, name, definition, version, updated_at) VALUES (?, ?, ?, ?, 1, ?)
    ON CONFLICT(id) DO UPDATE SET name=excluded.name, definition=excluded.definition, version=managed_connectors.version+1, updated_at=excluded.updated_at WHERE managed_connectors.org_id=excluded.org_id`,
    )
    .run(key, orgId, c.name, JSON.stringify(c), now());
  return getConnector(orgId, key)!;
}
export function deleteConnector(orgId: string, id: string) {
  if (listDeliveredWorkspaces(orgId).some((w) => w.connectorIds.includes(id)))
    throw new OrgError("Remove this connector from its workspaces first", 409);
  return (
    db()
      .prepare("DELETE FROM managed_connectors WHERE org_id=? AND id=?")
      .run(orgId, id).changes > 0
  );
}
export function listDeliveredWorkspaces(orgId: string) {
  return (
    db()
      .prepare(
        "SELECT * FROM delivered_workspaces WHERE org_id=? ORDER BY name",
      )
      .all(orgId) as Row[]
  ).map(workspaceRow);
}
export function saveWorkspace(
  orgId: string,
  raw: unknown,
  id?: string,
): DeliveredWorkspace {
  const w = validate(workspace, raw);
  machinesInOrg(orgId, w.machineIds);
  for (const cid of w.connectorIds)
    if (!getConnector(orgId, cid))
      throw new OrgError("Connector does not exist in this organization");
  if (id && !listDeliveredWorkspaces(orgId).some((w) => w.id === id))
    throw new OrgError("Workspace not found", 404);
  const key = id ?? randomUUID();
  db()
    .prepare(
      `INSERT INTO delivered_workspaces (id, org_id, name, definition, version, updated_at) VALUES (?, ?, ?, ?, 1, ?)
    ON CONFLICT(id) DO UPDATE SET name=excluded.name, definition=excluded.definition, version=delivered_workspaces.version+1, updated_at=excluded.updated_at WHERE delivered_workspaces.org_id=excluded.org_id`,
    )
    .run(key, orgId, w.name, JSON.stringify(w), now());
  return listDeliveredWorkspaces(orgId).find((w) => w.id === key)!;
}
export function deleteWorkspace(orgId: string, id: string) {
  return (
    db()
      .prepare("DELETE FROM delivered_workspaces WHERE org_id=? AND id=?")
      .run(orgId, id).changes > 0
  );
}
export interface WorkspaceDelivery {
  id: string;
  name: string;
  version: number;
  connections: {
    id: string;
    name: string;
    protocol: string;
    host: string;
    port: number | null;
    username: string;
    remotePath: string;
    credentialId: string | null;
  }[];
}
/** Only exact, current organization grants. No inheritance of secrets or machine access. */
export function workspaceDeliveries(m: Machine): WorkspaceDelivery[] {
  if (m.status !== "active" || m.agentKind !== "desktop") return [];
  const all = listConnectors(m.orgId);
  const grants = [
    ...listDeliveredWorkspaces(m.orgId).filter((w) =>
      w.machineIds.includes(m.id),
    ),
    ...all
      .filter((c) => c.machineIds.includes(m.id))
      .map((c) => ({
        id: `connector-${c.id}`,
        name: c.name,
        version: c.version,
        connectorIds: [c.id],
      })),
  ];
  return grants.map((w) => {
    const selected = all.filter((c) => w.connectorIds.includes(c.id));
    // Include credential rotations: the desktop must install new secret bytes before acknowledging this revision.
    const versions = selected
      .map(
        (c) =>
          `${c.id}:${c.version}:${listCredentials(m.orgId).find((x) => x.id === c.credentialId)?.version ?? 0}`,
      )
      .sort();
    const version =
      Number.parseInt(
        createHash("sha256")
          .update(`${w.version}|${versions.join(",")}`)
          .digest("hex")
          .slice(0, 12),
        16,
      ) + 1;
    return {
      id: w.id,
      name: w.name,
      version,
      connections: selected.map((c) => ({
        id: c.id,
        name: c.name,
        protocol: c.protocol,
        host: c.host,
        port: c.port,
        username: c.username,
        remotePath: c.remotePath,
        credentialId: c.credentialId ? `central:${c.credentialId}` : null,
      })),
    };
  });
}
export function workspaceCredentialVersions(m: Machine) {
  const ids = new Set(
    workspaceDeliveries(m).flatMap((w) =>
      w.connections.flatMap((c) =>
        c.credentialId ? [c.credentialId.slice(8)] : [],
      ),
    ),
  );
  return listCredentials(m.orgId).filter((c) => ids.has(c.id));
}
export function workspaceCredentials(m: Machine) {
  const ids = new Set(
    workspaceDeliveries(m).flatMap((w) =>
      w.connections.flatMap((c) =>
        c.credentialId ? [c.credentialId.slice(8)] : [],
      ),
    ),
  );
  return (
    db()
      .prepare("SELECT * FROM central_credentials WHERE org_id=?")
      .all(m.orgId) as Row[]
  )
    .filter((r) => ids.has(String(r.id)))
    .map((r) => ({
      id: String(r.id),
      credentialId: `central:${r.id}`,
      name: String(r.name),
      secret: open(String(r.ciphertext)),
      updatedAt: String(r.updated_at),
      version: Number(r.version),
    }));
}
export function deliveryStatus(orgId: string, id: string) {
  const ms = (
    db()
      .prepare(
        "SELECT id FROM machines WHERE org_id=? AND status='active' AND agent_kind='desktop'",
      )
      .all(orgId) as { id: string }[]
  ).map((r) => getMachine(r.id)!);
  return ms.flatMap((m) => {
    const w = workspaceDeliveries(m).find((w) => w.id === id);
    if (!w) return [];
    const receipt = db()
      .prepare("SELECT errors FROM workspace_receipts WHERE machine_id=?")
      .get(m.id) as Row | undefined;
    const error =
      parseJson<Record<string, string>>(receipt?.errors, {})[
        `workspace:${id}`
      ] ?? null;
    return [
      {
        machineId: m.id,
        name: m.name,
        version: w.version,
        status:
          m.appliedPolicies[`workspace:${id}`] === w.version
            ? "applied"
            : error
              ? "failed"
              : "pending",
        error,
      },
    ];
  });
}
export function recordWorkspaceReceipt(
  m: Machine,
  errors: Record<string, string>,
) {
  db()
    .prepare(
      "INSERT INTO workspace_receipts (machine_id, errors, updated_at) VALUES (?, ?, ?) ON CONFLICT(machine_id) DO UPDATE SET errors=excluded.errors, updated_at=excluded.updated_at",
    )
    .run(
      m.id,
      JSON.stringify(
        Object.fromEntries(
          Object.entries(errors).filter(([k]) => k.startsWith("workspace:")),
        ),
      ),
      now(),
    );
}
/** Revoke references in one transaction when a credential is removed. */
export function assertCredentialUnused(orgId: string, id: string) {
  if (listConnectors(orgId).some((c) => c.credentialId === id))
    throw new OrgError("Used by a provisioned connector", 409);
}
