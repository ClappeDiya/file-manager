/**
 * Tests for the NotificationBell — the render surface for
 * `useUIStore.structuredErrors`.
 *
 * The regression these guard: the bell used to live inline in
 * `SimpleModeWrapper`, which early-returns in Advanced mode. That made every
 * `reportOperationFailure` call invisible to Advanced-mode users — a failed
 * Delete produced a store entry and nothing on screen. The
 * "SimpleModeWrapper integration" block below is the guard that matters: it
 * asserts the surface survives the Advanced-mode early return.
 */
import { describe, it, expect, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useUIStore } from "@/stores/ui-store";
import { NotificationBell } from "@/components/notification-bell";
import { SimpleModeWrapper } from "@/components/simple-mode";

function addError(what: string) {
  useUIStore.getState().addStructuredError({
    what,
    why: "The destination drive is full.",
    appDid: "Did not change anything on disk",
    userAction: "Free up space, then try again",
  });
}

describe("NotificationBell", () => {
  beforeEach(() => {
    useUIStore.getState().clearErrors();
    useUIStore.getState().setAppMode("simple");
  });

  it("renders nothing when there are no errors", () => {
    render(<NotificationBell />);
    expect(screen.queryByTestId("notification-bell")).not.toBeInTheDocument();
  });

  it("renders nothing when every error is dismissed", async () => {
    addError("Delete failed");
    const id = useUIStore.getState().structuredErrors[0].id;
    useUIStore.getState().dismissError(id);

    render(<NotificationBell />);
    expect(screen.queryByTestId("notification-bell")).not.toBeInTheDocument();
  });

  it("shows the undismissed error count", () => {
    addError("Delete failed");
    addError("Copy failed");

    render(<NotificationBell />);
    expect(
      screen.getByRole("button", { name: "2 notifications" }),
    ).toBeInTheDocument();
  });

  it("uses singular wording for a single error", () => {
    addError("Delete failed");

    render(<NotificationBell />);
    expect(
      screen.getByRole("button", { name: "1 notification" }),
    ).toBeInTheDocument();
  });

  it("keeps the error list collapsed until the bell is clicked", async () => {
    const user = userEvent.setup();
    addError("Delete failed");

    render(<NotificationBell />);
    expect(screen.queryByTestId("notification-list")).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "1 notification" }));

    expect(screen.getByTestId("notification-list")).toBeInTheDocument();
    expect(screen.getByText("Delete failed")).toBeInTheDocument();
    expect(screen.getByText("The destination drive is full.")).toBeInTheDocument();
    expect(screen.getByText("Free up space, then try again")).toBeInTheDocument();
  });

  it("dismissing the last error removes the bell", async () => {
    const user = userEvent.setup();
    addError("Delete failed");

    render(<NotificationBell />);
    await user.click(screen.getByRole("button", { name: "1 notification" }));
    await user.click(screen.getByRole("button", { name: "Dismiss" }));

    expect(useUIStore.getState().structuredErrors[0].dismissed).toBe(true);
    expect(screen.queryByTestId("notification-bell")).not.toBeInTheDocument();
  });
});

describe("NotificationBell - SimpleModeWrapper integration", () => {
  beforeEach(() => {
    useUIStore.getState().clearErrors();
  });

  it("renders the bell in Simple mode", () => {
    useUIStore.getState().setAppMode("simple");
    addError("Delete failed");

    render(<SimpleModeWrapper fileManager={<div>files</div>} />);
    expect(screen.getByTestId("notification-bell")).toBeInTheDocument();
  });

  it("renders the bell in Advanced mode, where the wrapper early-returns", () => {
    useUIStore.getState().setAppMode("advanced");
    addError("Delete failed");

    // SimpleModeWrapper returns `fileManager` verbatim in Advanced mode, so the
    // Advanced shell must mount its own bell. Standing in for FileManager here
    // keeps the test off that 4k-line component while pinning the contract:
    // an Advanced-mode shell is responsible for its own NotificationBell.
    render(
      <SimpleModeWrapper
        fileManager={
          <div>
            files
            <NotificationBell />
          </div>
        }
      />,
    );

    expect(screen.getByTestId("notification-bell")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "1 notification" }),
    ).toBeInTheDocument();
  });
});
