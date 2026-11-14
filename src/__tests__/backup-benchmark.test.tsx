/**
 * Benchmark (K56) and read-only inspection (K111) in the desktop UI: the
 * benchmark sends `BenchOptions` to `backup_bench` and renders the report
 * with the suggested `ufop backup run` flags; the maintenance view runs the
 * `inspect_*` operations (Advanced only).
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { useUIStore } from "@/stores/ui-store";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { BackupBenchmark } from "@/components/backup/backup-benchmark";
import { BackupMaintenance } from "@/components/backup/backup-maintenance";
import { benchRunFlags, DEFAULT_BENCH_OPTIONS, type BenchReport } from "@/stores/backup-bench";
import { newBackupJob } from "@/stores/backup-store";

const REPORT: BenchReport = {
  sample: { source: "generated", bytes: 16 * 1024 * 1024, files: 0, seed: 1 },
  repeat: 3,
  threads: 2,
  block_size: 102400,
  compression: [
    { name: "zstd-3", codec: "zstd", level: 5, native_level: 3, zip_only: false, compressed_bytes: 8_000_000, ratio: 0.48, compress_mb_per_sec: 310.2, decompress_mb_per_sec: 900, rank: 1 },
    { name: "bzip2-9", codec: "bzip2", level: 9, native_level: 9, zip_only: true, compressed_bytes: 7_000_000, ratio: 0.42, compress_mb_per_sec: 12, decompress_mb_per_sec: 30, rank: 2 },
    { name: "stored", codec: "stored", level: 0, native_level: 0, zip_only: false, compressed_bytes: 16_777_216, ratio: 1, compress_mb_per_sec: 5000, decompress_mb_per_sec: 5000, rank: 3 },
  ],
  ciphers: [{ cipher: "aes256_gcm", seal_mb_per_sec: 1500, open_mb_per_sec: 1600 }],
  kdf: [{ name: "default", params: { memory_kib: 65536, iterations: 3, parallelism: 4 }, millis: 180 }],
  kdf_suggestion: { name: "suggested", params: { memory_kib: 65536, iterations: 17, parallelism: 4 }, millis: 1010 },
  kdf_target_ms: 1000,
  chunkers: [{ name: "fastcdc-100K", spec: { type: "fastcdc", min: 25600, avg: 102400, max: 409600 }, mb_per_sec: 700, chunks: 160, min_size: 25600, max_size: 409600, avg_size: 104857, stddev: 50000, p10: 40000, p50: 90000, p90: 200000, dedup_ratio: 0.97, new_bytes: 400000 }],
  hashes: [{ algorithm: "sha512", mb_per_sec: 800 }, { algorithm: "sha256", mb_per_sec: 500 }],
  recommended: {
    block_codec: "zstd",
    compression_level: 5,
    chunker: { type: "fastcdc", min: 25600, avg: 102400, max: 409600 },
    block_size: 102400,
    cipher: "aes256_gcm",
    kdf: { memory_kib: 65536, iterations: 17, parallelism: 4 },
    reasons: ["zstd-3 keeps 48% of the size at 310 MB/s."],
  },
  warnings: [],
};

beforeEach(() => {
  useUIStore.setState({ backupAdvanced: true });
  mockInvoke.mockReset();
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});

describe("benchmark", () => {
  it("maps the recommendation onto the CLI flags", () => {
    expect(benchRunFlags(REPORT.recommended)).toEqual([
      "--block-codec", "zstd", "--compression-level", "5", "--chunker", "fastcdc", "--block-size", "102400",
      "--cipher", "aes256-gcm", "--kdf-memory-kib", "65536", "--kdf-iterations", "17", "--kdf-parallelism", "4",
    ]);
    expect(benchRunFlags({ ...REPORT.recommended, block_codec: null, chunker: null, block_size: null, cipher: null, kdf: null, compression_level: 0 })).toEqual(["--compression-level", "0"]);
  });

  it("sends the options and shows every table", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => (cmd === "backup_bench" ? REPORT : null));
    render(<BackupBenchmark />);
    fireEvent.change(screen.getByTestId("backup-bench-size"), { target: { value: String(4 * 1024 * 1024) } });
    fireEvent.change(screen.getByTestId("backup-bench-threads"), { target: { value: "2" } });
    fireEvent.change(screen.getByTestId("backup-bench-sample"), { target: { value: "/data/sample" } });
    fireEvent.click(screen.getByTestId("backup-bench-run"));
    expect(await screen.findByTestId("backup-bench-results")).toBeInTheDocument();
    expect(mockInvoke).toHaveBeenCalledWith("backup_bench", { options: { ...DEFAULT_BENCH_OPTIONS, size: 4 * 1024 * 1024, threads: 2, sample_path: "/data/sample" } });
    expect(screen.getByTestId("backup-bench-compression")).toHaveTextContent("bzip2-9 (ZIP volumes)");
    expect(screen.getByTestId("backup-bench-kdf")).toHaveTextContent("suggested");
    expect(screen.getByTestId("backup-bench-chunkers")).toHaveTextContent("97.0%");
    expect(screen.getByTestId("backup-bench-hashes")).toHaveTextContent("sha512");
    expect(screen.getByTestId("backup-bench-flags")).toHaveTextContent("ufop backup run --block-codec zstd");
  });

  it("shows backend errors", async () => {
    mockInvoke.mockRejectedValue({ message: "Benchmark size must be between 1 byte and 4 GiB", advice: "" });
    render(<BackupBenchmark />);
    fireEvent.click(screen.getByTestId("backup-bench-run"));
    expect(await screen.findByText(/Benchmark size must be/)).toBeInTheDocument();
  });
});

describe("inspection tools", () => {
  it("runs the read-only inspect operations from maintenance", async () => {
    mockInvoke.mockImplementation(async (cmd: string, args: { operation?: { type: string } }) =>
      cmd === "backup_execute" ? { run_id: "r", status: "success", summary: `ran ${args.operation?.type}`, report: { blocks: 3 }, log: [] } : [],
    );
    render(<BackupMaintenance job={{ ...newBackupJob(), id: "j1" }} onDeleted={() => {}} />);
    fireEvent.click(screen.getByTestId("backup-op-inspect_stats"));
    expect(await screen.findByText("ran inspect_stats")).toBeInTheDocument();
    expect(mockInvoke).toHaveBeenCalledWith("backup_execute", { id: "j1", operation: { type: "inspect_stats" } });
    fireEvent.click(screen.getByTestId("backup-op-inspect_header"));
    expect(await screen.findByText("ran inspect_header")).toBeInTheDocument();
  });

  it("is hidden in Standard mode", () => {
    useUIStore.setState({ backupAdvanced: false });
    render(<BackupMaintenance job={{ ...newBackupJob(), id: "j1" }} onDeleted={() => {}} />);
    expect(screen.queryByTestId("backup-op-inspect_stats")).toBeNull();
  });
});
