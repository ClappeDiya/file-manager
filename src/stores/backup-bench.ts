/**
 * Codec / cipher / Argon2id / chunker / hash benchmark (K56): wire types of
 * `ufop_core::backup::bench` and the `backup_bench` command. The same report
 * comes from `ufop backup bench --json`.
 */
import { isBackendAvailable, tauriInvoke } from "@/hooks/use-tauri";
import type { ChunkerSpec, CipherKind, KdfParams } from "./backup-store";

export type BenchSection = "compression" | "crypto" | "kdf" | "chunkers" | "hashes";

export interface BenchOptions {
  size: number;
  seed: number;
  sample_path: string | null;
  repeat: number;
  threads: number;
  block_size: number;
  /** Empty = every section. */
  sections: BenchSection[];
  kdf_target_ms: number;
  kdf_memory_kib: number;
  kdf_parallelism: number;
  kdf_presets: string[];
}

export const DEFAULT_BENCH_OPTIONS: BenchOptions = {
  size: 16 * 1024 * 1024,
  seed: 1,
  sample_path: null,
  repeat: 3,
  threads: 1,
  block_size: 100 * 1024,
  sections: [],
  kdf_target_ms: 1000,
  kdf_memory_kib: 65536,
  kdf_parallelism: 4,
  kdf_presets: [],
};

export interface CompressionResult {
  name: string;
  codec: string;
  level: number;
  native_level: number;
  zip_only: boolean;
  compressed_bytes: number;
  ratio: number;
  compress_mb_per_sec: number;
  decompress_mb_per_sec: number;
  rank: number;
}

export interface CipherResult {
  cipher: CipherKind;
  seal_mb_per_sec: number;
  open_mb_per_sec: number;
}

export interface KdfResult {
  name: string;
  params: KdfParams;
  millis: number;
}

export interface ChunkerResult {
  name: string;
  spec: ChunkerSpec;
  mb_per_sec: number;
  chunks: number;
  min_size: number;
  max_size: number;
  avg_size: number;
  stddev: number;
  p10: number;
  p50: number;
  p90: number;
  dedup_ratio: number;
  new_bytes: number;
}

export interface HashResult {
  algorithm: "sha256" | "sha512";
  mb_per_sec: number;
}

export interface BenchRecommendation {
  block_codec: "deflate" | "zstd" | null;
  compression_level: number | null;
  chunker: ChunkerSpec | null;
  block_size: number | null;
  cipher: CipherKind | null;
  kdf: KdfParams | null;
  reasons: string[];
}

export interface BenchReport {
  sample: { source: string; bytes: number; files: number; seed: number | null };
  repeat: number;
  threads: number;
  block_size: number;
  compression: CompressionResult[];
  ciphers: CipherResult[];
  kdf: KdfResult[];
  kdf_suggestion: KdfResult | null;
  kdf_target_ms: number;
  chunkers: ChunkerResult[];
  hashes: HashResult[];
  recommended: BenchRecommendation;
  warnings: string[];
}

/** Run the benchmark (null without a backend). */
export async function runBench(options: BenchOptions): Promise<BenchReport | null> {
  if (!isBackendAvailable()) return null;
  return tauriInvoke<BenchReport>("backup_bench", { options });
}

/** The `ufop backup run` flags the CLI prints with `--print-options`. */
export function benchRunFlags(r: BenchRecommendation): string[] {
  const f: string[] = [];
  if (r.block_codec) f.push("--block-codec", r.block_codec);
  if (r.compression_level !== null) f.push("--compression-level", String(r.compression_level));
  if (r.chunker) f.push("--chunker", r.chunker.type === "fastcdc" ? "fastcdc" : "fixed");
  if (r.block_size !== null) f.push("--block-size", String(r.block_size));
  if (r.cipher) f.push("--cipher", r.cipher.replace(/_/g, "-"));
  if (r.kdf) f.push("--kdf-memory-kib", String(r.kdf.memory_kib), "--kdf-iterations", String(r.kdf.iterations), "--kdf-parallelism", String(r.kdf.parallelism));
  return f;
}
