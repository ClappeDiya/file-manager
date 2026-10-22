/** Connection form: S3 role assumption and preset are saved as `extras`. */
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { ConnectionPanel } from "@/components/connection-panel";

beforeAll(() => {
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});
beforeEach(() => {
  mockInvoke.mockReset().mockImplementation(async (cmd: string) => {
    if (cmd === "list_connections" || cmd === "list_connection_groups" || cmd === "list_aws_profiles") return [];
    if (cmd === "connector_list_protocols") return ["sftp", "s3"];
    return undefined;
  });
});

describe("connection form extras", () => {
  it("saves an S3 connection with an assumed role", async () => {
    const { container } = render(<ConnectionPanel />);
    fireEvent.click(await screen.findByText("Add your first connection"));
    const form = container.querySelector("form")!;
    const [nameInput] = form.querySelectorAll("input");
    fireEvent.change(nameInput, { target: { value: "Prod bucket" } });
    fireEvent.change(form.querySelector("select")!, { target: { value: "s3" } });
    fireEvent.change(await screen.findByLabelText("Role ARN"), { target: { value: " arn:aws:iam::123456789012:role/Reader " } });
    fireEvent.change(screen.getByLabelText("External ID"), { target: { value: "ext-42" } });
    fireEvent.change(screen.getByLabelText("Session length (s)"), { target: { value: "1800" } });
    fireEvent.submit(form);
    await waitFor(() => expect(mockInvoke.mock.calls.some(([c]) => c === "save_connection")).toBe(true));
    const args = mockInvoke.mock.calls.find(([c]) => c === "save_connection")![1];
    expect(args.extras).toEqual({
      rclone_remote: null,
      preset: null,
      assume_role: { role_arn: "arn:aws:iam::123456789012:role/Reader", external_id: "ext-42", session_name: null, duration_secs: 1800 },
    });
  });
});
