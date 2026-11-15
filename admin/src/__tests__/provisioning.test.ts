import { afterEach, describe, expect, it } from "vitest";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import net from "node:net";
import { db, resetDb } from "@/lib/db";
import { createOrg, setMembership } from "@/lib/db/repos/orgs";
import {
  createLink,
  registerWithToken,
  getMachine,
} from "@/lib/db/repos/machines";
import {
  createCredential,
  deleteCredential,
  policySetVersion,
  rotateCredential,
} from "@/lib/db/repos/fleet-policies";
import {
  CONNECTOR_PROTOCOLS,
  listConnectors,
  listDeliveredWorkspaces,
  saveConnector,
  saveWorkspace,
  workspaceDeliveries,
  deliveryStatus,
} from "@/lib/db/repos/provisioning";
import { json, makeUser, req } from "./helpers";
import * as connectors from "@/app/api/connectors/route";
import * as connector from "@/app/api/connectors/[id]/route";
import * as workspaces from "@/app/api/workspaces/route";
import * as workspace from "@/app/api/workspaces/[id]/route";
import * as agent from "@/app/api/agent/policies/route";
import * as receipt from "@/app/api/agent/policies/applied/route";
const P = (id: string) => ({ params: Promise.resolve({ id }) });
const enroll = (org = "org_default", agentKind = "desktop") =>
  registerWithToken(createLink(org, {}, "test").token, {
    name: "Desktop",
    agentKind,
  });
