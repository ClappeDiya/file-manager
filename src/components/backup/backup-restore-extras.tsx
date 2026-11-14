/**
 * Restore options added for Kopia parity (WP5): "restore as it was at …"
 * (K41), mirror restore with its deletion list (K45), and the Advanced
 * owner / error policy / sparse / cache settings (K46, K47, K48, K77).
 */
import { useState } from "react";
import { inputCls, formatDate } from "@/lib/backup-helpers";
import { localInputToIso, pickVersionAsOf, toLocalInput } from "@/lib/backup-time";
import { deletionSummary } from "@/stores/backup-access";
import type { ErrorAction, Fileset, OwnerPolicy, RestoreOptions, SparseMode, UnknownEntryAction } from "@/stores/backup-store";
import { AdvancedOnly } from "./advanced-only";
import { Banner, Btn, Check, Field } from "./backup-ui";

/** "Restore as it was at …": picks the matching version and says which one. */
export function RestoreAsOfPicker({ versions, onPick }: { versions: Fileset[]; onPick: (index: number | null, iso: string | null) => void }) {
  const [value, setValue] = useState("");
  const [partial, setPartial] = useState(false);
  const iso = localInputToIso(value);
  const index = iso ? pickVersionAsOf(versions, iso, partial) : null;
  const update = (v: string, p: boolean) => {
    setValue(v);
    setPartial(p);
    const t = localInputToIso(v);
    onPick(t ? pickVersionAsOf(versions, t, p) : null, t);
  };
  return (
    <div className="space-y-1 text-xs" data-testid="restore-as-of">
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-medium">Restore as it was at</span>
        <input
          type="datetime-local"
          className={`${inputCls} w-auto`}
          value={value}
          max={toLocalInput(new Date())}
          onChange={(e) => update(e.target.value, partial)}
          aria-label="Restore as it was at"
          data-testid="restore-as-of-input"
        />
        {value && (
          <Btn onClick={() => update("", partial)} title="Pick a version from the list instead">
            Clear
          </Btn>
        )}
      </div>
      <AdvancedOnly>
        <Check label="Allow interrupted (partial) versions" checked={partial} onChange={(p) => update(value, p)} />
      </AdvancedOnly>
      {iso && (
        <p className={index === null ? "text-amber-600" : "text-zinc-600 dark:text-zinc-400"} data-testid="restore-as-of-result">
          {index === null
            ? "No complete version was made at or before that time."
            : `Uses the version from ${formatDate(versions[index].timestamp)}${index === 0 ? " (the latest)" : ""}.`}
        </p>
      )}
    </div>
  );
}

const ERROR_ACTIONS: [ErrorAction, string][] = [
  ["fail", "Error (continue with the rest)"],
  ["continue", "Warning only"],
];
const METADATA_ACTIONS: [UnknownEntryAction, string][] = [
  ["warn", "Warning"],
  ["ignore", "Ignore"],
  ["fail", "Error"],
];

/** Mirror (Standard) and owner / error / sparse / cache options (Advanced). */
export function RestoreSafetyOptions({ value, onChange }: { value: RestoreOptions; onChange: (v: RestoreOptions) => void }) {
  const ep = value.error_policy;
  const setEp = (patch: Partial<RestoreOptions["error_policy"]>) => onChange({ ...value, error_policy: { ...ep, ...patch } });
  const select = <T extends string>(v: T, opts: [T, string][], set: (v: T) => void, label: string) => (
    <select className={inputCls} value={v} aria-label={label} onChange={(e) => set(e.target.value as T)}>
      {opts.map(([k, l]) => (
        <option key={k} value={k}>
          {l}
        </option>
      ))}
    </select>
  );
  return (
    <div className="space-y-2">
      {value.target !== null && (
        <Check
          label="Mirror: delete files in the target folder that are not in this version"
          checked={value.delete_extraneous}
          onChange={(v) => onChange({ ...value, delete_extraneous: v })}
          hint="Only inside the folders this restore recreates. You see the list and confirm before anything is deleted."
        />
      )}
      <AdvancedOnly>
        <details className="text-xs" data-testid="restore-advanced-options">
          <summary className="cursor-pointer select-none">Owners, errors and sparse files</summary>
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-2 mt-2">
            <Field label="Restore file owners" hint="Needs administrator / root rights; Auto skips owners without them.">
              {select<OwnerPolicy>(value.restore_owner, [["auto", "Auto (when running with privileges)"], ["always", "Always (fail without privileges)"], ["never", "Never"]], (v) => onChange({ ...value, restore_owner: v }), "Restore file owners")}
            </Field>
            <Field label="Sparse files" hint="Leave holes for runs of zeros instead of writing them.">
              {select<SparseMode>(value.sparse, [["auto", "Auto"], ["off", "Off (write every byte)"]], (v) => onChange({ ...value, sparse: v }), "Sparse files")}
            </Field>
            <Field label="A file cannot be written">{select(ep.on_write_error, ERROR_ACTIONS, (v) => setEp({ on_write_error: v }), "A file cannot be written")}</Field>
            <Field label="A folder cannot be created">{select(ep.on_dir_error, ERROR_ACTIONS, (v) => setEp({ on_dir_error: v }), "A folder cannot be created")}</Field>
            <Field label="Data is missing or damaged">{select(ep.on_missing_data, ERROR_ACTIONS, (v) => setEp({ on_missing_data: v }), "Data is missing or damaged")}</Field>
            <Field label="Permissions, owner or times cannot be set">{select(ep.on_metadata_error, METADATA_ACTIONS, (v) => setEp({ on_metadata_error: v }), "Metadata errors")}</Field>
            <Field label="Stop after more than N problems (empty = never)">
              <input
                className={inputCls}
                type="number"
                min={0}
                value={ep.max_errors ?? ""}
                onChange={(e) => setEp({ max_errors: e.target.value === "" ? null : Math.max(0, Math.floor(Number(e.target.value))) })}
                aria-label="Stop after more than N problems"
              />
            </Field>
          </div>
          <Check label="Stop at the first error (fail fast)" checked={ep.fail_fast} onChange={(v) => setEp({ fail_fast: v })} />
          <Check label="Read volumes through the local volume cache" checked={value.use_cache} onChange={(v) => onChange({ ...value, use_cache: v })} hint="Useful when restoring the same data repeatedly; a large one-off restore would only fill the cache." />
        </details>
      </AdvancedOnly>
    </div>
  );
}

/** The dry-run deletion list of a mirror restore, with confirm / cancel. */
export function MirrorConfirm({ deletions, busy, onConfirm, onCancel }: { deletions: string[]; busy: boolean; onConfirm: () => void; onCancel: () => void }) {
  return (
    <Banner tone="warning">
      <div className="space-y-1" data-testid="mirror-confirm">
        <div className="font-medium">{deletionSummary(deletions)} will be deleted from the target folder:</div>
        <ul className="max-h-40 overflow-y-auto font-mono text-[11px]">
          {deletions.slice(0, 200).map((p) => (
            <li key={p} className="truncate">
              − {p}
            </li>
          ))}
          {deletions.length > 200 && <li>… and {deletions.length - 200} more</li>}
        </ul>
        <div className="flex gap-1">
          <Btn tone="danger" disabled={busy} onClick={onConfirm} testId="mirror-confirm-delete">
            Restore and delete them
          </Btn>
          <Btn disabled={busy} onClick={onCancel}>
            Cancel
          </Btn>
        </div>
      </div>
    </Banner>
  );
}
