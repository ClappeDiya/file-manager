/**
 * Portable mode (K131): the Settings header badge and the storage details
 * read `get_portable_status`; nothing is shown as "on" outside portable mode.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { PortableModeBadge, PortableModeDetails, usePortableStatus, type PortableStatus } from "@/components/portable-mode-status";

const ON: PortableStatus = {
  enabled: true,
  root: "/media/stick/ufop-data",
  source: "marker",
  data_dir: "/media/stick/ufop-data/data",
  config_dir: "/media/stick/ufop-data/config",
  cache_dir: "/media/stick/ufop-data/cache",
  credential_store: "vault_file",
};
const OFF: PortableStatus = { ...ON, enabled: false, root: null, source: null, data_dir: "/home/u/.local/share/unified-file-ops", credential_store: "keychain" };

function Harness() {
  const s = usePortableStatus();
  return (
    <>
      <PortableModeBadge status={s} />
      <PortableModeDetails status={s} />
    </>
  );
}

beforeEach(() => {
  mockInvoke.mockReset();
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});

describe("portable mode status", () => {
  it("shows the badge, folders and the vault when on", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => (cmd === "get_portable_status" ? ON : null));
    render(<Harness />);
    expect(await screen.findByTestId("portable-mode-badge")).toHaveTextContent("Portable mode: on (/media/stick/ufop-data)");
    expect(screen.getByTestId("portable-mode-details")).toHaveTextContent("the “portable” file next to the app");
    expect(screen.getByTestId("portable-credentials")).toHaveTextContent("encrypted vault file");
    expect(screen.getByText(/UFOP_PORTABLE_PASSPHRASE/)).toBeInTheDocument();
  });

  it("has no badge when off and explains how to turn it on", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => (cmd === "get_portable_status" ? OFF : null));
    render(<Harness />);
    expect(await screen.findByText(/Portable mode: off/)).toBeInTheDocument();
    expect(screen.queryByTestId("portable-mode-badge")).toBeNull();
    expect(screen.getByTestId("portable-credentials")).toHaveTextContent("system keychain");
  });

  it("falls back quietly without a backend", () => {
    render(<PortableModeDetails status={null} />);
    expect(screen.getByText(/desktop app/)).toBeInTheDocument();
    render(<PortableModeBadge status={null} />);
    expect(screen.queryByTestId("portable-mode-badge")).toBeNull();
  });
});
