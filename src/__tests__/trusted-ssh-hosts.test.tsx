/**
 * Trusted SSH hosts: lists persisted host keys grouped per host and
 * forgets them only after an explicit confirmation.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { TrustedSshHostsSection, type KnownHostEntry } from "@/components/trusted-ssh-hosts-section";

const hosts: KnownHostEntry[] = [
  { hostname: "build.example.com", port: 22, key_type: "ssh-ed25519", fingerprint: "SHA256:aaa", added_at: "2026-10-01T00:00:00Z" },
  { hostname: "build.example.com", port: 22, key_type: "ssh-rsa", fingerprint: "SHA256:bbb", added_at: "2026-10-01T00:00:00Z" },
  { hostname: "git.example.com", port: 2222, key_type: "ssh-ed25519", fingerprint: "SHA256:ccc", added_at: "2026-10-02T00:00:00Z" },
];

beforeEach(() => {
  mockInvoke.mockReset();
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});

describe("trusted SSH hosts", () => {
  it("groups key types per host and shows non-default ports", async () => {
    mockInvoke.mockResolvedValue(hosts);
    render(<TrustedSshHostsSection />);
    expect(await screen.findByText("build.example.com")).toBeInTheDocument();
    expect(screen.getByText("git.example.com:2222")).toBeInTheDocument();
    expect(screen.getByText("ssh-ed25519 SHA256:aaa")).toBeInTheDocument();
    expect(screen.getByText("ssh-rsa SHA256:bbb")).toBeInTheDocument();
    expect(screen.getAllByRole("listitem")).toHaveLength(2);
  });

  it("forgets a host only after confirmation", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => (cmd === "list_known_hosts" ? hosts : 2));
    render(<TrustedSshHostsSection />);
    await screen.findByText("build.example.com");
    fireEvent.click(screen.getAllByText("Forget")[0]);
    expect(mockInvoke).not.toHaveBeenCalledWith("forget_known_host", expect.anything());
    expect(screen.getByText(/rebuilt or re-keyed/)).toBeInTheDocument();
    fireEvent.click(screen.getByText("Forget keys"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("forget_known_host", { host: "build.example.com", port: 22 }),
    );
  });

  it("explains the empty state", async () => {
    mockInvoke.mockResolvedValue([]);
    render(<TrustedSshHostsSection />);
    expect(await screen.findByText(/No hosts trusted yet/)).toBeInTheDocument();
  });
});
