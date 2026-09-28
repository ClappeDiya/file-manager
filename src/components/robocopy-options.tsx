/**
 * Robocopy-equivalent copy options for sync pairs.
 *
 * Everything Windows Robocopy does — /MIR, /XD, /XO, /MT, /Z, /R /W, /RH,
 * /LOG … — in plain language. Options stay tucked into collapsed groups so
 * the create form is no busier than before; a one-click preset or a pasted
 * Robocopy command fills them in. Each option shows its Robocopy switch as a
 * small hint for people who already know the tool.
 */
import { useEffect, useState, type ReactNode } from "react";
import { tauriInvoke, tauriInvokeSafe } from "@/hooks/use-tauri";

import {
  COPY_PRESETS,
  type CopyOptions,
  type CopyPreset,
  type RobocopyJob,
  type SymlinkMode,
} from "@/lib/copy-options";

const splitList = (s: string) =>
  s
    .split(",")
    .map((x) => x.trim())
    .filter(Boolean);

// ── Small building blocks ──

const inputCls =
  "px-2 py-1 text-xs border rounded dark:bg-zinc-800 dark:border-zinc-600";

function Switch({ flag }: { flag: string }) {
  return (
    <span className="ml-1 font-mono text-[10px] text-zinc-400 dark:text-zinc-500">{flag}</span>
  );
}

function Check({
  label,
  flag,
  checked,
  onChange,
}: {
  label: string;
  flag: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <label className="flex items-center gap-2 text-xs text-zinc-600 dark:text-zinc-400">
      <input type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} />
      <span>
        {label}
        <Switch flag={flag} />
      </span>
    </label>
  );
}

function NumberField({
  label,
  flag,
  value,
  unit,
  min = 0,
  max,
  onChange,
}: {
  label: string;
  flag: string;
  value: number;
  unit?: string;
  min?: number;
  max?: number;
  onChange: (v: number) => void;
}) {
  return (
    <label className="flex items-center justify-between gap-2 text-xs text-zinc-600 dark:text-zinc-400">
      <span>
        {label}
        <Switch flag={flag} />
      </span>
      <span className="flex items-center gap-1">
        <input
          type="number"
          min={min}
          max={max}
          value={value}
          onChange={(e) => {
            const n = parseInt(e.target.value, 10);
            onChange(Number.isNaN(n) ? min : Math.max(min, max ? Math.min(max, n) : n));
          }}
          className={`${inputCls} w-16 text-right`}
        />
        {unit && <span className="w-10 text-[11px] text-zinc-400">{unit}</span>}
      </span>
    </label>
  );
}

function Group({ title, children }: { title: string; children: ReactNode }) {
  return (
    <details className="group rounded border border-zinc-200 dark:border-zinc-700">
      <summary className="cursor-pointer select-none px-2 py-1.5 text-xs font-medium text-zinc-700 dark:text-zinc-300">
        {title}
      </summary>
      <div className="space-y-2 px-2 pb-2">{children}</div>
    </details>
  );
}

// ── Presets row ──

export function CopyPresetPicker({
  activeId,
  onPick,
}: {
  activeId: string | null;
  onPick: (preset: CopyPreset) => void;
}) {
  return (
    <div className="space-y-1">
      <div className="text-xs font-medium text-zinc-600 dark:text-zinc-400">Start from</div>
      <div className="flex flex-wrap gap-1.5" role="group" aria-label="Copy presets">
        {COPY_PRESETS.map((p) => (
          <button
            key={p.id}
            type="button"
            title={p.hint}
            aria-pressed={activeId === p.id}
            onClick={() => onPick(p)}
            className={`text-xs px-2.5 py-1 rounded-full border transition-colors ${
              activeId === p.id
                ? "bg-blue-500 text-white border-blue-500"
                : "border-zinc-300 dark:border-zinc-600 text-zinc-600 dark:text-zinc-400 hover:bg-zinc-100 dark:hover:bg-zinc-800"
            }`}
          >
            {p.label}
          </button>
        ))}
      </div>
      {activeId && (
        <p className="text-[11px] text-zinc-500">
          {COPY_PRESETS.find((p) => p.id === activeId)?.hint}
        </p>
      )}
    </div>
  );
}

// ── Import a Robocopy command ──

