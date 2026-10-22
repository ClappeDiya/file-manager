/**
 * Support tools for a job: diagnostics (database consistency with the
 * destination, DNS/TCP/TLS timings, upload/download throughput) and a
 * sanitized bug-report zip (paths hashed, no secrets).
 */
import { useState } from "react";
import { tauriInvoke } from "@/hooks/use-tauri";
import { errorText, type BackupJob, type BugReport, type Diagnostics } from "@/stores/backup-store";
import { formatBytes } from "@/lib/format-bytes";
import { inputCls } from "@/lib/backup-helpers";
import { Banner, Btn, Check, Field, PathInput, Section } from "./backup-ui";

function joinPath(folder: string, name: string): string {
  const sep = folder.includes("\\") && !folder.includes("/") ? "\\" : "/";
  return folder.replace(/[\\/]+$/, "") + sep + name;
}

function ms(v: number | null): string {
  return v == null ? "—" : `${v.toFixed(1)} ms`;
}

function List({ title, items, tone }: { title: string; items: string[]; tone: "error" | "warning" | "neutral" }) {
  if (items.length === 0) return null;
  const cls = tone === "error" ? "text-red-600" : tone === "warning" ? "text-amber-600" : "text-zinc-500";
  return (
    <div className="text-xs">
      <div className={`font-medium ${cls}`}>
        {title} ({items.length})
      </div>
      <ul className="font-mono text-[11px] max-h-28 overflow-auto">
        {items.slice(0, 200).map((i) => (
          <li key={i} className="truncate">
            {i}
          </li>
        ))}
      </ul>
    </div>
  );
}

export function DiagnosticsResult({ d }: { d: Diagnostics }) {
  const db = d.database;
  const dbOk = db && db.problems.length === 0 && db.missing_remote.length === 0 && db.size_mismatch.length === 0;
  return (
    <div className="space-y-2" data-testid="diagnostics-result">
      <div className="text-xs">
        <span className="font-medium">Database: </span>
        {d.database_error ? (
          <span className="text-amber-600">{d.database_error}</span>
        ) : db ? (
          <span className={dbOk ? "text-green-600" : "text-red-600"}>
            {dbOk ? "consistent" : "problems found"} · {db.tracked_volumes} tracked volumes, {db.remote_files} remote files · {db.check_ms.toFixed(0)} ms
          </span>
        ) : null}
      </div>
      {db && (
        <>
          <List title="Integrity problems" items={db.problems} tone="error" />
          <List title="Missing at the destination" items={db.missing_remote} tone="error" />
          <List title="Size mismatch" items={db.size_mismatch} tone="error" />
          <List title="Not tracked by the database" items={db.untracked_remote} tone="warning" />
        </>
      )}
      {d.network && (
        <div className="text-xs space-y-0.5">
          <div>
            <span className="font-medium">Network: </span>
            {d.network.host}:{d.network.port} {d.network.addresses.length > 0 && `(${d.network.addresses.join(", ")})`}
          </div>
          <div className="grid grid-cols-3 gap-2 font-mono text-[11px]">
            <span>DNS {ms(d.network.dns_ms)}</span>
            <span>TCP {ms(d.network.tcp_ms)}</span>
            {d.network.tls && <span>TLS {ms(d.network.tls_ms)} {d.network.tls_version ?? ""}</span>}
          </div>
          {d.network.error && <div className="text-red-600">{d.network.error}</div>}
        </div>
      )}
      {d.throughput && (
        <div className="text-xs space-y-0.5">
          <div>
            <span className="font-medium">Throughput: </span>
            {formatBytes(d.throughput.test_bytes)} test file {d.throughput.verified ? "verified" : "not verified"}
          </div>
          <div className="grid grid-cols-3 gap-2 font-mono text-[11px]">
            <span>List {d.throughput.list_ms.map((v) => v.toFixed(0)).join(" / ")} ms</span>
            <span>Up {formatBytes(d.throughput.upload_bps)}/s</span>
            <span>Down {formatBytes(d.throughput.download_bps)}/s</span>
          </div>
          {d.throughput.error && <div className="text-red-600">{d.throughput.error}</div>}
        </div>
      )}
    </div>
  );
}

export function BackupSupportTools({ job }: { job: BackupJob }) {
  const [network, setNetwork] = useState(true);
  const [sizeKb, setSizeKb] = useState(1024);
  const [diag, setDiag] = useState<Diagnostics | null>(null);
  const [folder, setFolder] = useState("");
  const [report, setReport] = useState<BugReport | null>(null);
  const [busy, setBusy] = useState<"diag" | "report" | null>(null);
  const [error, setError] = useState<string | null>(null);

  const run = async <T,>(kind: "diag" | "report", f: () => Promise<T>, set: (v: T) => void) => {
    setBusy(kind);
    setError(null);
    try {
      set(await f());
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(null);
    }
  };

  const reportName = `ufop-bug-report-${job.name.replace(/[^\w.-]+/g, "_") || "job"}-${new Date().toISOString().slice(0, 10)}.zip`;

  return (
    <>
      {error && (
        <Banner tone="error" onClose={() => setError(null)}>
          {error}
        </Banner>
      )}
      <Section title="Diagnostics" hint="Checks the local database against the destination, times DNS, TCP and TLS to the service, and measures upload/download speed with a temporary test file.">
        <div className="flex gap-3 items-end flex-wrap">
          <Check label="Network timings" checked={network} onChange={setNetwork} />
          <Field label="Throughput test (KB, 0 = skip)">
            <input className={inputCls} type="number" min={0} max={65536} value={sizeKb} onChange={(e) => setSizeKb(Math.min(65536, Math.max(0, Number(e.target.value) || 0)))} />
          </Field>
          <Btn
            testId="backup-diagnose"
            disabled={busy !== null}
            onClick={() => run("diag", () => tauriInvoke<Diagnostics>("backup_diagnose", { jobId: job.id, network, throughputKb: sizeKb }), setDiag)}
          >
            {busy === "diag" ? "Running…" : "Run diagnostics"}
          </Btn>
        </div>
        {diag && <DiagnosticsResult d={diag} />}
      </Section>
      <Section title="Bug report" hint="A zip with a copy of the job database (file and folder names replaced by hashes), the configuration without passwords, tokens, addresses or paths, recent run logs and system information.">
        <Field label="Save to folder">
          <PathInput value={folder} onChange={setFolder} placeholder="/Users/me/Desktop" testId="bug-report-folder" />
        </Field>
        <Btn
          testId="backup-bug-report"
          disabled={busy !== null || !folder.trim()}
          onClick={() => run("report", () => tauriInvoke<BugReport>("backup_bug_report", { jobId: job.id, outPath: joinPath(folder.trim(), reportName) }), setReport)}
        >
          {busy === "report" ? "Creating…" : "Create bug report"}
        </Btn>
        {report && (
          <Banner tone="success" onClose={() => setReport(null)}>
            Saved {report.path} ({formatBytes(report.size)}): {report.files.join(", ")} · {report.paths_obfuscated} paths hashed, {report.runs_included} runs.
          </Banner>
        )}
      </Section>
    </>
  );
}
