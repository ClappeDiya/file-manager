"use client";
import { useState, type FormEvent } from "react";
import Link from "next/link";
import { PageHeader } from "@/components/layout/page-header";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Select } from "@/components/ui/select";
import {
  Dialog,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { ErrorBanner, Loading } from "@/components/fleet/badges";
import { useAuthStore } from "@/lib/stores/auth-store";
import { useApi } from "@/lib/use-api";
import { api, errorText } from "@/lib/api-client";
import type {
  ConnectorDefinition,
  DeliveredWorkspace,
} from "@/lib/db/repos/provisioning";

type Delivery = {
  machineId: string;
  name: string;
  status: string;
  error: string | null;
};
type Connector = ConnectorDefinition & { delivery: Delivery[] };
type Workspace = DeliveredWorkspace & { delivery: Delivery[] };
type Draft = {
  id?: string;
  name: string;
  description: string;
  protocol: ConnectorDefinition["protocol"];
  host: string;
  port: string;
  username: string;
  remotePath: string;
  credentialId: string;
  connectorIds: string[];
  machineIds: string[];
};
const empty = (): Draft => ({
  name: "",
  description: "",
  protocol: "sftp",
  host: "",
  port: "",
  username: "",
  remotePath: "/",
  credentialId: "",
  connectorIds: [],
  machineIds: [],
});
const protocols = [
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
];
function Choices({
  legend,
  items,
  value,
  onChange,
}: {
  legend: string;
  items: { id: string; name: string }[];
  value: string[];
  onChange: (v: string[]) => void;
}) {
  return (
    <fieldset className="min-w-0 rounded border p-3">
      <legend className="px-1 text-sm font-medium">{legend}</legend>
      {items.length ? (
        <div className="max-h-40 space-y-2 overflow-y-auto">
          {items.map((x) => (
            <label
              key={x.id}
              className="flex items-start gap-2 text-sm break-words"
            >
              <input
                type="checkbox"
                checked={value.includes(x.id)}
                onChange={(e) =>
                  onChange(
                    e.target.checked
                      ? [...value, x.id]
                      : value.filter((id) => id !== x.id),
                  )
                }
                className="mt-1"
              />
              {x.name}
            </label>
          ))}
        </div>
      ) : (
        <p className="text-sm text-foreground-secondary">
          None available in this organization.
        </p>
      )}
    </fieldset>
  );
}
export function ProvisioningPage({
  kind,
}: {
  kind: "connectors" | "workspaces";
}) {
  const isConnector = kind === "connectors";
  const singular = isConnector ? "connector" : "workspace";
  const canWrite = useAuthStore((s) =>
    s.checkPermission(isConnector ? "connectors.write" : "workspaces.write"),
  );
  const { data, error, loading, reload } = useApi<{
    connectors?: Connector[];
    workspaces?: Workspace[];
  }>(`/api/${kind}`, 10_000);
  const definitions = useApi<{ connectors: Connector[] }>("/api/connectors");
  const machines = useApi<{
    machines: {
      id: string;
      name: string;
      agentKind: string;
      status: string;
      orgId: string;
    }[];
  }>("/api/fleet/machines");
  const credentials = useApi<{ credentials: { id: string; name: string }[] }>(
    "/api/fleet/credentials",
  );
  const orgId = useAuthStore((s) => s.organization?.id);
  const [draft, setDraft] = useState<Draft | null>(null),
    [busy, setBusy] = useState(false),
    [err, setErr] = useState<string | null>(null),
    [notice, setNotice] = useState<string | null>(null);
  const items = isConnector
    ? (data?.connectors ?? [])
    : (data?.workspaces ?? []);
  const targets = (machines.data?.machines ?? []).filter(
    (m) =>
      m.agentKind === "desktop" &&
      m.status === "active" &&
      (!orgId || m.orgId === orgId),
  );
  const edit = (item: Connector | Workspace) => {
    setErr(null);
    setDraft(
      "protocol" in item
        ? {
            ...empty(),
            ...item,
            port: item.port?.toString() ?? "",
            credentialId: item.credentialId ?? "",
          }
        : { ...empty(), ...item },
    );
  };
  const save = async (e: FormEvent) => {
    e.preventDefault();
    if (!draft || busy) return;
    setBusy(true);
    setErr(null);
    try {
      const body = isConnector
        ? {
            name: draft.name,
            protocol: draft.protocol,
            host: draft.host,
            port: draft.port ? Number(draft.port) : null,
            username: draft.username,
            remotePath: draft.remotePath,
            credentialId: draft.credentialId || null,
            machineIds: draft.machineIds,
          }
        : {
            name: draft.name,
            description: draft.description,
            connectorIds: draft.connectorIds,
            machineIds: draft.machineIds,
          };
      await api(`/api/${kind}${draft.id ? `/${draft.id}` : ""}`, {
        method: draft.id ? "PATCH" : "POST",
        body,
      });
      setDraft(null);
      setNotice(
        "Saved. Delivery stays pending until each desktop installs and acknowledges the revision.",
      );
      await reload();
      await definitions.reload();
    } catch (e) {
      setErr(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  const remove = async (item: Connector | Workspace) => {
    if (
      !window.confirm(
        `Remove “${item.name}” and revoke its direct desktop grants? Offline desktops apply revocation when they reconnect.`,
      )
    )
      return;
    setBusy(true);
    setErr(null);
    try {
      await api(`/api/${kind}/${item.id}`, { method: "DELETE" });
      setNotice(
        "Removed. Desktops reconcile grants on their next successful console sync.",
      );
      await reload();
      await definitions.reload();
    } catch (e) {
      setErr(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  const probe = async (c: Connector) => {
    setBusy(true);
    setErr(null);
    try {
      const r = await api<{
        reachable: boolean;
        error: string | null;
        durationMs: number;
      }>(`/api/connectors/${c.id}`, { body: { action: "test" } });
      setNotice(
        r.reachable
          ? `TCP endpoint reachable in ${r.durationMs} ms. Authentication and folder access have not been tested.`
          : `Endpoint unreachable: ${r.error}`,
      );
    } catch (e) {
      setErr(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div>
      <PageHeader
        title={isConnector ? "Connectors" : "Shared Workspaces"}
        description={
          isConnector
            ? "Provision saved storage connections to selected enrolled desktops."
            : "Deliver a set of saved connections to selected enrolled desktops."
        }
        actions={
          canWrite && (
            <Button
              onClick={() => {
                setErr(null);
                setDraft(empty());
              }}
            >
              Add {singular}
            </Button>
          )
        }
      />
      <p className="mb-4 text-sm text-foreground-secondary">
        Credentials stay encrypted and are delivered only to granted desktops.
        Storage permissions come from the remote account. Offline desktops keep
        their last configuration until they reconnect.{" "}
        <Link className="text-primary underline" href="/fleet/credentials">
          Manage credentials
        </Link>
      </p>
      <ErrorBanner message={err ?? error} />
      {notice && (
        <p role="status" className="mb-4 rounded border p-3 text-sm">
          {notice}
        </p>
      )}
      {loading && !data ? (
        <Loading />
      ) : items.length === 0 ? (
        <section className="rounded border p-6">
          <h2 className="font-medium">No {kind} yet</h2>
          <p className="mt-2 text-sm">
            {isConnector
              ? "Add a connector and choose the desktops that should receive it."
              : "Create connectors first, then choose a set and grant it to your team’s desktops."}
          </p>
        </section>
      ) : (
        <ul className="space-y-4">
          {items.map((item) => (
            <li key={item.id} className="min-w-0 rounded border p-4">
              <div className="flex flex-wrap items-start justify-between gap-3">
                <div className="min-w-0">
                  <h2 className="font-semibold break-words">{item.name}</h2>
                  {"protocol" in item ? (
                    <p className="break-all text-sm text-foreground-secondary">
                      {item.protocol.toUpperCase()} · {item.host}
                      {item.port ? `:${item.port}` : ""} · {item.remotePath}
                    </p>
                  ) : (
                    <p className="text-sm break-words">
                      {item.description} · {item.connectorIds.length}{" "}
                      connection(s)
                    </p>
                  )}
                </div>
                {canWrite && (
                  <div className="flex flex-wrap gap-2">
                    <Button
                      variant="outline"
                      disabled={busy}
                      onClick={() => edit(item)}
                      aria-label={`Edit ${item.name}`}
                    >
                      Edit
                    </Button>
                    {"protocol" in item && (
                      <Button
                        variant="outline"
                        disabled={busy}
                        onClick={() => void probe(item)}
                        aria-label={`Test endpoint for ${item.name}`}
                      >
                        Test endpoint
                      </Button>
                    )}
                    <Button
                      variant="outline"
                      disabled={busy}
                      onClick={() => void remove(item)}
                      aria-label={`Remove ${item.name}`}
                    >
                      Remove
                    </Button>
                  </div>
                )}
              </div>
              <p className="mt-3 text-sm">
                {item.delivery.filter((d) => d.status === "applied").length} of{" "}
                {item.delivery.length} directly granted desktops acknowledged
              </p>
              {item.delivery.length > 0 && (
                <ul className="mt-1 space-y-1 text-sm">
                  {item.delivery.map((d) => (
                    <li key={d.machineId} className="break-words">
                      {d.name}:{" "}
                      {d.status === "applied"
                        ? "Installed"
                        : d.status === "failed"
                          ? "Failed — retrying on next sync"
                          : "Pending"}
                      {d.error && ` · ${d.error}`}
                    </li>
                  ))}
                </ul>
              )}
              {isConnector && (
                <p className="mt-2 text-xs text-foreground-secondary">
                  Workspace grants are tracked on Shared Workspaces.
                </p>
              )}
            </li>
          ))}
        </ul>
      )}
      <Dialog
        open={draft !== null}
        onClose={() => {
          if (!busy) setDraft(null);
        }}
      >
        <DialogHeader>
          <DialogTitle>
            {draft?.id ? "Edit" : "Add"} {singular}
          </DialogTitle>
          <DialogDescription>
            {isConnector
              ? "Choose an endpoint and an existing credential. No connection is attempted during provisioning."
              : "Select connections and enrolled desktops. Grants persist until you remove them."}
          </DialogDescription>
        </DialogHeader>
        {draft && (
          <form onSubmit={save} className="space-y-4">
            <ErrorBanner message={err} />
            <Input
              label="Name"
              required
              maxLength={120}
              value={draft.name}
              onChange={(e) => setDraft({ ...draft, name: e.target.value })}
            />
            {isConnector ? (
              <>
                <Select
                  label="Protocol"
                  options={protocols.map((p) => ({
                    value: p,
                    label: p.toUpperCase(),
                  }))}
                  value={draft.protocol}
                  onChange={(e) =>
                    setDraft({
                      ...draft,
                      protocol: e.target.value as Draft["protocol"],
                    })
                  }
                />
                <Input
                  label="Host or S3 region"
                  required
                  placeholder="files.example.org"
                  value={draft.host}
                  onChange={(e) => setDraft({ ...draft, host: e.target.value })}
                />
                <Input
                  label="Port (optional)"
                  type="number"
                  min={1}
                  max={65535}
                  value={draft.port}
                  onChange={(e) => setDraft({ ...draft, port: e.target.value })}
                />
                <Input
                  label="Username / access key ID / Azure account"
                  value={draft.username}
                  onChange={(e) =>
                    setDraft({ ...draft, username: e.target.value })
                  }
                />
                <Input
                  label="Remote folder / bucket / container"
                  required
                  value={draft.remotePath}
                  onChange={(e) =>
                    setDraft({ ...draft, remotePath: e.target.value })
                  }
                />
                <Select
                  label="Credential"
                  value={draft.credentialId}
                  options={[
                    { value: "", label: "No credential" },
                    ...(credentials.data?.credentials ?? []).map((c) => ({
                      value: c.id,
                      label: c.name,
                    })),
                  ]}
                  onChange={(e) =>
                    setDraft({ ...draft, credentialId: e.target.value })
                  }
                />
                {credentials.error && (
                  <ErrorBanner message={credentials.error} />
                )}
              </>
            ) : (
              <>
                <Input
                  label="Description"
                  maxLength={1000}
                  value={draft.description}
                  onChange={(e) =>
                    setDraft({ ...draft, description: e.target.value })
                  }
                />
                <Choices
                  legend="Connections"
                  items={definitions.data?.connectors ?? []}
                  value={draft.connectorIds}
                  onChange={(v) => setDraft({ ...draft, connectorIds: v })}
                />
                {definitions.error && (
                  <ErrorBanner message={definitions.error} />
                )}
              </>
            )}
            <Choices
              legend="Granted desktops"
              items={targets}
              value={draft.machineIds}
              onChange={(v) => setDraft({ ...draft, machineIds: v })}
            />
            {machines.error && <ErrorBanner message={machines.error} />}
            <DialogFooter>
              <Button
                type="button"
                variant="outline"
                disabled={busy}
                onClick={() => setDraft(null)}
              >
                Cancel
              </Button>
              <Button type="submit" disabled={busy}>
                {busy ? "Saving…" : "Save"}
              </Button>
            </DialogFooter>
          </form>
        )}
      </Dialog>
    </div>
  );
}
