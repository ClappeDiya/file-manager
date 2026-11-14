import "@testing-library/jest-dom/vitest";
import { beforeEach, expect, it, vi } from "vitest";
import {
  fireEvent,
  render,
  screen,
  waitFor,
  cleanup,
} from "@testing-library/react";
import { AdminShell } from "../layout/admin-shell";
import LoginPage from "../../app/login/page";
import { useAuthStore } from "../../lib/stores/auth-store";
const replace = vi.hoisted(() => vi.fn());
vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace }),
  usePathname: () => "/policies",
}));
beforeEach(() => {
  cleanup();
  useAuthStore.setState({
    user: null,
    isAuthenticated: false,
    isValidating: false,
    error: null,
  });
  replace.mockClear();
});
it("uses the authenticated owner identity on first sign-in", async () => {
  const owner = {
    id: "real-owner",
    role: "org_owner",
    email: "owner@example.invalid",
    name: "Owner",
  };
  vi.stubGlobal(
    "fetch",
    vi.fn(async (url: string) => ({
      ok: true,
      json: async () =>
        url.endsWith("/login") ? { user: owner } : { user: owner },
    })),
  );
  render(<LoginPage />);
  fireEvent.change(screen.getByLabelText("Email"), {
    target: { value: owner.email },
  });
  fireEvent.change(screen.getByLabelText("Password"), {
    target: { value: "test-only-password" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Sign in" }));
  await waitFor(() =>
    expect(useAuthStore.getState().user?.role).toBe("org_owner"),
  );
  expect(useAuthStore.getState().user?.id).toBe("real-owner");
  vi.unstubAllGlobals();
});

it("keeps an authenticated deep link while its session is validating", async () => {
  useAuthStore.setState({ isValidating: true });
  let finish!: (value: unknown) => void;
  vi.stubGlobal(
    "fetch",
    vi.fn(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    ),
  );
  render(
    <AdminShell>
      <h1>Deep-linked policy</h1>
    </AdminShell>,
  );
  expect(replace).not.toHaveBeenCalled();
  expect(
    screen.queryByRole("heading", { name: "Deep-linked policy" }),
  ).toBeNull();
  finish({
    ok: true,
    json: async () => ({ user: { id: "owner", role: "org_owner" } }),
  });
  expect(
    await screen.findByRole("heading", { name: "Deep-linked policy" }),
  ).toBeVisible();
  expect(replace).not.toHaveBeenCalledWith("/login");
  vi.unstubAllGlobals();
});
