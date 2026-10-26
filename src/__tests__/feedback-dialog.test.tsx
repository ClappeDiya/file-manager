/** WS-I row 100: Help > Send feedback. */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { FeedbackDialog, FeedbackForm, MAX_FEEDBACK_CHARS, type FeedbackDestination, type FeedbackOutcome } from "@/components/feedback-dialog";
import { getDefaultCommands } from "@/components/command-palette";
import { useFeedbackDialog } from "@/stores/feedback-store";

let destination: FeedbackDestination;
let submit: (req: Record<string, unknown>) => FeedbackOutcome;
const submitted = () => mockInvoke.mock.calls.filter(([c]) => c === "feedback_submit").map(([, a]) => (a as { request: Record<string, unknown> }).request);

beforeEach(() => {
  mockInvoke.mockReset();
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  destination = { type: "console", console_url: "https://console.example.com", organization: "Acme" };
  submit = () => ({ type: "sent", id: "dgn_1", console_url: "https://console.example.com", includes_diagnostics: false });
  mockInvoke.mockImplementation(async (cmd: string, args: { request?: Record<string, unknown> }) => {
    if (cmd === "feedback_destination") return destination;
    if (cmd === "feedback_submit") return submit(args.request ?? {});
    if (cmd === "pick_folder") return "/Users/me/Desktop";
    return null;
  });
});

const write = (text: string) => fireEvent.change(screen.getByLabelText("Message"), { target: { value: text } });

describe("FeedbackForm with a console", () => {
  it("sends the message and category, with diagnostics off unless ticked", async () => {
    render(<FeedbackForm />);
    expect(await screen.findByTestId("feedback-destination")).toHaveTextContent("management console (Acme) at https://console.example.com");
    expect(screen.getByLabelText("Include diagnostics")).not.toBeChecked();
    const send = screen.getByRole("button", { name: "Send feedback" });
    expect(send).toBeDisabled();
    write("  The sync stalls  ");
    fireEvent.change(screen.getByLabelText("Category"), { target: { value: "idea" } });
    fireEvent.click(send);
    await screen.findByTestId("feedback-done");
    expect(submitted()).toEqual([{ category: "idea", message: "The sync stalls", include_diagnostics: false, save_path: null, save_only: false }]);
    expect(screen.getByRole("status")).toHaveTextContent("sent to the management console at https://console.example.com (reference dgn_1)");
    expect(screen.getByRole("status")).toHaveTextContent("does not include diagnostics");
  });

  it("includes diagnostics only when the box is ticked", async () => {
    render(<FeedbackForm />);
    await screen.findByTestId("feedback-destination");
    write("Crash on start");
    fireEvent.click(screen.getByLabelText("Include diagnostics"));
    fireEvent.click(screen.getByRole("button", { name: "Send feedback" }));
    await screen.findByTestId("feedback-done");
    expect(submitted()[0]).toMatchObject({ include_diagnostics: true });
  });

  it("does not say it was sent when the console refused, and offers a file instead", async () => {
    submit = (req) => {
      if (req.save_only) return { type: "saved", path: "/Users/me/Desktop/ufop-feedback-1.zip", size: 900, includes_diagnostics: false };
      throw { message: "The feedback was not sent: maintenance.", advice: "Check the connection." };
    };
    render(<FeedbackForm />);
    await screen.findByTestId("feedback-destination");
    write("Hello");
    fireEvent.click(screen.getByRole("button", { name: "Send feedback" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("The feedback was not sent");
    expect(screen.queryByTestId("feedback-done")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Save it to a file instead" }));
    expect(screen.getByTestId("feedback-destination")).toHaveTextContent("saved to a file instead of being sent");
    fireEvent.change(screen.getByLabelText("Folder to save the feedback"), { target: { value: "/Users/me/Desktop" } });
    fireEvent.click(screen.getByRole("button", { name: "Save feedback to file" }));
    await screen.findByTestId("feedback-done");
    expect(submitted().at(-1)).toMatchObject({ save_only: true, save_path: "/Users/me/Desktop" });
    expect(screen.getByRole("status")).toHaveTextContent("has not been sent anywhere");
  });
});

describe("FeedbackForm without a console", () => {
  beforeEach(() => {
    destination = { type: "file" };
    submit = (req) => ({ type: "saved", path: `${req.save_path}/ufop-feedback-1.zip`, size: 900, includes_diagnostics: !!req.include_diagnostics });
  });

  it("says it cannot send, asks where to save, and never claims a send", async () => {
    render(<FeedbackForm />);
    expect(await screen.findByTestId("feedback-destination")).toHaveTextContent("not connected to a management console");
    expect(screen.queryByRole("button", { name: "Send feedback" })).toBeNull();
    write("Hello");
    const save = screen.getByRole("button", { name: "Save feedback to file" });
    expect(save).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Browse…" }));
    await waitFor(() => expect(screen.getByLabelText("Folder to save the feedback")).toHaveValue("/Users/me/Desktop"));
    fireEvent.click(screen.getByLabelText("Include diagnostics"));
    fireEvent.click(save);
    await screen.findByTestId("feedback-done");
    expect(submitted()[0]).toMatchObject({ save_path: "/Users/me/Desktop", save_only: false, include_diagnostics: true });
    expect(screen.getByRole("status")).toHaveTextContent("saved to /Users/me/Desktop/ufop-feedback-1.zip");
    expect(screen.getByRole("status")).toHaveTextContent("has not been sent anywhere");
    expect(screen.getByRole("status")).toHaveTextContent("includes the diagnostics bundle");
  });
});

describe("limits and the dialog", () => {
  it("counts characters and keeps the message within the limit", async () => {
    render(<FeedbackForm />);
    await screen.findByTestId("feedback-destination");
    write("x".repeat(10));
    expect(screen.getByText(new RegExp(`10/${MAX_FEEDBACK_CHARS}`))).toBeInTheDocument();
    expect(screen.getByLabelText("Message")).toHaveAttribute("maxLength", String(MAX_FEEDBACK_CHARS));
  });

  it("opens from the store, closes on Escape-free backdrop click, and starts fresh each time", async () => {
    useFeedbackDialog.setState({ open: true });
    const { rerender } = render(<FeedbackDialog />);
    expect(await screen.findByRole("dialog", { name: "Send feedback" })).toBeInTheDocument();
    write("draft");
    fireEvent.click(screen.getByLabelText("Close feedback"));
    expect(useFeedbackDialog.getState().open).toBe(false);
    rerender(<FeedbackDialog />);
    expect(screen.queryByRole("dialog")).toBeNull();
    useFeedbackDialog.setState({ open: true });
    rerender(<FeedbackDialog />);
    expect(await screen.findByLabelText("Message")).toHaveValue("");
  });

  it("is in the command palette under Help", () => {
    const open = vi.fn();
    const entry = getDefaultCommands({ onSendFeedback: open }).find((c) => c.id === "send-feedback");
    expect(entry?.label).toBe("Help: Send Feedback…");
    entry?.action();
    expect(open).toHaveBeenCalled();
    expect(getDefaultCommands({}).some((c) => c.id === "send-feedback")).toBe(false);
  });
});
