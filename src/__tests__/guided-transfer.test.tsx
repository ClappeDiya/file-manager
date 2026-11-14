import { beforeEach, expect, it, vi } from "vitest";
import {
  render,
  screen,
  fireEvent,
  waitFor,
  cleanup,
} from "@testing-library/react";
import { StartTransferWizard } from "@/components/guided-flows/operation-flows";
import { BackupFolderWizard } from "@/components/guided-flows/utility-flows";
import { useUIStore } from "@/stores/ui-store";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
beforeEach(() => {
  cleanup();
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    value: {},
    configurable: true,
  });
  useUIStore.setState({ activeGuidedFlow: "start-transfer", activityFeed: [] });
  invoke.mockReset();
});
function next() {
  fireEvent.click(screen.getByRole("button", { name: "Next" }));
}
function readyTransfer() {
  render(<StartTransferWizard />);
  fireEvent.change(screen.getByRole("textbox", { name: "Source" }), {
    target: { value: "/source" },
  });
  next();
  fireEvent.change(screen.getByRole("textbox", { name: "Destination" }), {
    target: { value: "/backup" },
  });
  next();
  next();
}
it("submits the operation and waits before recording completed work", async () => {
  let finish!: (value: unknown) => void;
  invoke.mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  readyTransfer();
  fireEvent.click(screen.getByRole("button", { name: "Start Transfer" }));
  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith(
      "run_local_transfer",
      expect.objectContaining({ source: "/source", destination: "/backup" }),
    ),
  );
  expect(useUIStore.getState().activeGuidedFlow).toBe("start-transfer");
  expect(screen.getByRole("button", { name: /Processing/ })).toBeDisabled();
  finish({
    files_transferred: 2,
    bytes_transferred: 14,
    files_skipped: 0,
    verified: false,
  });
  await screen.findByText(/2 files copied/);
});
it("keeps paths and shows a failed copy without recording success", async () => {
  invoke.mockRejectedValue({
    message: "Destination is read-only",
    advice: "Choose a writable folder.",
  });
  readyTransfer();
  fireEvent.click(screen.getByRole("button", { name: "Start Transfer" }));
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Destination is read-only",
  );
  expect(screen.getByText("/source")).toBeVisible();
  expect(useUIStore.getState().activeGuidedFlow).toBe("start-transfer");
});
it("executes a one-time verified backup and reports the actual result", async () => {
  invoke.mockResolvedValue({
    files_transferred: 2,
    bytes_transferred: 14,
    files_skipped: 0,
    verified: true,
  });
  render(<BackupFolderWizard />);
  fireEvent.change(screen.getByRole("textbox", { name: "Folder" }), {
    target: { value: "/source" },
  });
  next();
  fireEvent.change(
    screen.getByRole("textbox", { name: "Backup destination" }),
    { target: { value: "/backup" } },
  );
  next();
  next();
  fireEvent.click(screen.getByRole("button", { name: "Start Backup" }));
  await waitFor(() =>
    expect(invoke).toHaveBeenCalledWith(
      "run_local_transfer",
      expect.objectContaining({
        source: "/source",
        destination: "/backup",
        verify: true,
        moveSource: false,
      }),
    ),
  );
  expect(await screen.findByText(/2 files copied/)).toBeVisible();
  expect(screen.getByText(/Verified/)).toBeVisible();
});

it("keeps unavailable scheduled backups from being represented as active", () => {
  render(<BackupFolderWizard />);
  fireEvent.change(screen.getByRole("textbox", { name: "Folder" }), {
    target: { value: "/source" },
  });
  next();
  fireEvent.change(
    screen.getByRole("textbox", { name: "Backup destination" }),
    { target: { value: "/backup" } },
  );
  next();
  expect(
    screen.getByRole("button", { name: "Daily (unavailable)" }),
  ).toBeDisabled();
  expect(
    screen.getByRole("button", { name: "Weekly (unavailable)" }),
  ).toBeDisabled();
  expect(invoke).not.toHaveBeenCalled();
});
it("submits only once while the operation is pending", async () => {
  invoke.mockImplementation(() => new Promise(() => {}));
  readyTransfer();
  const button = screen.getByRole("button", { name: "Start Transfer" });
  fireEvent.click(button);
  fireEvent.click(button);
  expect(invoke).toHaveBeenCalledTimes(1);
});
