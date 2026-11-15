import { useState } from "react";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { PromptDialogHost } from "@/components/prompt-dialog-host";
import { confirmText, promptText } from "@/lib/prompt-dialog";
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

it("keeps a sibling confirmation above the editor focus trap and defaults to Cancel", async () => {
  const closeEditor = vi.fn();
  const results: boolean[] = [];
  render(
    <>
      <WorkspaceDialog aria-label="Editor" onClose={closeEditor}>
        <button
          onClick={() => {
            void confirmText("Discard unsaved edits?", "Discard", true).then(
              (value) => results.push(value),
            );
          }}
        >
          Ask to close
        </button>
      </WorkspaceDialog>
      <PromptDialogHost />
    </>,
  );
  const ask = screen.getByRole("button", { name: "Ask to close" });
  await waitFor(() => expect(ask).toHaveFocus());
  fireEvent.click(ask);
  const cancel = screen.getByRole("button", { name: "Cancel" });
  await waitFor(() => expect(cancel).toHaveFocus());
  expect(
    screen.getByRole("dialog", { name: "Editor", hidden: true }).inert,
  ).toBe(true);
  const discard = screen.getByRole("button", { name: "Discard" });
  fireEvent.keyDown(cancel, { key: "Tab", shiftKey: true });
  expect(discard).toHaveFocus();
  fireEvent.keyDown(discard, { key: "Tab" });
  expect(cancel).toHaveFocus();
  fireEvent.keyDown(document, { key: "Escape" });
  await waitFor(() => expect(results).toEqual([false]));
  expect(closeEditor).not.toHaveBeenCalled();
  expect(ask).toHaveFocus();
  expect(screen.getByRole("dialog", { name: "Editor" }).inert).toBeFalsy();
});

it("focuses a nested text prompt and restores the editor when cancelled", async () => {
  const closeEditor = vi.fn();
  const results: (string | null)[] = [];
  render(
    <>
      <WorkspaceDialog aria-label="Editor" onClose={closeEditor}>
        <button
          onClick={() => {
            void promptText("File name", "draft.txt").then((value) =>
              results.push(value),
            );
          }}
        >
          Ask for name
        </button>
      </WorkspaceDialog>
      <PromptDialogHost />
    </>,
  );
  const ask = screen.getByRole("button", { name: "Ask for name" });
  await waitFor(() => expect(ask).toHaveFocus());
  fireEvent.click(ask);
  await waitFor(() =>
    expect(screen.getByRole("textbox", { name: "File name" })).toHaveFocus(),
  );
  fireEvent.keyDown(document, { key: "Escape" });
  await waitFor(() => expect(results).toEqual([null]));
  expect(closeEditor).not.toHaveBeenCalled();
  expect(ask).toHaveFocus();
});

it("keeps focus inside a loading dialog before its controls arrive", async () => {
  const close = vi.fn();
  const view = render(<WorkspaceDialog aria-label="Loading editor" onClose={close}><p>Loading file…</p></WorkspaceDialog>);
  const dialog = screen.getByRole("dialog", { name: "Loading editor" });
  await waitFor(() => expect(dialog).toHaveFocus());
  view.rerender(<WorkspaceDialog aria-label="Loading editor" onClose={close}><textarea aria-label="File contents"/></WorkspaceDialog>);
  expect(dialog).toHaveFocus();
  fireEvent.keyDown(document, { key: "Escape" }); expect(close).toHaveBeenCalledOnce();
});
