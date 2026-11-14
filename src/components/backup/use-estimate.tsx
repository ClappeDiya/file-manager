import { useState } from "react";
import { errorText, useBackupStore, type BackupEstimate } from "@/stores/backup-store";
import { EstimatePanel } from "./backup-estimate";
import { Btn } from "./backup-ui";

/** "Estimate" button and its result panel, for a job card. */
export function useEstimate(jobId: string) {
  const estimate = useBackupStore((s) => s.estimate);
  const [result, setResult] = useState<BackupEstimate | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const run = async () => {
    setBusy(true);
    setErr(null);
    try {
      setResult(await estimate(jobId));
    } catch (e) {
      setErr(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  const button = (
    <Btn disabled={busy} onClick={() => void run()} testId="backup-estimate-btn" title="How much the next backup reads and uploads, and how long it may take">
      {busy ? "Estimating…" : "Estimate"}
    </Btn>
  );
  const panel = (
    <>
      {err && <div className="text-xs text-red-600 mt-1">{err}</div>}
      {result && <EstimatePanel estimate={result} onClose={() => setResult(null)} />}
    </>
  );
  return { button, panel };
}
