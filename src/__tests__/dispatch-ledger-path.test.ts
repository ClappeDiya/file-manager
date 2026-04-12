import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  dispatchLedgerPath,
  type LedgerDispatchHandlers,
} from "@/lib/ledger-dispatch";

vi.mock("@/hooks/use-tauri", () => ({
  tauriInvokeSafe: vi.fn(),
}));

import { tauriInvokeSafe } from "@/hooks/use-tauri";
const mockTauriInvokeSafe = vi.mocked(tauriInvokeSafe);

function handlers(
  overrides: Partial<LedgerDispatchHandlers> = {},
): LedgerDispatchHandlers {
  return {
    isArchivePath: () => false,
    isPlainTextPath: () => false,
    isPreviewablePath: () => false,
    onNavigateDir: vi.fn(),
    onOpenArchive: vi.fn(),
    onOpenFile: vi.fn(),
    onOpenPreview: vi.fn(),
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("dispatchLedgerPath", () => {
  it("navigates to the path when metadata says it's a directory", async () => {
    mockTauriInvokeSafe.mockResolvedValue({ is_dir: true, size: 0 });
    const h = handlers();
    await dispatchLedgerPath("/home/docs", h);
    expect(h.onNavigateDir).toHaveBeenCalledWith("/home/docs");
    expect(h.onOpenArchive).not.toHaveBeenCalled();
    expect(h.onOpenFile).not.toHaveBeenCalled();
    expect(h.onOpenPreview).not.toHaveBeenCalled();
  });

  it("opens archive when extension matches and handler exists", async () => {
    mockTauriInvokeSafe.mockResolvedValue({ is_dir: false, size: 500 });
    const h = handlers({ isArchivePath: () => true });
    await dispatchLedgerPath("/data/backup.zip", h);
    expect(h.onOpenArchive).toHaveBeenCalledWith("/data/backup.zip");
    expect(h.onNavigateDir).not.toHaveBeenCalled();
  });

  it("opens text file when extension matches and size is within limit", async () => {
    mockTauriInvokeSafe.mockResolvedValue({ is_dir: false, size: 1024 });
    const h = handlers({ isPlainTextPath: () => true });
    await dispatchLedgerPath("/src/main.rs", h);
    expect(h.onOpenFile).toHaveBeenCalledWith("/src/main.rs", 1024);
  });

  it("skips text editor when file exceeds 1 MB", async () => {
    const oversized = 1024 * 1024 + 1;
    mockTauriInvokeSafe.mockResolvedValue({ is_dir: false, size: oversized });
    const h = handlers({ isPlainTextPath: () => true });
    await dispatchLedgerPath("/src/huge.log", h);
    expect(h.onOpenFile).not.toHaveBeenCalled();
    expect(h.onNavigateDir).toHaveBeenCalledWith("/src");
  });

  it("opens preview when extension matches and handler exists", async () => {
    mockTauriInvokeSafe.mockResolvedValue({ is_dir: false, size: 5000 });
    const h = handlers({ isPreviewablePath: () => true });
    await dispatchLedgerPath("/photos/cat.png", h);
    expect(h.onOpenPreview).toHaveBeenCalledWith("/photos/cat.png");
  });

  it("archive takes priority over text and preview", async () => {
    mockTauriInvokeSafe.mockResolvedValue({ is_dir: false, size: 100 });
    const h = handlers({
      isArchivePath: () => true,
      isPlainTextPath: () => true,
      isPreviewablePath: () => true,
    });
    await dispatchLedgerPath("/mixed.tar.gz", h);
    expect(h.onOpenArchive).toHaveBeenCalled();
    expect(h.onOpenFile).not.toHaveBeenCalled();
    expect(h.onOpenPreview).not.toHaveBeenCalled();
  });

  it("text takes priority over preview", async () => {
    mockTauriInvokeSafe.mockResolvedValue({ is_dir: false, size: 100 });
    const h = handlers({
      isPlainTextPath: () => true,
      isPreviewablePath: () => true,
    });
    await dispatchLedgerPath("/readme.md", h);
    expect(h.onOpenFile).toHaveBeenCalled();
    expect(h.onOpenPreview).not.toHaveBeenCalled();
  });

  it("falls back to parent directory when no predicate matches", async () => {
    mockTauriInvokeSafe.mockResolvedValue({ is_dir: false, size: 100 });
    const h = handlers();
    await dispatchLedgerPath("/data/unknown.bin", h);
    expect(h.onNavigateDir).toHaveBeenCalledWith("/data");
  });

  it("falls back to root when parent extraction yields empty", async () => {
    mockTauriInvokeSafe.mockResolvedValue({ is_dir: false, size: 0 });
    const h = handlers();
    await dispatchLedgerPath("/singlefile", h);
    expect(h.onNavigateDir).toHaveBeenCalledWith("/");
  });

  it("navigates to the raw path when metadata IPC fails and no extension hint", async () => {
    mockTauriInvokeSafe.mockResolvedValue(null);
    const h = handlers();
    await dispatchLedgerPath("/mystery", h);
    expect(h.onNavigateDir).toHaveBeenCalledWith("/mystery");
  });

  it("still routes by extension when metadata IPC returns null", async () => {
    mockTauriInvokeSafe.mockResolvedValue(null);
    const h = handlers({ isArchivePath: () => true });
    await dispatchLedgerPath("/backup.zip", h);
    expect(h.onOpenArchive).toHaveBeenCalledWith("/backup.zip");
  });

  it("falls back gracefully when tauriInvokeSafe throws", async () => {
    mockTauriInvokeSafe.mockRejectedValue(new Error("IPC timeout"));
    const h = handlers();
    await dispatchLedgerPath("/broken", h);
    expect(h.onNavigateDir).toHaveBeenCalledWith("/broken");
  });

  it("uses size 0 as fallback when metadata is null but text predicate matches", async () => {
    mockTauriInvokeSafe.mockResolvedValue(null);
    const h = handlers({ isPlainTextPath: () => true });
    await dispatchLedgerPath("/notes.txt", h);
    expect(h.onOpenFile).toHaveBeenCalledWith("/notes.txt", 0);
  });

  it("does not call onOpenArchive when handler is not provided", async () => {
    mockTauriInvokeSafe.mockResolvedValue({ is_dir: false, size: 100 });
    const h = handlers({
      isArchivePath: () => true,
      onOpenArchive: undefined,
    });
    await dispatchLedgerPath("/backups/data.zip", h);
    expect(h.onNavigateDir).toHaveBeenCalledWith("/backups");
  });

  it("does not call onOpenFile when handler is not provided", async () => {
    mockTauriInvokeSafe.mockResolvedValue({ is_dir: false, size: 100 });
    const h = handlers({
      isPlainTextPath: () => true,
      onOpenFile: undefined,
    });
    await dispatchLedgerPath("/readme.md", h);
    expect(h.onNavigateDir).toHaveBeenCalledWith("/");
  });

  it("text at exactly 1 MB is accepted", async () => {
    mockTauriInvokeSafe.mockResolvedValue({
      is_dir: false,
      size: 1024 * 1024,
    });
    const h = handlers({ isPlainTextPath: () => true });
    await dispatchLedgerPath("/exact-limit.txt", h);
    expect(h.onOpenFile).toHaveBeenCalledWith(
      "/exact-limit.txt",
      1024 * 1024,
    );
  });
});
