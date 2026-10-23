/**
 * Server accounts and access rules UI: lists, adds accounts and rules,
 * changes roles, previews who loses access before deleting a rule, and
 * keeps organization-managed entries read-only.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import {
  AccessPanel,
  type AclRule,
  type ServerUser,
} from "@/components/server/access-panel";
import { __setAccessTokenForTests, login } from "@/lib/server-client";

const calls: { method: string; url: string; body: unknown }[] = [];
let users: ServerUser[];
let rules: AclRule[];

const user = (
  username: string,
  role: ServerUser["role"],
  managed = false,
): ServerUser => ({
  username,
  role,
  disabled: false,
  managed,
  created_at: "",
  updated_at: "",
});

beforeEach(() => {
  calls.length = 0;
  users = [
    user("admin", "admin"),
    user("ops", "operator"),
    user("org", "viewer", true),
  ];
  rules = [
    {
      id: "acl_1",
      principal: "ops",
      job_pattern: "prod-*",
      permission: "run",
      managed: false,
      created_at: "",
    },
    {
      id: "acl_2",
      principal: "*",
      job_pattern: "j9",
      permission: "read",
      managed: true,
      created_at: "",
    },
  ];
  window.__UFOP_SERVER__ = { api: "/api/v1" };
  __setAccessTokenForTests("tok");
  vi.stubGlobal(
    "fetch",
    vi.fn(async (url: string, init: RequestInit) => {
      const method = init.method ?? "GET";
      const body = init.body ? JSON.parse(String(init.body)) : undefined;
      calls.push({ method, url, body });
      if (url === "/api/v1/users" && method === "GET")
        return new Response(JSON.stringify({ users }));
      if (url === "/api/v1/acl" && method === "GET")
        return new Response(JSON.stringify({ rules }));
      if (url === "/api/v1/users" && method === "POST") {
        users = [...users, user(body.username, body.role)];
        return new Response(JSON.stringify(users[users.length - 1]));
      }
      if (url.startsWith("/api/v1/users/") && method === "PUT") {
        const name = decodeURIComponent(url.split("/").pop()!);
        users = users.map((u) => (u.username === name ? { ...u, ...body } : u));
        return new Response(JSON.stringify({}));
      }
      if (url === "/api/v1/acl" && method === "POST") {
        rules = [
          ...rules,
          { id: "acl_3", managed: false, created_at: "", ...body },
        ];
        return new Response(JSON.stringify(rules[rules.length - 1]));
      }
      if (url === "/api/v1/acl/acl_1/preview-delete")
        return new Response(
          JSON.stringify({
            rule_id: "acl_1",
            changes: [
              {
                username: "ops",
                job_id: "j1",
                job_name: "prod-db",
                before: "run",
                after: "none",
              },
            ],
          }),
        );
      if (url === "/api/v1/acl/acl_1" && method === "DELETE") {
        rules = rules.filter((r) => r.id !== "acl_1");
        return new Response(JSON.stringify({ ok: true }));
      }
      return new Response(
        JSON.stringify({ message: "unexpected", advice: "" }),
        { status: 400 },
      );
    }),
  );
});

describe("access panel", () => {
  it("adds accounts and changes roles; organization accounts are read-only", async () => {
    render(<AccessPanel />);
    const table = await screen.findByTestId("server-users");
    await waitFor(() => expect(within(table).getByText("ops")).toBeTruthy());
    expect(
      (screen.getByLabelText("Role of org") as HTMLSelectElement).disabled,
    ).toBe(true);
    fireEvent.change(screen.getByLabelText("Role of ops"), {
      target: { value: "viewer" },
    });
    await waitFor(() =>
      expect(
        calls.some((c) => c.method === "PUT" && c.url === "/api/v1/users/ops"),
      ).toBe(true),
    );
    expect(calls.find((c) => c.method === "PUT")?.body).toEqual({
      role: "viewer",
    });
    fireEvent.change(screen.getByLabelText("New account name"), {
      target: { value: "val" },
    });
    fireEvent.change(screen.getByLabelText("New account role"), {
      target: { value: "operator" },
    });
    const add = screen.getByRole("button", {
      name: "Add account",
    }) as HTMLButtonElement;
    fireEvent.change(screen.getByLabelText("New account password"), {
      target: { value: "short" },
    });
    expect(add.disabled).toBe(true);
    fireEvent.change(screen.getByLabelText("New account password"), {
      target: { value: "long-enough-pw" },
    });
    fireEvent.click(add);
    await waitFor(() =>
      expect(
        within(screen.getByTestId("server-users")).getByText("val"),
      ).toBeTruthy(),
    );
    expect(
      calls.find((c) => c.method === "POST" && c.url === "/api/v1/users")?.body,
    ).toEqual({
      username: "val",
      role: "operator",
      password: "long-enough-pw",
    });
  });

  it("previews the effect before deleting a rule and adds rules", async () => {
    render(<AccessPanel />);
    const table = await screen.findByTestId("server-rules");
    await waitFor(() => expect(within(table).getByText("prod-*")).toBeTruthy());
    expect(within(table).getByText("organization")).toBeTruthy();
    fireEvent.click(within(table).getByRole("button", { name: "Delete…" }));
    const dialog = await screen.findByRole("dialog", {
      name: "Delete access rule",
    });
    expect(within(dialog).getByTestId("rule-effects").textContent).toContain(
      "ops on prod-db: run → none",
    );
    expect(calls.some((c) => c.method === "DELETE")).toBe(false);
    fireEvent.click(
      within(dialog).getByRole("button", { name: "Delete rule" }),
    );
    await waitFor(() => expect(screen.queryByText("prod-*")).toBeNull());
    fireEvent.change(screen.getByLabelText("Rule principal"), {
      target: { value: "role:viewer" },
    });
    fireEvent.change(screen.getByLabelText("Rule jobs"), {
      target: { value: "*" },
    });
    fireEvent.change(screen.getByLabelText("Rule permission"), {
      target: { value: "read" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Add rule" }));
    await waitFor(() =>
      expect(
        calls.find((c) => c.method === "POST" && c.url === "/api/v1/acl")?.body,
      ).toEqual({
        principal: "role:viewer",
        job_pattern: "*",
        permission: "read",
      }),
    );
  });

  it("signs in with a user name", async () => {
    __setAccessTokenForTests(null);
    await login("pw-123456", "ops").catch(() => undefined);
    expect(calls[0]).toMatchObject({
      method: "POST",
      url: "/api/v1/auth/login",
      body: { username: "ops", password: "pw-123456" },
    });
  });
});
