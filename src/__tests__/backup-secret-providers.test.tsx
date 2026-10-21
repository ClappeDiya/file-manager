/**
 * Secret providers UI: provider shapes match the Rust serde types (flattened
 * `type` tag), credentials go only to the secure store, tests and the job
 * placeholder check call the right commands.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { JobSecretProviders, SecretProvidersEditor, newProvider } from "@/components/backup/backup-secret-providers";
import type { SecretSettings } from "@/stores/backup-store";

let stored: SecretSettings;
beforeEach(() => {
  stored = { providers: [], cache: "memory", cache_ttl_secs: 3600 };
  mockInvoke.mockReset();
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  mockInvoke.mockImplementation(async (cmd: string, a: Record<string, unknown>) => {
    if (cmd === "backup_get_secret_settings") return stored;
    if (cmd === "backup_set_secret_settings") return (stored = a.settings as SecretSettings);
    if (cmd === "backup_has_destination_secret") return false;
    if (cmd === "backup_test_secret_providers")
      return stored.providers.map((p) => ({ provider_id: p.id, provider: p.name, kind: p.type, ok: true, has_credential: true, latency_ms: 4.2, found: ["db_pass"], missing: [], errors: [] }));
    if (cmd === "backup_job_placeholders")
      return { providers: ["Vault"], placeholders: [{ name: "db_pass", provider: "Vault", from_cache: false, error: null }, { name: "nope", provider: null, from_cache: false, error: "Secret '$nope' was not found in: Vault" }] };
    return null;
  });
});

describe("secret providers editor", () => {
  it("adds a Vault provider, stores its token separately, saves and tests", async () => {
    render(<SecretProvidersEditor />);
    fireEvent.change(await screen.findByTestId("sp-add"), { target: { value: "vault" } });
    fireEvent.change(screen.getByPlaceholderText("https://vault.example.com:8200"), { target: { value: "http://127.0.0.1:8200" } });
    fireEvent.change(screen.getByPlaceholderText("ufop/prod"), { target: { value: "ufop" } });
    fireEvent.change(screen.getByTestId("sp-secret"), { target: { value: "s.token" } });
    fireEvent.click(screen.getByTestId("sp-secret-save"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("backup_set_destination_secret", expect.objectContaining({ secret: "s.token" })));
    fireEvent.click(screen.getByTestId("sp-save"));
    await waitFor(() => expect(stored.providers).toHaveLength(1));
    expect(stored.providers[0]).toMatchObject({ type: "vault", address: "http://127.0.0.1:8200", mount: "secret", path: "ufop", kv_version: 2, enabled: true, global: true, tls: { pinned_sha256: [] } });
    expect(JSON.stringify(stored)).not.toContain("s.token");
    fireEvent.change(screen.getByTestId("sp-probe"), { target: { value: "db_pass" } });
    fireEvent.click(screen.getByTestId("sp-test"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("backup_test_secret_providers", { providerId: null, names: ["db_pass"] }));
    expect(await screen.findByText(/found db_pass/)).toBeInTheDocument();
  });

  it("defaults for every provider kind", () => {
    expect(newProvider("aws_secrets_manager")).toMatchObject({ type: "aws_secrets_manager", region: "us-east-1", secret_id: "", endpoint: "" });
    expect(newProvider("azure_key_vault")).toMatchObject({ type: "azure_key_vault", vault_url: "", authority: "" });
    expect(newProvider("pass")).toMatchObject({ type: "pass", executable: "", prefix: "" });
    expect(newProvider("env")).toMatchObject({ type: "env", prefix: "" });
  });
});

describe("job secrets", () => {
  it("selects non-global providers and checks placeholders", async () => {
    stored.providers = [{ ...newProvider("file"), id: "f1", name: "Team file", global: false } as never, { ...newProvider("env"), id: "e1", name: "Env" } as never];
    const onChange = vi.fn();
    render(<JobSecretProviders jobId="j1" value={[]} onChange={onChange} />);
    fireEvent.click(await screen.findByLabelText("Team file"));
    expect(onChange).toHaveBeenCalledWith(["f1"]);
    fireEvent.click(screen.getByTestId("job-placeholders-check"));
    expect(await screen.findByText(/\$db_pass: from Vault/)).toBeInTheDocument();
    expect(screen.getByText(/was not found/)).toBeInTheDocument();
  });
});
