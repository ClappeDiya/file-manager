/** Types for the Rust `logging::Retention` and `diagnostics::SavedBundle`. */

/** Mirrors `logging::Retention`. */
export interface LogRetention {
  max_age_days: number;
  max_total_mb: number;
}

/** Mirrors `diagnostics::SavedBundle`. */
export interface SavedBundle {
  path: string;
  size: number;
  files: string[];
  truncated: boolean;
}

export const DEFAULT_RETENTION: LogRetention = { max_age_days: 14, max_total_mb: 200 };
