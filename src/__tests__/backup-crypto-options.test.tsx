/** Encryption method options and the file encryption tool send engine-shaped values. */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { BackupAdvancedOptions } from "@/components/backup/backup-advanced-options";
import { BackupFileCrypt } from "@/components/backup/backup-file-crypt";
import { KDF_PRESETS, newBackupJob, type BackupOptions } from "@/stores/backup-store";

beforeEach(() => {
  mockInvoke.mockReset();
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});

describe("encryption method", () => {
  it("picks GPG with recipients and Argon2id presets", () => {
    let o: BackupOptions = newBackupJob().options;
    const onChange = vi.fn((p: Partial<BackupOptions>) => (o = { ...o, ...p }));
    const { rerender } = render(<BackupAdvancedOptions options={o} locked={false} onChange={onChange} />);
    fireEvent.change(screen.getByTestId("opt-kdf"), { target: { value: "sensitive" } });
    expect(o.kdf).toEqual(KDF_PRESETS.sensitive);
    rerender(<BackupAdvancedOptions options={o} locked={false} onChange={onChange} />);
    fireEvent.change(screen.getByTestId("opt-kdf"), { target: { value: "app" } });
    expect(o.kdf).toBeNull();
    fireEvent.change(screen.getByTestId("opt-encryption-module"), { target: { value: "gpg" } });
    rerender(<BackupAdvancedOptions options={o} locked={false} onChange={onChange} />);
    fireEvent.change(screen.getByTestId("opt-gpg-recipients"), { target: { value: "alice@example.org\n0xDEADBEEF\n" } });
    expect(o).toMatchObject({ encryption_module: "gpg", gpg: { recipients: ["alice@example.org", "0xDEADBEEF"], homedir: "", executable: "" } });
  });

  it("is locked once the backup exists", () => {
    render(<BackupAdvancedOptions options={newBackupJob().options} locked onChange={() => {}} />);
    expect(screen.getByTestId("opt-encryption-module")).toBeDisabled();
  });
});

describe("file encryption tool", () => {
  it("encrypts with a preset and decrypts a volume with its header", async () => {
    mockInvoke.mockResolvedValue({ input_bytes: 10, output_bytes: 60, chunks: 1 });
    render(<BackupFileCrypt />);
    fireEvent.change(screen.getByTestId("crypt-input"), { target: { value: "/tmp/a.txt" } });
    fireEvent.change(screen.getByTestId("crypt-output"), { target: { value: "/tmp/a.ufopenc" } });
    fireEvent.change(screen.getByTestId("crypt-pass"), { target: { value: "pw" } });
    fireEvent.click(screen.getByTestId("crypt-run"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("backup_crypt_file", { decrypt: false, input: "/tmp/a.txt", output: "/tmp/a.ufopenc", passphrase: "pw", preset: "default" }));
    expect(await screen.findByText(/Wrote \/tmp\/a.ufopenc/)).toBeInTheDocument();
    fireEvent.change(screen.getByTestId("crypt-mode"), { target: { value: "volume" } });
    expect(screen.getByTestId("crypt-run")).toBeDisabled();
    fireEvent.change(screen.getByTestId("crypt-header"), { target: { value: "/d/ufop-repo.json" } });
    fireEvent.click(screen.getByTestId("crypt-run"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("backup_decrypt_volume", { volume: "/tmp/a.txt", header: "/d/ufop-repo.json", output: "/tmp/a.ufopenc", passphrase: null }));
  });
});
