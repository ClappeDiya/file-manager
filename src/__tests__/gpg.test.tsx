/** OpenPGP UI: dialog modes, recipients vs passphrase, results, keys, menu. */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { GpgDialog } from "@/components/gpg-dialog";
import { GpgKeysSection } from "@/components/gpg-keys-section";
import { getFileContextMenuItems } from "@/components/context-menu";
import { daysToExpiry, useGpgDialog, type GpgKey } from "@/stores/gpg-store";

const now = Math.floor(Date.now() / 1000);
const key = (over: Partial<GpgKey>): GpgKey => ({
  fingerprint: "F".repeat(40), key_id: "AAAABBBBCCCCDDDD", user_ids: ["Alice <a@x.org>"], created: now - 100, expires: now + 400 * 86400,
  expired: false, revoked: false, can_encrypt: true, can_sign: true, secret: true, algorithm: "EdDSA", ...over,
});
const ALICE = key({});
const BOB = key({ fingerprint: "B".repeat(40), key_id: "BOB0000011112222", user_ids: ["Bob <b@x.org>"], secret: false, can_sign: false, expires: now + 10 * 86400 });
const OLD = key({ fingerprint: "0".repeat(40), user_ids: ["Old <o@x.org>"], expired: true });
let available = true;

beforeEach(() => {
  available = true;
  useGpgDialog.getState().close();
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  mockInvoke.mockReset().mockImplementation(async (cmd: string, a: Record<string, unknown>) => {
    switch (cmd) {
      case "gpg_status":
        return available ? { available: true, version: "gpg (GnuPG) 2.4.4", error: null } : { available: false, version: null, error: "Could not start gpg" };
      case "gpg_list_keys":
        return [ALICE, BOB, OLD];
      case "gpg_encrypt_files":
        return (a.paths as string[]).map((p) => ({ input: p, output: `${p}.gpg`, signature: null, error: null }));
      case "gpg_decrypt_files":
        return [{ input: "/d/a.txt.gpg", output: "/d/a.txt", signature: { valid: true, key_expired: false, key_revoked: false, missing_key: false, message: "Good signature from Alice", signed_at: null, key_id: null, fingerprint: null, user_id: null }, error: null }];
      case "gpg_verify":
        return { valid: false, missing_key: false, key_expired: false, key_revoked: false, message: "BAD signature claimed to be from Alice", signed_at: null, key_id: null, fingerprint: null, user_id: null };
      case "gpg_import_keys":
        return 1;
      default:
        return null;
    }
  });
});

describe("GpgDialog", () => {
  it("encrypts to chosen keys only (expired keys are not offered)", async () => {
    render(<GpgDialog />);
    act(() => useGpgDialog.getState().open("encrypt", ["/d/a.txt", "/d/b.txt"]));
    expect(await screen.findByText(/Bob <b@x.org>/)).toBeTruthy();
    expect(screen.queryByText(/Old <o@x.org>/)).toBeNull();
    expect(screen.getByText(/expires in 9d|expires in 10d/)).toBeTruthy();
    const run = screen.getByRole("button", { name: "Encrypt" }) as HTMLButtonElement;
    expect(run.disabled).toBe(true);
    fireEvent.click(screen.getByLabelText(/Bob/));
    fireEvent.click(screen.getByLabelText(/Text output/));
    fireEvent.click(run);
    await screen.findByText("→ a.txt.gpg");
    expect(mockInvoke).toHaveBeenCalledWith("gpg_encrypt_files", { paths: ["/d/a.txt", "/d/b.txt"], options: { recipients: ["B".repeat(40)], armor: true, sign_with: null }, passphrase: null });
  });

  it("encrypts with a passphrase instead", async () => {
    render(<GpgDialog />);
    act(() => useGpgDialog.getState().open("encrypt", ["/d/a.txt"]));
    fireEvent.click(await screen.findByLabelText("With a passphrase"));
    fireEvent.change(screen.getByLabelText("Passphrase"), { target: { value: "s3cret" } });
    fireEvent.click(screen.getByRole("button", { name: "Encrypt" }));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("gpg_encrypt_files", expect.objectContaining({ options: expect.objectContaining({ recipients: [] }), passphrase: "s3cret" })));
    expect((screen.getByLabelText("Passphrase") as HTMLInputElement).value).toBe("", "passphrase cleared after use");
  });

  it("decrypts and reports the signature", async () => {
    render(<GpgDialog />);
    act(() => useGpgDialog.getState().open("decrypt", ["/d/a.txt.gpg"]));
    fireEvent.click(await screen.findByRole("button", { name: "Decrypt" }));
    expect(await screen.findByText(/Good signature from Alice/)).toBeTruthy();
  });

  it("shows a bad signature as a failure", async () => {
    render(<GpgDialog />);
    act(() => useGpgDialog.getState().open("verify", ["/d/a.txt.sig"]));
    fireEvent.click(await screen.findByRole("button", { name: "Verify signature" }));
    const line = await screen.findByText(/BAD signature/);
    expect(line.closest("p")?.className).toContain("text-red-500");
  });

  it("explains when GnuPG is missing", async () => {
    available = false;
    render(<GpgDialog />);
    act(() => useGpgDialog.getState().open("sign", ["/d/a.txt"]));
    expect((await screen.findByRole("alert")).textContent).toContain("Install GnuPG");
  });
});

describe("GpgKeysSection", () => {
  it("lists keys with expiry and imports pasted keys", async () => {
    render(<GpgKeysSection />);
    const expiry = await screen.findAllByTestId("key-expiry");
    expect(expiry.map((e) => e.textContent)).toEqual([expect.stringMatching(/^expires /), expect.stringMatching(/^expires in (9|10) days/), "expired"]);
    fireEvent.change(screen.getByLabelText(/Import a key/), { target: { value: "-----BEGIN PGP PUBLIC KEY BLOCK-----" } });
    fireEvent.click(screen.getByText("Import pasted key"));
    expect(await screen.findByText("Imported 1 key.")).toBeTruthy();
    expect(mockInvoke).toHaveBeenCalledWith("gpg_import_keys", { path: null, text: "-----BEGIN PGP PUBLIC KEY BLOCK-----" });
  });

  it("computes days to expiry", () => {
    expect(daysToExpiry(key({ expires: null }))).toBeNull();
    expect(daysToExpiry(key({ expires: 86400 * 3 }), 0)).toBe(3);
  });
});

describe("context menu", () => {
  const base = { hasSelection: true, isDirectory: false, selectionCount: 1, onCopy() {}, onCut() {}, onPaste() {}, onDelete() {}, onRename() {}, onDuplicate() {} };
  it("offers decrypt and verify only for matching files", () => {
    const ids = (o: object) => getFileContextMenuItems({ ...base, ...o } as Parameters<typeof getFileContextMenuItems>[0]).map((i) => i.id);
    const plain = ids({ onGpg: () => {} });
    expect(plain).toEqual(expect.arrayContaining(["gpg-encrypt", "gpg-sign"]));
    expect(plain).not.toContain("gpg-decrypt");
    expect(ids({ onGpg: () => {}, gpgDecrypt: true, gpgVerify: true })).toEqual(expect.arrayContaining(["gpg-decrypt", "gpg-verify"]));
    expect(ids({ onGpg: () => {}, isDirectory: true })).not.toContain("gpg-encrypt");
  });
});