export function RobocopyImport({ onImport }: { onImport: (job: RobocopyJob) => void }) {
  const [open, setOpen] = useState(false);
  const [command, setCommand] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [notes, setNotes] = useState<string[]>([]);

  const handleImport = async () => {
    setError(null);
    setNotes([]);
    try {
      const job = await tauriInvoke<RobocopyJob>("parse_robocopy_command", { command });
      onImport(job);
      setNotes(job.unsupported);
      if (job.unsupported.length === 0) setOpen(false);
    } catch (e: unknown) {
      setError((e as Error)?.message || String(e) || "Could not read that command");
    }
  };

  if (!open) {
    return (
      <button
        type="button"
        onClick={() => setOpen(true)}
        className="text-xs text-blue-600 dark:text-blue-400 hover:underline"
      >
        Have a Robocopy command? Paste it here
      </button>
    );
  }

  return (
    <div className="space-y-1.5 rounded border border-zinc-200 dark:border-zinc-700 p-2">
      <label className="block text-xs text-zinc-600 dark:text-zinc-400">
        Robocopy command
        <textarea
          value={command}
          onChange={(e) => setCommand(e.target.value)}
          rows={2}
          className={`mt-1 block w-full font-mono ${inputCls}`}
          placeholder="robocopy C:\Data D:\Backup /MIR /R:3 /W:5 /MT:8"
        />
      </label>
      <div className="flex gap-2">
        <button
          type="button"
          onClick={handleImport}
          disabled={!command.trim()}
          className="px-2 py-1 text-xs rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-50"
        >
          Import
        </button>
        <button
          type="button"
          onClick={() => setOpen(false)}
          className="px-2 py-1 text-xs rounded border border-zinc-300 dark:border-zinc-600"
        >
          Close
        </button>
      </div>
      {error && <p className="text-[11px] text-red-600 dark:text-red-400">{error}</p>}
      {notes.length > 0 && (
        <ul className="text-[11px] text-amber-700 dark:text-amber-400 list-disc pl-4">
          {notes.map((n, i) => (
            <li key={i}>{n}</li>
          ))}
        </ul>
      )}
    </div>
  );
}

// ── All options, grouped and collapsed ──

export function RobocopyOptionsSection({
  options,
  onChange,
  mode,
}: {
  options: CopyOptions;
  onChange: (o: CopyOptions) => void;
  mode: string;
}) {
  const set = <K extends keyof CopyOptions>(k: K, v: CopyOptions[K]) =>
    onChange({ ...options, [k]: v });
  const [excludeDirsText, setExcludeDirsText] = useState(options.exclude_dirs.join(", "));
  useEffect(() => setExcludeDirsText(options.exclude_dirs.join(", ")), [options.exclude_dirs]);
  const runHoursInvalid =
    options.run_hours.trim() !== "" && !/^\s*\d{2}:?\d{2}\s*-\s*\d{2}:?\d{2}\s*$/.test(options.run_hours);

  return (
    <div className="space-y-1.5">
      <Group title="What to copy">
        <Check label="Include subfolders" flag="/S" checked={options.copy_subdirs} onChange={(v) => set("copy_subdirs", v)} />
        <Check label="Also create empty folders" flag="/E" checked={options.include_empty_dirs} onChange={(v) => set("include_empty_dirs", v)} />
        {options.copy_subdirs && (
          <NumberField label="Folder depth limit (0 = all)" flag="/LEV" value={options.max_depth} onChange={(v) => set("max_depth", v)} />
        )}
        <label className="block text-xs text-zinc-600 dark:text-zinc-400">
          Skip folders (names or paths, comma-separated)<Switch flag="/XD" />
          <input
            type="text"
            value={excludeDirsText}
            onChange={(e) => setExcludeDirsText(e.target.value)}
            onBlur={() => set("exclude_dirs", splitList(excludeDirsText))}
            className={`mt-1 block w-full ${inputCls}`}
            placeholder="node_modules, .git, cache"
          />
        </label>
        <NumberField label="Skip files older than" flag="/MAXAGE" value={options.max_age_days} unit="days" onChange={(v) => set("max_age_days", v)} />
        <NumberField label="Skip files newer than" flag="/MINAGE" value={options.min_age_days} unit="days" onChange={(v) => set("min_age_days", v)} />
        <Check label="Skip hidden files" flag="/XA:H" checked={options.exclude_hidden} onChange={(v) => set("exclude_hidden", v)} />
        <Check label="Skip read-only files" flag="/XA:R" checked={options.exclude_readonly} onChange={(v) => set("exclude_readonly", v)} />
        <label className="flex items-center justify-between gap-2 text-xs text-zinc-600 dark:text-zinc-400">
          <span>
            Shortcuts / symbolic links<Switch flag="/XJ /SL" />
          </span>
          <select
            value={options.symlinks}
            onChange={(e) => set("symlinks", e.target.value as SymlinkMode)}
            className={inputCls}
          >
            <option value="follow">Copy what they point to</option>
            <option value="skip">Skip them</option>
            <option value="copy_link">Copy the link itself</option>
          </select>
        </label>
      </Group>

      <Group title="When to overwrite">
        <Check label="Never replace a newer destination file" flag="/XO" checked={options.exclude_older} onChange={(v) => set("exclude_older", v)} />
        <Check label="Never replace an older destination file" flag="/XN" checked={options.exclude_newer} onChange={(v) => set("exclude_newer", v)} />
        <Check label="Skip files whose size changed but date didn't" flag="/XC" checked={options.exclude_changed} onChange={(v) => set("exclude_changed", v)} />
        <Check label="Only update files already at the destination" flag="/XL" checked={options.exclude_lonely} onChange={(v) => set("exclude_lonely", v)} />
        <Check label="Re-copy identical files too" flag="/IS" checked={options.include_same} onChange={(v) => set("include_same", v)} />
        <Check label="Allow 2-second time differences (FAT, NAS)" flag="/FFT" checked={options.fat_time_tolerance} onChange={(v) => set("fat_time_tolerance", v)} />
        <Check label="Ignore 1-hour daylight-saving shifts" flag="/DST" checked={options.dst_tolerance} onChange={(v) => set("dst_tolerance", v)} />
      </Group>

      <Group title="Cleanup & moving">
        {mode === "mirror" ? (
          <Check label="Keep extra files at the destination (don't delete)" flag="/XX" checked={options.exclude_extra} onChange={(v) => set("exclude_extra", v)} />
        ) : mode !== "two_way" ? (
          <Check label="Delete destination files the source no longer has" flag="/PURGE" checked={options.purge} onChange={(v) => set("purge", v)} />
        ) : null}
        <Check label="Move files (delete from source after copying)" flag="/MOV" checked={options.move_files} onChange={(v) => onChange({ ...options, move_files: v, move_dirs: v && options.move_dirs })} />
        {options.move_files && (
          <Check label="…and remove emptied source folders" flag="/MOVE" checked={options.move_dirs} onChange={(v) => set("move_dirs", v)} />
        )}
        <Check label="Keep file dates" flag="/COPY:T" checked={options.copy_timestamps} onChange={(v) => set("copy_timestamps", v)} />
        <Check label="Keep folder dates" flag="/DCOPY:T" checked={options.copy_dir_timestamps} onChange={(v) => set("copy_dir_timestamps", v)} />
        <Check label="Create folder structure and empty files only" flag="/CREATE" checked={options.create_only} onChange={(v) => set("create_only", v)} />
      </Group>

      <Group title="Reliability & speed">
        <Check label="Resume interrupted copies" flag="/Z" checked={options.restartable} onChange={(v) => set("restartable", v)} />
        <NumberField label="Retries per file" flag="/R" value={options.retries} onChange={(v) => set("retries", v)} />
        {options.retries > 0 && (
          <NumberField label="Wait between retries" flag="/W" value={options.retry_wait_secs} unit="sec" onChange={(v) => set("retry_wait_secs", v)} />
        )}
        <NumberField label="Parallel copies" flag="/MT" value={options.threads} min={1} max={128} unit="threads" onChange={(v) => set("threads", v)} />
        <NumberField label="Slow down to save bandwidth" flag="/IPG" value={options.inter_packet_gap_ms} unit="ms" onChange={(v) => set("inter_packet_gap_ms", v)} />
      </Group>

      <Group title="Time window & log">
        <label className="block text-xs text-zinc-600 dark:text-zinc-400">
          Only copy between (HHMM-HHMM, empty = any time)<Switch flag="/RH" />
          <input
            type="text"
            value={options.run_hours}
            onChange={(e) => set("run_hours", e.target.value)}
            aria-invalid={runHoursInvalid}
            className={`mt-1 block w-full ${inputCls} ${runHoursInvalid ? "border-red-400" : ""}`}
            placeholder="2200-0600"
          />
        </label>
        <label className="block text-xs text-zinc-600 dark:text-zinc-400">
          Write a log file<Switch flag="/LOG" />
          <input
            type="text"
            value={options.log_file}
            onChange={(e) => set("log_file", e.target.value)}
            className={`mt-1 block w-full ${inputCls}`}
            placeholder="/path/to/sync.log"
          />
        </label>
        {options.log_file.trim() && (
          <Check label="Append instead of overwrite" flag="/LOG+" checked={options.log_append} onChange={(v) => set("log_append", v)} />
        )}
      </Group>
    </div>
  );
}

