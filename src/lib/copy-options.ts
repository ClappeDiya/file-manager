/**
 * Robocopy-equivalent copy options: types, defaults, presets and helpers.
 * Types mirror `CopyOptions` / `RobocopyJob` in src-tauri core/types.rs and
 * sync_engine/robocopy.rs.
 */

export type SymlinkMode = "follow" | "skip" | "copy_link";

export interface CopyOptions {
  copy_subdirs: boolean;
  include_empty_dirs: boolean;
  max_depth: number;
  exclude_dirs: string[];
  max_age_days: number;
  min_age_days: number;
  exclude_hidden: boolean;
  exclude_readonly: boolean;
  symlinks: SymlinkMode;
  exclude_older: boolean;
  exclude_newer: boolean;
  exclude_changed: boolean;
  exclude_lonely: boolean;
  include_same: boolean;
  fat_time_tolerance: boolean;
  dst_tolerance: boolean;
  purge: boolean;
  exclude_extra: boolean;
  copy_timestamps: boolean;
  copy_dir_timestamps: boolean;
  move_files: boolean;
  move_dirs: boolean;
  create_only: boolean;
  restartable: boolean;
  retries: number;
  retry_wait_secs: number;
  threads: number;
  inter_packet_gap_ms: number;
  run_hours: string;
  log_file: string;
  log_append: boolean;
}

export const DEFAULT_COPY_OPTIONS: CopyOptions = {
  copy_subdirs: true,
  include_empty_dirs: false,
  max_depth: 0,
  exclude_dirs: [],
  max_age_days: 0,
  min_age_days: 0,
  exclude_hidden: false,
  exclude_readonly: false,
  symlinks: "follow",
  exclude_older: false,
  exclude_newer: false,
  exclude_changed: false,
  exclude_lonely: false,
  include_same: false,
  fat_time_tolerance: false,
  dst_tolerance: false,
  purge: false,
  exclude_extra: false,
  copy_timestamps: true,
  copy_dir_timestamps: false,
  move_files: false,
  move_dirs: false,
  create_only: false,
  restartable: false,
  retries: 0,
  retry_wait_secs: 30,
  threads: 1,
  inter_packet_gap_ms: 0,
  run_hours: "",
  log_file: "",
  log_append: false,
};

export interface RobocopyJob {
  source_path: string;
  dest_path: string;
  mode: string;
  trigger: string;
  cron_expr: string | null;
  filter: {
    include_patterns: string[];
    exclude_patterns: string[];
    min_size: number;
    max_size: number;
  };
  copy_options: CopyOptions;
  list_only: boolean;
  ignored: string[];
  unsupported: string[];
}

// ── Presets: the common Robocopy recipes, one click each ──

export interface CopyPreset {
  id: string;
  label: string;
  hint: string;
  mode: string;
  options: Partial<CopyOptions>;
}

const RELIABLE = { retries: 3, retry_wait_secs: 5, restartable: true };

export const COPY_PRESETS: CopyPreset[] = [
  {
    id: "backup",
    label: "Backup",
    hint: "Copy new and changed files; never delete (robocopy /E /Z /R:3 /W:5)",
    mode: "one_way",
    options: { include_empty_dirs: true, ...RELIABLE },
  },
  {
    id: "mirror",
    label: "Mirror",
    hint: "Make the destination an exact copy, deleting extras (robocopy /MIR)",
    mode: "mirror",
    options: { include_empty_dirs: true, copy_dir_timestamps: true, ...RELIABLE },
  },
  {
    id: "move",
    label: "Move",
    hint: "Copy, then remove files and folders from the source (robocopy /MOVE /E)",
    mode: "one_way",
    options: { include_empty_dirs: true, move_files: true, move_dirs: true, ...RELIABLE },
  },
  {
    id: "update",
    label: "Update only",
    hint: "Refresh files that already exist at the destination, never older ones (robocopy /XO /XL)",
    mode: "one_way",
    options: { exclude_older: true, exclude_lonely: true },
  },
  {
    id: "fast",
    label: "Fast bulk copy",
    hint: "8 parallel threads, no per-file waits (robocopy /E /MT:8 /R:1 /W:1)",
    mode: "one_way",
    options: { include_empty_dirs: true, threads: 8, retries: 1, retry_wait_secs: 1 },
  },
];

/** Apply a preset on top of the defaults (presets don't stack). */
export function applyPreset(preset: CopyPreset): CopyOptions {
  return { ...DEFAULT_COPY_OPTIONS, ...preset.options };
}

/** Plain-language meaning of a Robocopy-style exit code. */
export function describeExitCode(code: number): string {
  if (code & 16) return "Fatal error — nothing was copied";
  if (code & 8) return "Some files could not be copied";
  if (code === 0) return "Already in sync — nothing to do";
  if (code === 1) return "Files copied successfully";
  if (code === 2) return "Extra files at destination; nothing copied";
  if (code === 3) return "Files copied; extra files at destination";
  return "Completed with mismatches";
}

/** Number of options that differ from the defaults (shown as a badge). */
export function countChangedOptions(options: CopyOptions): number {
  return (Object.keys(DEFAULT_COPY_OPTIONS) as (keyof CopyOptions)[]).filter(
    (k) => JSON.stringify(options[k]) !== JSON.stringify(DEFAULT_COPY_OPTIONS[k]),
  ).length;
}

