/** Connection form: S3 role assumption, preset and Swift sign-in settings are saved as `extras`. */
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
      swift: null,
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

describe("OpenStack Swift connections", () => {
  const swiftProfile = {
    id: "c1",
    name: "Lab Swift",
    protocol: "swift",
    host: "keystone.lab",
    port: 5000,
    username: "alice",
    remote_path: "/",
    group_id: null,
    created_at: "2024-01-01T00:00:00Z",
    last_used: null,
    extras: {
      assume_role: null,
      rclone_remote: null,
      preset: null,
      swift: { auth_kind: "password", auth_url: "https://keystone.lab:5000/v3", project_name: "proj", domain_name: "Corp", region: "R2", application_credential_id: null },
    },
  };

  beforeEach(() => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "list_connections") return [swiftProfile];
      if (cmd === "list_connection_groups" || cmd === "list_aws_profiles" || cmd === "connection_presets") return [];
      if (cmd === "connector_list_protocols") return ["sftp", "swift"];
      return undefined;
    });
  });

  async function saved() {
    await waitFor(() => expect(mockInvoke.mock.calls.some(([c]) => c === "save_connection")).toBe(true));
    return mockInvoke.mock.calls.find(([c]) => c === "save_connection")![1];
  }

  it("saves the Keystone settings, which were never stored before", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "list_connections" || cmd === "list_connection_groups" || cmd === "list_aws_profiles" || cmd === "connection_presets") return [];
      if (cmd === "connector_list_protocols") return ["sftp", "swift"];
      return undefined;
    });
    const { container } = render(<ConnectionPanel />);
    fireEvent.click(await screen.findByText("Add your first connection"));
    const form = container.querySelector("form")!;
    fireEvent.change(form.querySelectorAll("input")[0], { target: { value: "Swift" } });
    const proto = form.querySelector("select")!;
    await waitFor(() => expect([...proto.options].some((o) => o.value === "swift")).toBe(true));
    fireEvent.change(proto, { target: { value: "swift" } });
    expect(screen.getByTestId("swift-fields")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Auth URL"), { target: { value: " https://keystone.example.com/identity/v3 " } });
    fireEvent.change(screen.getByLabelText("Project name"), { target: { value: "proj" } });
    fireEvent.change(screen.getByLabelText("Domain"), { target: { value: "Corp" } });
    fireEvent.change(screen.getByLabelText(/Region/), { target: { value: "RegionOne" } });
    fireEvent.submit(form);
    const args = await saved();
    expect(args.extras.swift).toEqual({
      auth_kind: "password",
      auth_url: "https://keystone.example.com/identity/v3",
      project_name: "proj",
      domain_name: "Corp",
      region: "RegionOne",
      application_credential_id: null,
    });
  });

  it("shows only the fields each sign-in kind uses", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "list_connections" || cmd === "list_connection_groups" || cmd === "list_aws_profiles" || cmd === "connection_presets") return [];
      if (cmd === "connector_list_protocols") return ["sftp", "swift"];
      return undefined;
    });
    const { container } = render(<ConnectionPanel />);
    fireEvent.click(await screen.findByText("Add your first connection"));
    const form = container.querySelector("form")!;
    fireEvent.change(form.querySelectorAll("input")[0], { target: { value: "Swift" } });
    const proto = form.querySelector("select")!;
    await waitFor(() => expect([...proto.options].some((o) => o.value === "swift")).toBe(true));
    fireEvent.change(proto, { target: { value: "swift" } });
    fireEvent.change(screen.getByLabelText("Sign in with"), { target: { value: "temp_auth" } });
    expect(screen.queryByLabelText(/Region/)).toBeNull();
    expect(screen.queryByLabelText("Project name")).toBeNull();
    fireEvent.change(screen.getByLabelText("Sign in with"), { target: { value: "application_credential" } });
    expect(screen.queryByLabelText("Project name")).toBeNull();
    fireEvent.change(screen.getByLabelText("Application credential ID"), { target: { value: "ac-123" } });
    // With an auth URL the host is optional.
    fireEvent.change(screen.getByLabelText("Auth URL"), { target: { value: "https://keystone.example.com/v3" } });
    fireEvent.submit(form);
    expect((await saved()).extras.swift).toEqual({
      auth_kind: "application_credential",
      auth_url: "https://keystone.example.com/v3",
      project_name: null,
      domain_name: null,
      region: null,
      application_credential_id: "ac-123",
    });
  });

  it("loads the saved settings when editing", async () => {
    render(<ConnectionPanel />);
    fireEvent.click(await screen.findByTitle("Edit"));
    expect((screen.getByLabelText("Auth URL") as HTMLInputElement).value).toBe("https://keystone.lab:5000/v3");
    expect((screen.getByLabelText("Project name") as HTMLInputElement).value).toBe("proj");
    expect((screen.getByLabelText("Domain") as HTMLInputElement).value).toBe("Corp");
    expect((screen.getByLabelText(/Region/) as HTMLInputElement).value).toBe("R2");
    fireEvent.change(screen.getByLabelText(/Region/), { target: { value: "R3" } });
    fireEvent.submit(document.querySelector("form")!);
    const args = await saved();
    expect([args.connectionId ?? args.id, args.extras.swift.region, args.extras.swift.project_name]).toEqual(["c1", "R3", "proj"]);
  });
});
