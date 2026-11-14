import "@testing-library/jest-dom/vitest";
import { afterEach, expect, it, vi } from "vitest";
import { NextRequest } from "next/server";
const payload = vi.hoisted(() => ({
  sub: "owner@example.invalid",
  userId: "owner-id",
  role: "org_owner",
}));
const user = vi.hoisted(() => ({
  id: "owner-id",
  email: "owner@example.invalid",
  name: "Owner",
  role: "org_owner",
  isActive: true,
}));
vi.mock("jose", () => ({ jwtVerify: vi.fn(async () => ({ payload })) }));
vi.mock("next/headers", () => ({
  cookies: async () => ({ get: () => ({ value: "test-session" }) }),
}));
vi.mock("@/lib/data-store", () => ({ dataStore: { users: [user] } }));
import { GET } from "../../app/api/auth/session/route";
import { requireRole, isAuthorized } from "../../lib/utils/require-role";
afterEach(() => {
  payload.role = "org_owner";
  payload.userId = "owner-id";
  user.isActive = true;
  vi.unstubAllEnvs();
});
it("returns the complete persisted session identity", async () => {
  vi.stubEnv("NEXTAUTH_SECRET", "test-only-secret");
  const response = await GET();
  expect(response.status).toBe(200);
  expect((await response.json()).user).toMatchObject({
    id: "owner-id",
    role: "org_owner",
    isActive: true,
  });
});
it("rejects a session whose user identity no longer matches", async () => {
  vi.stubEnv("NEXTAUTH_SECRET", "test-only-secret");
  payload.userId = "different-id";
  expect((await GET()).status).toBe(401);
});
it.each(["unknown-role", "user"])(
  "rejects %s for a manager-only action",
  async (role) => {
    vi.stubEnv("NEXTAUTH_SECRET", "test-only-secret");
    payload.role = role;
    const result = await requireRole(
      new NextRequest("http://localhost/api/policies", {
        headers: { cookie: "ufop-admin-session=test" },
      }),
      "manager",
    );
    expect(isAuthorized(result)).toBe(false);
    expect((result as Response).status).toBe(403);
  },
);
