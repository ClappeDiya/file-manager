/**
 * The sync pair form's state and its mapping to and from a stored pair, so
 * creating and editing a pair use one form (sync-panel.tsx).
 *
 * Copy options are layered (WS-C item 52): a pair created here stores the
 * options it changes from the global sync defaults in `option_overrides`;
 * everything else follows the defaults. Pairs from before (no
 * `option_overrides`) keep all their own options.
 */
import { DEFAULT_COPY_OPTIONS, type CopyOptions } from "@/lib/copy-options";
import type { FilterRule } from "@/stores/backup-store";
import { DEFAULT_RUN_OPTIONS, type SyncFilter, type SyncPair, type SyncRunOptions } from "@/stores/sync-store";

export interface SyncPairForm {
  name: string;
  sourcePath: string;
  destPath: string;
  mode: string;
  trigger: string;
  cronExpr: string;
  /** Whether the pair runs (on its trigger, and from "Sync Now"). */
  enabled: boolean;
  conflictPolicy: string;
  verifyMode: string;
  checksumEnabled: boolean;
  destState: string;
  checksumAlgorithm: string;
  advancedMode: boolean;
  excludePatterns: string;
  includePatterns: string;
  fileTypes: string;
  maxSize: number;
  minSize: number;
  timeOffsetSecs: number | null;
  copyOptions: CopyOptions;
  runOptions: SyncRunOptions;
  presetId: string | null;
  /** Ordered include/exclude rules (Advanced editor; presets in both views). */
  rules: FilterRule[];
  /** Filter lists the form has no fields for, kept as they were. */
  includeRegex: string[];
  excludeRegex: string[];
  excludeFileTypes: string[];
}

export const DEFAULT_CRON = "0 0 */6 * * * *";

export function emptyPairForm(defaults: CopyOptions = DEFAULT_COPY_OPTIONS): SyncPairForm {
  return {
    name: "",
    sourcePath: "",
    destPath: "",
    mode: "one_way",
    trigger: "manual",
    cronExpr: DEFAULT_CRON,
    enabled: true,
    conflictPolicy: "create_conflict_copy",
    verifyMode: "fast",
    checksumEnabled: false,
    destState: "scan",
    checksumAlgorithm: "xxh3",
    advancedMode: false,
    excludePatterns: "",
    includePatterns: "",
    fileTypes: "",
    maxSize: 0,
    minSize: 0,
    timeOffsetSecs: null,
    copyOptions: defaults,
    runOptions: DEFAULT_RUN_OPTIONS,
    presetId: null,
    rules: [],
    includeRegex: [],
    excludeRegex: [],
    excludeFileTypes: [],
  };
}

const splitList = (s: string) =>
  s
    .split(",")
    .map((x) => x.trim())
    .filter(Boolean);

/** The pair's filter as the form edits it. */
export function filterFromForm(form: SyncPairForm): SyncFilter {
  return {
    include_patterns: splitList(form.includePatterns),
    exclude_patterns: splitList(form.excludePatterns),
    include_regex: form.includeRegex,
    exclude_regex: form.excludeRegex,
    min_size: form.minSize,
    max_size: form.maxSize,
    file_types: splitList(form.fileTypes),
    exclude_file_types: form.excludeFileTypes,
    rules: form.rules,
  };
}

/** Option names whose value differs from `defaults`. */
export function changedOptionKeys(options: CopyOptions, defaults: CopyOptions): string[] {
  return (Object.keys(DEFAULT_COPY_OPTIONS) as (keyof CopyOptions)[]).filter(
    (k) => JSON.stringify(options[k]) !== JSON.stringify(defaults[k]),
  );
}

/** Effective options: the defaults, with the pair's own value for each override. */
export function layerOptions(defaults: CopyOptions, own: CopyOptions, overrides: string[] | null | undefined): CopyOptions {
  if (!overrides) return { ...DEFAULT_COPY_OPTIONS, ...own };
  const out: Record<string, unknown> = { ...DEFAULT_COPY_OPTIONS, ...defaults };
  for (const k of overrides) {
    if (k in own) out[k] = (own as unknown as Record<string, unknown>)[k];
  }
  return out as unknown as CopyOptions;
}

function triggerParts(trigger: SyncPair["trigger"]): { trigger: string; cronExpr: string } {
  if (typeof trigger === "string") return { trigger, cronExpr: DEFAULT_CRON };
  if (trigger && "scheduled" in trigger) return { trigger: "scheduled", cronExpr: trigger.scheduled.cron_expr };
  return { trigger: "manual", cronExpr: DEFAULT_CRON };
}

/** Fill the form from a stored pair (editing). */
export function formFromPair(pair: SyncPair, defaults: CopyOptions = DEFAULT_COPY_OPTIONS): SyncPairForm {
  const f = pair.filter;
  return {
    ...emptyPairForm(defaults),
    name: pair.name,
    sourcePath: pair.source_path,
    destPath: pair.dest_path,
    mode: pair.mode,
    ...triggerParts(pair.trigger),
    enabled: pair.enabled,
    conflictPolicy: pair.conflict_policy,
    verifyMode: pair.verify_mode,
    checksumEnabled: pair.checksum_enabled,
    destState: pair.dest_state ?? "scan",
    checksumAlgorithm: pair.checksum_algorithm ?? "xxh3",
    advancedMode: true,
    excludePatterns: f.exclude_patterns.join(", "),
    includePatterns: f.include_patterns.join(", "),
    fileTypes: f.file_types.join(", "),
    maxSize: f.max_size,
    minSize: f.min_size,
    timeOffsetSecs: pair.time_offset_secs,
    copyOptions: layerOptions(defaults, pair.copy_options ?? DEFAULT_COPY_OPTIONS, pair.option_overrides),
    runOptions: { ...DEFAULT_RUN_OPTIONS, ...pair.run_options },
    rules: f.rules ?? [],
    includeRegex: f.include_regex,
    excludeRegex: f.exclude_regex,
    excludeFileTypes: f.exclude_file_types,
  };
}

/** The updated pair to save: the stored pair with the form's settings. */
export function pairFromForm(form: SyncPairForm, original: SyncPair, defaults: CopyOptions): SyncPair {
  const trigger: SyncPair["trigger"] =
    form.trigger === "scheduled" ? { scheduled: { cron_expr: form.cronExpr } } : form.trigger;
  return {
    ...original,
    name: form.name,
    source_path: form.sourcePath,
    dest_path: form.destPath,
    mode: form.mode,
    trigger,
    enabled: form.enabled,
    filter: filterFromForm(form),
    conflict_policy: form.conflictPolicy,
    verify_mode: form.verifyMode,
    checksum_enabled: form.checksumEnabled,
    dest_state: form.destState as SyncPair["dest_state"],
    checksum_algorithm: form.checksumAlgorithm as SyncPair["checksum_algorithm"],
    time_offset_secs: form.timeOffsetSecs,
    copy_options: form.copyOptions,
    run_options: form.runOptions,
    // Pairs from before layering keep all their own options.
    option_overrides: original.option_overrides ? changedOptionKeys(form.copyOptions, defaults) : null,
  };
}
