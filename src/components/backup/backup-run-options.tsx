/**
 * Source and run settings added for Kopia parity: stay on one filesystem,
 * the read-error policy (per kind, fail-fast, error budget), percentage
 * rehash, the per-run upload limit, parallel hashing of large files and a
 * command whose output is backed up (`stdin_command`). Each field maps 1:1
 * onto `BackupOptions` / `BackupJob` in Rust.
 */
import { inputCls } from "@/lib/backup-helpers";
import { DEFAULT_PARALLEL_FILE_THRESHOLD, type BackupOptions, type ErrorAction, type ErrorPolicy, type StdinCommand, type UnknownEntryAction } from "@/stores/backup-store";
import { Check, Field, Section } from "./backup-ui";

const MiB = 1024 * 1024;
const GiB = 1024 * MiB;

/** Whole, non-negative number from an input; "" → null. */
const optNum = (v: string) => (v.trim() === "" ? null : Math.max(0, Math.floor(Number(v) || 0)));

const DEFAULT_ERROR_POLICY: ErrorPolicy = { on_file_error: "continue", on_dir_error: "continue", on_unknown_entry: "ignore", fail_fast: false, max_errors: null };

export function SourcesAndRunsOptions({ options: o, onChange }: { options: BackupOptions; onChange: (p: Partial<BackupOptions>) => void }) {
  // Settings saved by older versions may lack these fields.
  const ep = o.error_policy ?? DEFAULT_ERROR_POLICY;
  const threshold = o.parallel_file_threshold ?? DEFAULT_PARALLEL_FILE_THRESHOLD;
  const cap = o.max_upload_bytes_per_run ?? null;
  const setPolicy = (p: Partial<ErrorPolicy>) => onChange({ error_policy: { ...ep, ...p } });
  const capGb = cap === null ? "" : String(Math.round((cap / GiB) * 100) / 100);
  return (
    <>
      <Section title="Sources and runs" hint="How the backup walks the sources and when a run stops early. A stopped run keeps a partial version of what it finished; the next run continues.">
        <Check
          label="Stay on one filesystem"
          checked={!!o.one_file_system}
          onChange={(v) => onChange({ one_file_system: v })}
          hint="Folders on another drive or mount (network shares, /proc, USB disks mounted inside a source) are kept as empty folders. The run log lists them."
        />
        <div className="grid grid-cols-1 sm:grid-cols-3 gap-2">
          <Field label="Unreadable file">
            <select data-testid="opt-on-file-error" className={inputCls} value={ep.on_file_error} onChange={(e) => setPolicy({ on_file_error: e.target.value as ErrorAction })}>
              <option value="continue">Warn and keep going</option>
              <option value="fail">Count as an error</option>
            </select>
          </Field>
          <Field label="Unreadable folder">
            <select data-testid="opt-on-dir-error" className={inputCls} value={ep.on_dir_error} onChange={(e) => setPolicy({ on_dir_error: e.target.value as ErrorAction })}>
              <option value="continue">Warn and keep going</option>
              <option value="fail">Count as an error</option>
            </select>
          </Field>
          <Field label="Sockets, pipes, devices">
            <select data-testid="opt-on-unknown" className={inputCls} value={ep.on_unknown_entry} onChange={(e) => setPolicy({ on_unknown_entry: e.target.value as UnknownEntryAction })}>
              <option value="ignore">Skip silently</option>
              <option value="warn">Skip with a warning</option>
              <option value="fail">Count as an error</option>
            </select>
          </Field>
        </div>
        <Check
          label="Stop at the first error (fail fast)"
          checked={ep.fail_fast}
          onChange={(v) => setPolicy({ fail_fast: v })}
          hint="Without it, errors are reported at the end and the version is complete."
        />
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
          <Field label="Stop after more than N read problems (empty = never)">
            <input data-testid="opt-max-errors" className={inputCls} type="number" min={0} value={ep.max_errors ?? ""} onChange={(e) => setPolicy({ max_errors: optNum(e.target.value) })} />
          </Field>
          <Field label="Re-check unchanged files (% per run, 0–100)" hint="Re-reads a rotating sample of files whose size and time did not change, to catch silent corruption.">
            <input data-testid="opt-rehash" className={inputCls} type="number" min={0} max={100} value={o.rehash_percent ?? 0} onChange={(e) => onChange({ rehash_percent: Math.min(100, optNum(e.target.value) ?? 0) })} />
          </Field>
          <Field label="Upload at most per run (GB, empty = no limit)" hint="Spreads a large first backup over several runs.">
            <input
              data-testid="opt-max-upload"
              className={inputCls}
              type="number"
              min={0}
              step="0.1"
              value={capGb}
              onChange={(e) => {
                const gb = Number(e.target.value);
                onChange({ max_upload_bytes_per_run: e.target.value.trim() === "" || !(gb > 0) ? null : Math.round(gb * GiB) });
              }}
            />
          </Field>
          <Field label="Hash files larger than this in parallel (MB, 0 = off)" hint="Large files are split over several threads; the stored blocks are identical.">
            <input
              data-testid="opt-parallel-threshold"
              className={inputCls}
              type="number"
              min={0}
              value={Math.round(threshold / MiB)}
              onChange={(e) => onChange({ parallel_file_threshold: (optNum(e.target.value) ?? DEFAULT_PARALLEL_FILE_THRESHOLD / MiB) * MiB })}
            />
          </Field>
        </div>
      </Section>
    </>
  );
}

/** A command whose standard output each backup stores as one file. */
export function StdinCommandEditor({ value, onChange }: { value: StdinCommand | null; onChange: (v: StdinCommand | null) => void }) {
  const v = value ?? { command: "", path: "", timeout_secs: 0 };
  const set = (p: Partial<StdinCommand>) => {
    const next = { ...v, ...p };
    onChange(next.command.trim() === "" && next.path.trim() === "" && next.timeout_secs === 0 ? null : next);
  };
  const shown = v.path.trim() === "" ? "/stdin/command-output" : v.path.startsWith("/") ? v.path : `/stdin/${v.path}`;
  return (
    <Section
      title="Back up a command's output"
      hint="Runs the command at every backup and stores what it prints as one file, e.g. a database dump. It gets the same UFOP_* variables as scripts; a non-zero exit fails the run. Restore it like any file."
    >
      <div className="grid grid-cols-1 sm:grid-cols-3 gap-2">
        <Field label="Command">
          <input data-testid="stdin-command" className={`${inputCls} font-mono`} placeholder="pg_dump mydb" value={v.command} onChange={(e) => set({ command: e.target.value })} />
        </Field>
        <Field label="File name in the backup" hint={`Stored as ${shown}`}>
          <input data-testid="stdin-path" className={`${inputCls} font-mono`} placeholder="mydb.sql" value={v.path} onChange={(e) => set({ path: e.target.value })} />
        </Field>
        <Field label="Time limit (seconds, 0 = none)">
          <input data-testid="stdin-timeout" className={inputCls} type="number" min={0} value={v.timeout_secs} onChange={(e) => set({ timeout_secs: optNum(e.target.value) ?? 0 })} />
        </Field>
      </div>
    </Section>
  );
}
