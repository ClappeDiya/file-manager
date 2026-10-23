/**
 * Accounts and per-job access rules of `ufop-server` (administrators):
 * `/api/v1/users` and `/api/v1/acl`. Deleting a rule shows first who
 * would lose which access (`/acl/{id}/preview-delete`). Accounts and
 * rules provisioned by the management console are read-only here.
 */
import { useCallback, useEffect, useState } from "react";
import { errorText } from "@/lib/error-text";
import { serverApi } from "@/lib/server-client";

export type ServerRole = "admin" | "operator" | "viewer";
export type JobPermission = "none" | "read" | "run" | "write" | "full";

export interface ServerUser {
  username: string;
  role: ServerRole;
  disabled: boolean;
  managed: boolean;
  created_at: string;
  updated_at: string;
}

export interface AclRule {
  id: string;
  principal: string;
  job_pattern: string;
  permission: JobPermission;
  managed: boolean;
  created_at: string;
}

export interface RuleEffect {
  username: string;
  job_id: string;
  job_name: string;
  before: JobPermission;
  after: JobPermission;
}

const ROLES: ServerRole[] = ["viewer", "operator", "admin"];
const PERMISSIONS: JobPermission[] = ["read", "run", "write", "full", "none"];
const ROLE_HELP: Record<ServerRole, string> = {
  viewer: "Jobs through rules, read only",
  operator: "Jobs through rules; server folders, destination tests",
  admin: "Everything",
};

const btn =
  "px-2 py-0.5 rounded border border-zinc-300 dark:border-zinc-600 text-xs disabled:opacity-50";
const input =
  "px-2 py-1 rounded border border-zinc-300 dark:border-zinc-600 bg-white dark:bg-zinc-800 text-xs";
const enc = encodeURIComponent;