// ── Equivalent command readout ──

export function RobocopyCommandPreview({
  sourcePath,
  destPath,
  mode,
  filter,
  options,
}: {
  sourcePath: string;
  destPath: string;
  mode: string;
  filter: unknown;
  options: CopyOptions;
}) {
  const [command, setCommand] = useState("");
  const filterJson = JSON.stringify(filter);
  const optionsJson = JSON.stringify(options);

  useEffect(() => {
    const t = setTimeout(async () => {
      const cmd = await tauriInvokeSafe<string>(
        "robocopy_command_preview",
        { sourcePath, destPath, mode, filterJson, optionsJson },
        "",
      );
      setCommand(cmd);
    }, 250);
    return () => clearTimeout(t);
  }, [sourcePath, destPath, mode, filterJson, optionsJson]);

  if (!command) return null;
  return (
    <div className="space-y-1">
      <div className="flex items-center justify-between text-[11px] text-zinc-500">
        <span>Equivalent Robocopy command</span>
        <button
          type="button"
          onClick={() => navigator.clipboard?.writeText(command).catch(() => {})}
          className="text-blue-600 dark:text-blue-400 hover:underline"
        >
          Copy
        </button>
      </div>
      <code className="block break-all rounded bg-zinc-100 dark:bg-zinc-800 px-2 py-1 text-[10px] text-zinc-700 dark:text-zinc-300">
        {command}
      </code>
    </div>
  );
}