const input = {
  name: "Team files",
  protocol: "sftp" as const,
  host: "files.example.org",
  remotePath: "/team",
};
afterEach(() => {
  delete process.env.CONNECTORS_ALLOW_PRIVATE;
});
describe("connector provisioning and shared workspace delivery", () => {
  it("persists every supported protocol without claiming endpoint authentication", () => {
    for (const protocol of CONNECTOR_PROTOCOLS) {
      const c = saveConnector("org_default", {
        ...input,
        name: protocol,
        protocol,
      });
      expect(
        listConnectors("org_default").find((x) => x.id === c.id)?.protocol,
      ).toBe(protocol);
    }
  });
  it("requires authentication and permissions on every route", async () => {
    expect(
      (await connectors.GET(await req("GET", "/api/connectors"))).status,
    ).toBe(401);
    expect(
      (await workspaces.GET(await req("GET", "/api/workspaces"))).status,
    ).toBe(401);
    const u = makeUser("auditor@example.org", "read_only_auditor");
    expect(
      (
        await connectors.POST(
          await req("POST", "/api/connectors", { as: u, body: input }),
        )
      ).status,
    ).toBe(403);
    expect(
      (
        await workspaces.POST(
          await req("POST", "/api/workspaces", {
            as: u,
            body: { name: "x", connectorIds: [], machineIds: [] },
          }),
        )
      ).status,
    ).toBe(403);
    expect(
      (
        await connector.DELETE(
          await req("DELETE", "/api/connectors/x", { as: u }),
          P("x"),
        )
      ).status,
    ).toBe(403);
  });
  it("persists definitions and explicit machine grants across a database reopen", async () => {
    const dir = mkdtempSync(join(tmpdir(), "ufop-provisioning-"));
    resetDb();
    process.env.ADMIN_DB_PATH = join(dir, "admin.sqlite");
    try {
      const u = makeUser("admin@example.org", "org_admin");
      const m = enroll();
      const c = await json<{ connector: { id: string } }>(
        await connectors.POST(
          await req("POST", "/api/connectors", {
            as: u,
            body: { ...input, machineIds: [m.machine.id] },
          }),
        ),
      );
      expect(c.status).toBe(201);
      const w = await workspaces.POST(
        await req("POST", "/api/workspaces", {
          as: u,
          body: {
            name: "Team",
            connectorIds: [c.body.connector.id],
            machineIds: [m.machine.id],
          },
        }),
      );
      expect(w.status).toBe(201);
      resetDb();
      expect(listConnectors("org_default")[0].machineIds).toEqual([
        m.machine.id,
      ]);
      expect(listDeliveredWorkspaces("org_default")[0].connectorIds).toEqual([
        c.body.connector.id,
      ]);
      expect(workspaceDeliveries(getMachine(m.machine.id)!)).toHaveLength(2);
    } finally {
      resetDb();
      process.env.ADMIN_DB_PATH = ":memory:";
      rmSync(dir, { recursive: true, force: true });
    }
  });
  it("refuses cross-organization resources and non-desktop grants, including known IDs", async () => {
    const u = makeUser("admin@example.org", "org_admin"),
      other = createOrg({ name: "Other" });
    setMembership(u.id, other.id, "org_admin");
    const foreign = enroll(other.id);
    const server = enroll("org_default", "server");
    for (const id of [foreign.machine.id, server.machine.id])
      expect(
        (
          await connectors.POST(
            await req("POST", "/api/connectors", {
              as: u,
              body: { ...input, machineIds: [id] },
            }),
          )
        ).status,
      ).toBe(400);
    const c = saveConnector(other.id, input);
    expect(
      (
        await connector.GET(
          await req("GET", "/api/connectors/x", { as: u }),
          P(c.id),
        )
      ).status,
    ).toBe(404);
    expect(
      (
        await connector.PATCH(
          await req("PATCH", "/api/connectors/x", { as: u, body: input }),
          P(c.id),
        )
      ).status,
    ).toBe(404);
    expect(
      (
        await workspaces.POST(
          await req("POST", "/api/workspaces", {
            as: u,
            body: { name: "Leak", connectorIds: [c.id], machineIds: [] },
          }),
        )
      ).status,
    ).toBe(400);
    expect(
      (
        await json<{ connectors: unknown[] }>(
          await connectors.GET(await req("GET", "/api/connectors", { as: u })),
        )
      ).body.connectors,
    ).toEqual([]);
  });
  it("rejects embedded secrets, malformed definitions, and a credential from another organization", async () => {
    const u = makeUser("admin@example.org", "org_admin");
    const other = createOrg({ name: "Other" });
    const secret = createCredential(
      other.id,
      { name: "Foreign", secret: "never-exposed" },
      "test",
    );
    for (const bad of [
      { password: "secret" },
      { host: "https://user:secret@example.org" },
      { port: 0 },
      { credentialId: secret.id },
      { protocol: "imaginary" },
      { machineIds: "all" },
    ])
      expect(
        (
          await connectors.POST(
            await req("POST", "/api/connectors", {
              as: u,
              body: { ...input, ...bad },
            }),
          )
        ).status,
      ).toBe(400);
  });
  it("delivers only granted definitions and encrypted credential bytes, then tracks actual agent acknowledgements", async () => {
    const admin = makeUser("admin@example.org", "org_admin"),
      m = enroll(),
      ungranted = enroll();
    const credential = createCredential(
      "org_default",
      { name: "Storage", secret: "test-secret-bytes" },
      "test",
    );
    const c = saveConnector("org_default", {
      ...input,
      credentialId: credential.id,
    });
    const w = saveWorkspace("org_default", {
      name: "Design",
      connectorIds: [c.id],
      machineIds: [m.machine.id],
    });
    expect(
      db()
        .prepare("SELECT ciphertext FROM central_credentials WHERE id=?")
        .get(credential.id),
    ).not.toEqual({ ciphertext: "test-secret-bytes" });
    const denied = await json<{
      workspaces: unknown[];
      credentials: unknown[];
    }>(
      await agent.GET(
        await req("GET", "/api/agent/policies", {
          headers: { authorization: `Bearer ${ungranted.token}` },
        }),
      ),
    );
    expect(denied.body.workspaces).toEqual([]);
    expect(denied.body.credentials).toEqual([]);
    const allowed = await json<{
      workspaces: {
        id: string;
        version: number;
        connections: { credentialId: string }[];
      }[];
      credentials: { secret: string }[];
    }>(
      await agent.GET(
        await req("GET", "/api/agent/policies", {
          headers: { authorization: `Bearer ${m.token}` },
        }),
      ),
    );
    expect(allowed.body.workspaces[0].connections[0].credentialId).toBe(
      `central:${credential.id}`,
    );
    expect(allowed.body.credentials[0].secret).toBe("test-secret-bytes");
    const key = `workspace:${w.id}`,
      version = allowed.body.workspaces[0].version;
    expect(deliveryStatus("org_default", w.id)[0].status).toBe("pending");
    await receipt.POST(
      await req("POST", "/api/agent/policies/applied", {
        headers: { authorization: `Bearer ${m.token}` },
        body: { applied: {}, errors: { [key]: "Secure store unavailable" } },
      }),
    );
    expect(deliveryStatus("org_default", w.id)[0]).toMatchObject({
      status: "failed",
      error: "Secure store unavailable",
    });
    await receipt.POST(
      await req("POST", "/api/agent/policies/applied", {
        headers: { authorization: `Bearer ${m.token}` },
        body: { applied: { [key]: version }, errors: {} },
      }),
    );
    expect(deliveryStatus("org_default", w.id)[0]).toMatchObject({
      status: "applied",
      error: null,
    });
    const versionBefore = policySetVersion(getMachine(m.machine.id)!);
    rotateCredential("org_default", credential.id, "new-bytes");
    expect(policySetVersion(getMachine(m.machine.id)!)).not.toBe(versionBefore);
    expect(deliveryStatus("org_default", w.id)[0].status).toBe("pending");
    expect(
      JSON.stringify(
        (
          await json(
            await connectors.GET(
              await req("GET", "/api/connectors", { as: admin }),
            ),
          )
        ).body,
      ),
    ).not.toContain("test-secret-bytes");
    expect(() => deleteCredential("org_default", credential.id)).toThrow(
      /connector/,
    );
  });
  it("updates grants, blocks deleting referenced connectors, and revokes delivery", async () => {
    const u = makeUser("admin@example.org", "org_admin"),
      m = enroll();
    const c = saveConnector("org_default", input);
    const w = saveWorkspace("org_default", {
      name: "Team",
      connectorIds: [c.id],
      machineIds: [m.machine.id],
    });
    expect(
      (
        await connector.DELETE(
          await req("DELETE", "/api/connectors/x", { as: u }),
          P(c.id),
        )
      ).status,
    ).toBe(409);
    const before = policySetVersion(m.machine);
    expect(
      (
        await workspace.PATCH(
          await req("PATCH", "/api/workspaces/x", {
            as: u,
            body: { name: "Team", connectorIds: [c.id], machineIds: [] },
          }),
          P(w.id),
        )
      ).status,
    ).toBe(200);
    expect(workspaceDeliveries(m.machine)).toEqual([]);
    expect(policySetVersion(m.machine)).not.toBe(before);
    expect(
      (
        await workspace.DELETE(
          await req("DELETE", "/api/workspaces/x", { as: u }),
          P(w.id),
        )
      ).status,
    ).toBe(200);
    expect(
      (
        await connector.DELETE(
          await req("DELETE", "/api/connectors/x", { as: u }),
          P(c.id),
        )
      ).status,
    ).toBe(200);
  });
  it("probes actual destination sockets and never calls TCP reachability authenticated success", async () => {
    const u = makeUser("admin@example.org", "org_admin");
    const server = net.createServer((s) => s.end());
    await new Promise<void>((r) => server.listen(0, "127.0.0.1", r));
    try {
      const addr = server.address() as net.AddressInfo;
      const c = saveConnector("org_default", {
        ...input,
        host: "127.0.0.1",
        port: addr.port,
      });
      const probe = () => connector.POST;
      const request = () =>
        req("POST", "/api/connectors/x", { as: u, body: { action: "test" } });
      expect(
        (
          await json<{ reachable: boolean }>(
            await probe()(await request(), P(c.id)),
          )
        ).body.reachable,
      ).toBe(false);
      process.env.CONNECTORS_ALLOW_PRIVATE = "1";
      expect(
        (await json(await probe()(await request(), P(c.id)))).body,
      ).toMatchObject({ reachable: true, authenticationTested: false });
      await new Promise<void>((resolve, reject) =>
        server.close((e) => (e ? reject(e) : resolve())),
      );
      expect(
        (
          await json<{ reachable: boolean }>(
            await probe()(await request(), P(c.id)),
          )
        ).body.reachable,
      ).toBe(false);
    } finally {
      if (server.listening) server.close();
    }
  });
});
