import { useState } from "react";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, it } from "vitest";
import { WorkspaceDialog } from "@/components/workspace-dialog";

it("moves focus inside, wraps Tab, isolates the page, and restores the trigger on Escape", async () => {
  function Fixture() {
    const [open, setOpen] = useState(false);
    return (
      <>
        <button onClick={() => setOpen(true)}>Schedules</button>
        {open && (
          <WorkspaceDialog
            aria-label="Schedules"
            onClose={() => setOpen(false)}
          >
            <button>Close schedules</button>
            <input aria-label="Schedule name" />
            <button>New calendar</button>
          </WorkspaceDialog>
        )}
      </>
    );
  }
  render(<Fixture />);
  const trigger = screen.getByRole("button", {
    name: "Schedules",
  });
  trigger.focus();
  fireEvent.click(trigger);
  const first = screen.getByRole("button", { name: "Close schedules" });
  const last = screen.getByRole("button", { name: "New calendar" });
  await waitFor(() => expect(first).toHaveFocus());
  expect(trigger.inert).toBe(true);
  first.focus();
  fireEvent.keyDown(first, { key: "Tab", shiftKey: true });
  expect(last).toHaveFocus();
  fireEvent.keyDown(last, { key: "Tab" });
  expect(first).toHaveFocus();
  trigger.focus();
  expect(first).toHaveFocus();
  fireEvent.keyDown(document, { key: "Escape" });
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(trigger).toHaveFocus();
  expect(trigger.inert).toBeFalsy();
});
