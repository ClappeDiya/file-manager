import { afterEach, expect, it, vi } from "vitest";
import { POST } from "../../marketing/src/app/api/waitlist/route";
afterEach(() => {
  vi.unstubAllEnvs();
  vi.unstubAllGlobals();
});
it("does not confirm enrollment when no destination is configured", async () => {
  vi.stubEnv("WAITLIST_WEBHOOK_URL", "");
  const response = await POST(
    new Request("http://localhost/api/waitlist", {
      method: "POST",
      body: JSON.stringify({ email: "audit@example.invalid" }),
    }),
  );
  expect(response.status).toBe(503);
});
