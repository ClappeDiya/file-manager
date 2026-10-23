/**
 * Settings → Benchmark (K56, Advanced): measures compression, encryption,
 * Argon2id, chunkers and hashes on this computer (the engine's own code
 * paths) and suggests format settings for new backups — the desktop face of
 * `ufop backup bench`.
 */
import { useState } from "react";
import { errorText } from "@/lib/error-text";
import { formatBytes } from "@/lib/format-bytes";
import { inputCls } from "@/lib/backup-helpers";
import { benchRunFlags, DEFAULT_BENCH_OPTIONS, runBench, type BenchOptions, type BenchReport } from "@/stores/backup-bench";
import { Banner, Btn, Field, PathInput, Section } from "./backup-ui";

const MIB = 1024 * 1024;
const mbps = (v: number) => `${v.toFixed(1)} MB/s`;
const pct = (v: number) => `${(v * 100).toFixed(1)}%`;

function Table({ head, rows, testId }: { head: string[]; rows: (string | number)[][]; testId: string }) {
  return (
    <table className="w-full text-[11px]" data-testid={testId}>
      <thead>
        <tr className="text-left text-zinc-500">
          {head.map((h) => (
            <th key={h} className="font-medium pr-2">
              {h}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {rows.map((r, i) => (
          <tr key={i}>
            {r.map((c, j) => (
              <td key={j} className="pr-2 tabular-nums">
                {c}
              </td>
            ))}
          </tr>
        ))}
      </tbody>
    </table>
  );
}

export function BenchResults({ report: r }: { report: BenchReport }) {
  const flags = benchRunFlags(r.recommended);
  return (
    <div className="space-y-2" data-testid="backup-bench-results">
      <p className="text-[11px] text-zinc-500">
        Sample: {r.sample.source} ({formatBytes(r.sample.bytes)}), best of {r.repeat} run(s), {r.threads} thread(s)
      </p>
      {r.compression.length > 0 && (
        <Table
          testId="backup-bench-compression"
          head={["#", "Codec", "Size", "Compress", "Decompress"]}
          rows={r.compression.map((c) => [c.rank, c.zip_only ? `${c.name} (ZIP volumes)` : c.name, pct(c.ratio), mbps(c.compress_mb_per_sec), mbps(c.decompress_mb_per_sec)])}
        />
      )}
      {r.ciphers.length > 0 && <Table testId="backup-bench-ciphers" head={["Cipher", "Encrypt", "Decrypt"]} rows={r.ciphers.map((c) => [c.cipher, mbps(c.seal_mb_per_sec), mbps(c.open_mb_per_sec)])} />}
      {(r.kdf.length > 0 || r.kdf_suggestion) && (
        <Table
          testId="backup-bench-kdf"
          head={["Argon2id", "Memory", "Iterations", "Time"]}
          rows={[...r.kdf, ...(r.kdf_suggestion ? [r.kdf_suggestion] : [])].map((k) => [k.name, formatBytes(k.params.memory_kib * 1024), k.params.iterations, `${Math.round(k.millis)} ms`])}
        />
      )}
      {r.chunkers.length > 0 && (
        <Table
          testId="backup-bench-chunkers"
          head={["Chunker", "Speed", "Blocks", "Average", "Re-found after edits"]}
          rows={r.chunkers.map((c) => [c.name, mbps(c.mb_per_sec), c.chunks, formatBytes(Math.round(c.avg_size)), pct(c.dedup_ratio)])}
        />
      )}
      {r.hashes.length > 0 && <Table testId="backup-bench-hashes" head={["Hash", "Speed"]} rows={r.hashes.map((h) => [h.algorithm, mbps(h.mb_per_sec)])} />}
      {r.recommended.reasons.length > 0 && (
        <div className="text-xs space-y-1">
          <div className="font-medium">Suggested for new backups</div>
          <ul className="list-disc pl-4 text-zinc-600 dark:text-zinc-400">
            {r.recommended.reasons.map((why) => (
              <li key={why}>{why}</li>
            ))}
          </ul>
          <code className="block text-[11px] break-all bg-zinc-100 dark:bg-zinc-800 rounded p-1" data-testid="backup-bench-flags">
            ufop backup run {flags.join(" ")}
          </code>
        </div>
      )}
      {r.warnings.map((w) => (
        <Banner key={w} tone="warning">
          {w}
        </Banner>
      ))}
    </div>
  );
}

export function BackupBenchmark() {
  const [opts, setOpts] = useState<BenchOptions>(DEFAULT_BENCH_OPTIONS);
  const [report, setReport] = useState<BenchReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const set = (p: Partial<BenchOptions>) => setOpts({ ...opts, ...p });

  const start = async () => {
    setBusy(true);
    setError(null);
    try {
      const r = await runBench(opts);
      if (!r) setError("The benchmark needs the desktop app or ufop-server.");
      setReport(r);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Section title="Benchmark" hint="Measures compression, encryption, key derivation, chunking and hashing on this computer and suggests settings for new backups. It uses only the processor; nothing is uploaded.">
      {error && <Banner tone="error" onClose={() => setError(null)}>{error}</Banner>}
      <div className="grid grid-cols-1 sm:grid-cols-4 gap-2">
        <Field label="Sample size">
          <select data-testid="backup-bench-size" className={inputCls} value={opts.size} onChange={(e) => set({ size: Number(e.target.value) })}>
            {[4, 16, 64, 256].map((m) => (
              <option key={m} value={m * MIB}>
                {m} MiB
              </option>
            ))}
          </select>
        </Field>
        <Field label="Threads">
          <input data-testid="backup-bench-threads" type="number" min={1} max={64} className={inputCls} value={opts.threads} onChange={(e) => set({ threads: Math.max(1, Number(e.target.value) || 1) })} />
        </Field>
        <Field label="Runs">
          <input data-testid="backup-bench-repeat" type="number" min={1} max={20} className={inputCls} value={opts.repeat} onChange={(e) => set({ repeat: Math.max(1, Number(e.target.value) || 1) })} />
        </Field>
        <Field label="Key derivation target (ms)">
          <input data-testid="backup-bench-kdf-target" type="number" min={0} className={inputCls} value={opts.kdf_target_ms} onChange={(e) => set({ kdf_target_ms: Math.max(0, Number(e.target.value) || 0) })} />
        </Field>
      </div>
      <Field label="Sample data (optional)" hint="A file or folder with typical data; empty uses generated mixed data.">
        <PathInput testId="backup-bench-sample" value={opts.sample_path ?? ""} onChange={(v) => set({ sample_path: v.trim() ? v : null })} placeholder="Generated sample" />
      </Field>
      <Btn tone="primary" disabled={busy} onClick={() => void start()} testId="backup-bench-run">
        {busy ? "Measuring…" : "Run benchmark"}
      </Btn>
      {report && <BenchResults report={report} />}
    </Section>
  );
}
