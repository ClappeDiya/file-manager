/**
 * ufop-server web mode: commands go to /api/v1/rpc/<command> with the
 * desktop invoke arguments, the access token is refreshed once on 401, and
 * no refresh is attempted without a session cookie.
 */
import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { tauriInvoke, isBackendAvailable } from "@/hooks/use-tauri";
import { __setAccessTokenForTests, login, refresh, serverRpc } from "@/lib/server-client";

const fetchMock = vi.fn();

function res(status: number, body: unknown) {
  return { ok: status >= 200 && status < 300, status, json: async () => body } as Response;
}

beforeEach(() => {
  window.__UFOP_SERVER__ = { api: "/api/v1" };
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  vi.stubGlobal("fetch", fetchMock);
  fetchMock.mockReset();
  __setAccessTokenForTests(null);
  document.cookie = "ufop_session=; Max-Age=0";
});

afterEach(() => {
  delete window.__UFOP_SERVER__;
  vi.unstubAllGlobals();
});

describe("server client", () => {
  it("routes tauriInvoke through the RPC endpoint with the same args", async () => {
    expect(isBackendAvailable()).toBe(true);
    fetchMock.mockResolvedValueOnce(res(200, { access_token: "t1" }));
    await login("pw");
    fetchMock.mockResolvedValueOnce(res(200, ["job"]));
    const r = await tauriInvoke<string[]>("backup_list_jobs", { id: "x", dryRun: true });
    expect(r).toEqual(["job"]);
    const [url, init] = fetchMock.mock.calls[1];
    expect(url).toBe("/api/v1/rpc/backup_list_jobs");
    expect(init.headers.Authorization).toBe("Bearer t1");
    expect(JSON.parse(init.body)).toEqual({ id: "x", dryRun: true });
  });

  it("refreshes once on 401 and retries", async () => {
    __setAccessTokenForTests("expired");
    document.cookie = "ufop_session=1";
    fetchMock
      .mockResolvedValueOnce(res(401, {}))
      .mockResolvedValueOnce(res(200, { access_token: "fresh" }))
      .mockResolvedValueOnce(res(200, { ok: true }));
    expect(await serverRpc("backup_status")).toEqual({ ok: true });
    expect(fetchMock.mock.calls[1][0]).toBe("/api/v1/auth/refresh");
    expect(fetchMock.mock.calls[2][1].headers.Authorization).toBe("Bearer fresh");
  });

  it("does not call refresh without a session cookie", async () => {
    expect(await refresh()).toBe(false);
    expect(fetchMock).not.toHaveBeenCalled();
    await expect(serverRpc("backup_status")).rejects.toMatchObject({ message: "Not signed in" });
  });

  it("surfaces server errors with advice", async () => {
    fetchMock.mockResolvedValueOnce(res(401, { message: "Wrong password", advice: "Check the server password." }));
    await expect(login("bad")).rejects.toMatchObject({ message: "Wrong password" });
  });
});