export function AccessPanel() {
  const [users, setUsers] = useState<ServerUser[]>([]);
  const [rules, setRules] = useState<AclRule[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [newUser, setNewUser] = useState({
    username: "",
    role: "viewer" as ServerRole,
    password: "",
  });
  const [newRule, setNewRule] = useState({
    principal: "",
    job_pattern: "",
    permission: "read" as JobPermission,
  });
  const [preview, setPreview] = useState<{
    rule: AclRule;
    changes: RuleEffect[];
  } | null>(null);

  const run = async (f: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    setNote(null);
    try {
      await f();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const load = useCallback(
    () =>
      run(async () => {
        const [u, r] = await Promise.all([
          serverApi<{ users: ServerUser[] }>("GET", "/users"),
          serverApi<{ rules: AclRule[] }>("GET", "/acl"),
        ]);
        setUsers(u.users);
        setRules(r.rules);
      }),
    [],
  );
  useEffect(() => {
    void load();
  }, [load]);

  const patchUser = (u: ServerUser, patch: Record<string, unknown>) =>
    run(async () => {
      await serverApi("PUT", `/users/${enc(u.username)}`, patch);
      await load();
    });

  return (
    <div className="h-full overflow-y-auto p-4 space-y-6 text-sm">
      {error && (
        <p role="alert" className="text-red-600 text-xs">
          {error}
        </p>
      )}
      {note && <p className="text-green-700 text-xs">{note}</p>}

      <section aria-labelledby="accounts-h" className="space-y-2">
        <h2 id="accounts-h" className="font-semibold">
          Accounts
        </h2>
        <table className="w-full text-xs" data-testid="server-users">
          <thead className="text-left text-zinc-500">
            <tr>
              <th className="py-1">Name</th>
              <th>Role</th>
              <th>Enabled</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {users.map((u) => (
              <tr
                key={u.username}
                className="border-t border-zinc-100 dark:border-zinc-800"
              >
                <td className="py-1">
                  {u.username}
                  {u.managed && (
                    <span
                      className="ml-1 text-[10px] text-zinc-500"
                      title="Provisioned by the management console"
                    >
                      (organization)
                    </span>
                  )}
                </td>
                <td>
                  <select
                    aria-label={`Role of ${u.username}`}
                    className={input}
                    value={u.role}
                    disabled={busy || u.managed}
                    onChange={(e) =>
                      void patchUser(u, { role: e.target.value })
                    }
                  >
                    {ROLES.map((r) => (
                      <option key={r} value={r} title={ROLE_HELP[r]}>
                        {r}
                      </option>
                    ))}
                  </select>
                </td>
                <td>
                  <input
                    type="checkbox"
                    aria-label={`${u.username} enabled`}
                    checked={!u.disabled}
                    disabled={busy || u.managed}
                    onChange={(e) =>
                      void patchUser(u, { disabled: !e.target.checked })
                    }
                  />
                </td>
                <td className="text-right space-x-1">
                  <button
                    className={btn}
                    disabled={busy || u.managed}
                    onClick={() => {
                      const pw = window.prompt(
                        `New password for ${u.username} (8+ characters)`,
                      );
                      if (pw)
                        void patchUser(u, { password: pw }).then(() =>
                          setNote(
                            `Password of ${u.username} changed; its sessions were signed out.`,
                          ),
                        );
                    }}
                  >
                    Set password
                  </button>
                  <button
                    className={btn}
                    disabled={busy || u.managed}
                    onClick={() =>
                      window.confirm(
                        `Delete the account ${u.username}? Its sessions and API tokens stop working.`,
                      ) &&
                      void run(async () => {
                        await serverApi("DELETE", `/users/${enc(u.username)}`);
                        await load();
                      })
                    }
                  >
                    Delete
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        <form
          className="flex flex-wrap gap-2 items-end"
          onSubmit={(e) => {
            e.preventDefault();
            void run(async () => {
              await serverApi("POST", "/users", newUser);
              setNewUser({ username: "", role: "viewer", password: "" });
              await load();
            });
          }}
        >
          <label className="flex flex-col text-xs">
            New account
            <input
              className={input}
              aria-label="New account name"
              value={newUser.username}
              onChange={(e) =>
                setNewUser({ ...newUser, username: e.target.value })
              }
            />
          </label>
          <label className="flex flex-col text-xs">
            Role
            <select
              className={input}
              aria-label="New account role"
              value={newUser.role}
              onChange={(e) =>
                setNewUser({ ...newUser, role: e.target.value as ServerRole })
              }
            >
              {ROLES.map((r) => (
                <option key={r} value={r}>
                  {r}
                </option>
              ))}
            </select>
          </label>
          <label className="flex flex-col text-xs">
            Password
            <input
              type="password"
              autoComplete="new-password"
              className={input}
              aria-label="New account password"
              value={newUser.password}
              onChange={(e) =>
                setNewUser({ ...newUser, password: e.target.value })
              }
            />
          </label>
          <button
            type="submit"
            className={btn}
            disabled={busy || !newUser.username || newUser.password.length < 8}
          >
            Add account
          </button>
        </form>
      </section>

      <section aria-labelledby="rules-h" className="space-y-2">
        <h2 id="rules-h" className="font-semibold">
          Access rules
        </h2>
        <p className="text-xs text-zinc-500">
          Who may do what on which jobs. The strongest matching rule wins;
          administrators have full access to every job. Principal: a user name,
          role:operator, role:viewer or *. Jobs: an id, or a pattern such as
          prod-* matched against ids and names.
        </p>
        <table className="w-full text-xs" data-testid="server-rules">
          <thead className="text-left text-zinc-500">
            <tr>
              <th className="py-1">Principal</th>
              <th>Jobs</th>
              <th>Permission</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {rules.map((r) => (
              <tr
                key={r.id}
                className="border-t border-zinc-100 dark:border-zinc-800"
              >
                <td className="py-1">{r.principal}</td>
                <td className="font-mono">{r.job_pattern}</td>
                <td>{r.permission}</td>
                <td className="text-right">
                  {r.managed ? (
                    <span className="text-[10px] text-zinc-500">
                      organization
                    </span>
                  ) : (
                    <button
                      className={btn}
                      disabled={busy}
                      onClick={() =>
                        void run(async () => {
                          const p = await serverApi<{ changes: RuleEffect[] }>(
                            "GET",
                            `/acl/${enc(r.id)}/preview-delete`,
                          );
                          setPreview({ rule: r, changes: p.changes });
                        })
                      }
                    >
                      Delete…
                    </button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {preview && (
          <div
            role="dialog"
            aria-label="Delete access rule"
            className="rounded border border-amber-300 bg-amber-50 dark:bg-amber-950/30 p-3 space-y-2 text-xs"
          >
            <p>
              Delete the rule <b>{preview.rule.principal}</b> →{" "}
              <b>{preview.rule.permission}</b> on{" "}
              <span className="font-mono">{preview.rule.job_pattern}</span>?
            </p>
            {preview.changes.length === 0 ? (
              <p>Nobody loses access (other rules give the same).</p>
            ) : (
              <ul data-testid="rule-effects" className="list-disc pl-4">
                {preview.changes.map((c) => (
                  <li key={`${c.username}/${c.job_id}`}>
                    {c.username} on {c.job_name || c.job_id}: {c.before} →{" "}
                    {c.after}
                  </li>
                ))}
              </ul>
            )}
            <div className="flex gap-2">
              <button
                className={btn}
                onClick={() =>
                  void run(async () => {
                    await serverApi("DELETE", `/acl/${enc(preview.rule.id)}`);
                    setPreview(null);
                    await load();
                  })
                }
              >
                Delete rule
              </button>
              <button className={btn} onClick={() => setPreview(null)}>
                Keep it
              </button>
            </div>
          </div>
        )}
        <form
          className="flex flex-wrap gap-2 items-end"
          onSubmit={(e) => {
            e.preventDefault();
            void run(async () => {
              await serverApi("POST", "/acl", newRule);
              setNewRule({
                principal: "",
                job_pattern: "",
                permission: "read",
              });
              await load();
            });
          }}
        >
          <label className="flex flex-col text-xs">
            Principal
            <input
              className={input}
              aria-label="Rule principal"
              placeholder="alice, role:viewer or *"
              value={newRule.principal}
              onChange={(e) =>
                setNewRule({ ...newRule, principal: e.target.value })
              }
            />
          </label>
          <label className="flex flex-col text-xs">
            Jobs
            <input
              className={input}
              aria-label="Rule jobs"
              placeholder="job id or prod-*"
              value={newRule.job_pattern}
              onChange={(e) =>
                setNewRule({ ...newRule, job_pattern: e.target.value })
              }
            />
          </label>
          <label className="flex flex-col text-xs">
            Permission
            <select
              className={input}
              aria-label="Rule permission"
              value={newRule.permission}
              onChange={(e) =>
                setNewRule({
                  ...newRule,
                  permission: e.target.value as JobPermission,
                })
              }
            >
              {PERMISSIONS.map((p) => (
                <option key={p} value={p}>
                  {p}
                </option>
              ))}
            </select>
          </label>
          <button
            type="submit"
            className={btn}
            disabled={busy || !newRule.principal || !newRule.job_pattern}
          >
            Add rule
          </button>
        </form>
      </section>
    </div>
  );
}
