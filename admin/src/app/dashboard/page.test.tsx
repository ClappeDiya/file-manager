// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest";
import { beforeEach, expect, it, vi } from "vitest";
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
const push = vi.fn();
vi.mock("next/navigation", () => ({
  useRouter: () => ({ push }),
  usePathname: () => "/dashboard",
}));
vi.mock("@/lib/i18n", () => ({ useTranslate: () => (key: string) => key }));
import DashboardPage from "./page";
beforeEach(() => {
  cleanup();
  push.mockClear();
});
it("keeps successful dashboard sections usable when devices fail, and recovers on retry", async () => {
  let failDevices = true;
  vi.stubGlobal(
    "fetch",
    vi.fn(async (url: string) => {
      if (url === "/api/devices" && failDevices)
        return new Response("{}", { status: 503 });
      const data: Record<string, unknown> = {
        "/api/users": { users: [{ id: "u", isActive: true }] },
        "/api/devices": { devices: [] },
        "/api/policies": { policies: [] },
        "/api/approvals": { approvals: [] },
        "/api/audit?pageSize=5": { entries: [] },
      };
      return new Response(JSON.stringify(data[url]), { status: 200 });
    }),
  );
  render(<DashboardPage />);
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "devices: Request failed (503)",
  );
  expect(screen.getByText("Active Users").parentElement).toHaveTextContent("1");
  expect(screen.getByText("Online Devices").parentElement).toHaveTextContent(
    "Unavailable",
  );
  expect(screen.queryByText("All compliant")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: /View fleet/ }));
  expect(push).toHaveBeenCalledWith("/devices");
  failDevices = false;
  fireEvent.click(
    screen.getByRole("button", { name: "Retry unavailable sections" }),
  );
  await waitFor(() =>
    expect(screen.queryByRole("alert")).not.toBeInTheDocument(),
  );
  expect(screen.getByText("Online Devices").parentElement).toHaveTextContent(
    "0/0",
  );
  expect(screen.getByText("No devices enrolled")).toBeInTheDocument();
  vi.unstubAllGlobals();
});
