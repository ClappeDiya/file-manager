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
    if (cmd === "connector_list_protocols") return ["sftp", "s3", "rclone"];
    if (cmd === "connection_presets")
      return [
        { id: "r2", label: "Cloudflare R2", protocol: "s3", host: "https://{account}.r2.cloudflarestorage.com", port: null, default_region: "auto", path_style: false, fields: ["account"], username: null, help: "Account ID" },
        { id: "egnyte", label: "Egnyte", protocol: "sftp", host: "ftp-{domain}.egnyte.com", port: 22, default_region: null, path_style: false, fields: ["domain"], username: "{user}${domain}", help: "" },
      ];
    if (cmd === "rclone_list_remotes") return [{ name: "gcs:", backend: "google cloud storage" }, { name: "box:", backend: "box" }];
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

  it("picks an rclone remote and saves it as the connection's remote", async () => {
    const { container } = render(<ConnectionPanel />);
    fireEvent.click(await screen.findByText("Add your first connection"));
    const form = container.querySelector("form")!;
    fireEvent.change(form.querySelector("select")!, { target: { value: "rclone" } });
    const remote = (await screen.findByLabelText("rclone remote")) as HTMLSelectElement;
    await waitFor(() => expect([...remote.options].map((o) => o.textContent)).toEqual(["Choose a remote…", "gcs: (google cloud storage)", "box: (box)"]));
    fireEvent.change(remote, { target: { value: "box:" } });
    fireEvent.submit(form);
    await waitFor(() => expect(mockInvoke.mock.calls.some(([c]) => c === "save_connection")).toBe(true));
    const args = mockInvoke.mock.calls.find(([c]) => c === "save_connection")![1];
    expect([args.protocol, args.host, args.name, args.extras.rclone_remote]).toEqual(["rclone", "box:", "box", "box:"]);
  });

  it("fills host and port from a provider preset and saves the preset id", async () => {
    const { container } = render(<ConnectionPanel />);
    fireEvent.click(await screen.findByText("Add your first connection"));
    const form = container.querySelector("form")!;
    const inputs = form.querySelectorAll("input");
    fireEvent.change(inputs[0], { target: { value: "Egnyte" } });
    fireEvent.change(form.querySelector("select")!, { target: { value: "sftp" } });
    const provider = (await screen.findByLabelText("Provider")) as HTMLSelectElement;
    expect([...provider.options].map((o) => o.textContent)).toEqual(["Custom server", "Egnyte"]);
    fireEvent.change(provider, { target: { value: "egnyte" } });
    fireEvent.change(screen.getByLabelText("Domain"), { target: { value: "acme" } });
    expect(screen.getByTestId("preset-username").textContent).toBe("Username format: your-username$acme");
    fireEvent.submit(form);
    await waitFor(() => expect(mockInvoke.mock.calls.some(([c]) => c === "save_connection")).toBe(true));
    const args = mockInvoke.mock.calls.find(([c]) => c === "save_connection")![1];
    expect([args.host, args.port, args.extras.preset]).toEqual(["ftp-acme.egnyte.com", 22, "egnyte"]);
  });

  it("offers S3 presets like R2 with their account field", async () => {
    const { container } = render(<ConnectionPanel />);
    fireEvent.click(await screen.findByText("Add your first connection"));
    const form = container.querySelector("form")!;
    fireEvent.change(form.querySelector("select")!, { target: { value: "s3" } });
    fireEvent.change(await screen.findByLabelText("Provider"), { target: { value: "r2" } });
    fireEvent.change(screen.getByLabelText("Account ID"), { target: { value: "abc123" } });
    expect([...form.querySelectorAll("input")].some((i) => i.value === "https://abc123.r2.cloudflarestorage.com")).toBe(true);
  });
});

describe("Azure connections", () => {
  it.each([
    ["azure_blob", "Azure Blob Storage"],
    ["adls_gen2", "Azure Data Lake Storage Gen2"],
    ["azure_files", "Azure Files"],
  ])("saves %s with the account as user, the secret as password and the endpoint as host", async (protocol, label) => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "list_connections" || cmd === "list_connection_groups" || cmd === "list_aws_profiles" || cmd === "connection_presets") return [];
      if (cmd === "connector_list_protocols") return ["sftp", "azure_blob", "adls_gen2", "azure_files"];
      return undefined;
    });
    const { container } = render(<ConnectionPanel />);
    fireEvent.click(await screen.findByText("Add your first connection"));
    const form = container.querySelector("form")!;
    fireEvent.change(form.querySelectorAll("input")[0], { target: { value: "Lake" } });
    const proto = form.querySelector("select")!;
    await waitFor(() => expect([...proto.options].some((o) => o.textContent === label)).toBe(true));
    fireEvent.change(proto, { target: { value: protocol } });
    // Generic host / user / password fields give way to the Azure ones.
    expect(screen.queryByText("Host")).toBeNull();
    expect(screen.getByTestId("azure-fields")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Storage account"), { target: { value: "acct" } });
    fireEvent.change(screen.getByLabelText("Sign in with"), { target: { value: "sas_token" } });
    fireEvent.change(screen.getByLabelText("SAS token"), { target: { value: "sv=2021&sig=x" } });
    fireEvent.change(screen.getByLabelText(/Endpoint/), { target: { value: "acct.core.usgovcloudapi.net" } });
    expect(screen.queryByText("Azure AD")).toBeNull();
    fireEvent.submit(form);
    await waitFor(() => expect(mockInvoke.mock.calls.some(([c]) => c === "save_connection")).toBe(true));
    const args = mockInvoke.mock.calls.find(([c]) => c === "save_connection")![1];
    expect([args.protocol, args.username, args.password, args.host]).toEqual([protocol, "acct", "sv=2021&sig=x", "acct.core.usgovcloudapi.net"]);
  });
});
