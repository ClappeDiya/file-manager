/**
 * Activity Timeline Store
 *
 * Session-only Zustand store controlling the visibility of the Activity
 * Timeline panel. Intentionally not persisted: the panel is an on-demand
 * pull surface (like the command palette), not a layout preference the
 * user would want restored on next launch.
 *
 * The panel itself fetches data directly from the unified OperationLedger
 * via `ledger_recent` when it mounts, so this store only needs to track
 * open/closed state. Keeping it minimal avoids duplicating ledger data in
 * the frontend and keeps the surface area small.
 *
 * # Drill-in preset
 *
 * Ambient surfaces (engine-pulse status bar, since-last-seen toast,
 * future overview cards) often want to open the timeline pre-filtered
 * to the slice that's relevant *right now* — e.g. "you have 5 errors
 * in the last 5 minutes, click to see them". The `pendingPreset`
 * field is a single-shot intent the caller sets via `openWithPreset`;
 * the panel reads and clears it via `consumePendingPreset` exactly
 * once on mount, then runs from its own local filter state until
 * another caller sets a new preset. Keeping the preset out-of-band
 * from the panel's own state preserves the panel as the owner of its
 * filters while letting external surfaces compose with it cheaply.
 */
import { create } from "zustand";

/** A one-shot intent to open the Activity Timeline pre-filtered. Every
 *  field is optional; the panel applies whichever fields are set and
 *  leaves the others at their current local-state values.
 *
 *  Use `null` to *explicitly clear* a sibling filter — e.g. the
 *  engine-pulse error drill-in passes `engineFilter: "all"` and
 *  `correlationFilter: null` so the user sees every failure, not
 *  just failures inside whatever filter happened to be active.
 *  Omit a field (`undefined`) to leave the user's current value
 *  untouched. */
export interface TimelinePreset {
  /** If true, the panel's "failed only" toggle is forced on. */
  failedOnly?: boolean;
  /** If set, focuses a single engine (e.g. `"fs"`, `"transfer"`) or
   *  `"all"` to clear an active engine filter. */
  engineFilter?: string;
  /** If a string, narrows to that correlation trace. If `null`,
   *  clears any active correlation filter. */
  correlationFilter?: string | null;
  /** If a string, narrows the timeline to events whose `subject_path`
   *  OR `target_path` starts with this prefix — i.e. "what happened
   *  inside this folder?". Caller is responsible for trimming a
   *  trailing separator so prefix matching is unambiguous. Pass
   *  `null` to clear any active path filter; omit to leave it
   *  untouched. Same three-state semantics as `correlationFilter`. */
  pathFilter?: string | null;
}

interface ActivityTimelineState {
  panelOpen: boolean;
  /** Pending preset to apply on the next panel open. Cleared by
   *  `consumePendingPreset` so successive opens start fresh unless
   *  a new caller sets a new preset. */
  pendingPreset: TimelinePreset | null;
  openPanel: () => void;
  closePanel: () => void;
  togglePanel: () => void;
  /** Open the panel and stage a preset for the panel to consume on
   *  mount. Idempotent if the panel is already open — the panel's
   *  mount-effect runs on `pendingPreset` change too, so the preset
   *  is applied without a close/re-open cycle. */
  openWithPreset: (preset: TimelinePreset) => void;
  /** Atomically read and clear the pending preset. Returns `null` if
   *  no preset is pending. Called by the panel exactly once per
   *  intent so the same preset never re-applies on its own. */
  consumePendingPreset: () => TimelinePreset | null;
}

export const useActivityTimelineStore = create<ActivityTimelineState>(
  (set, get) => ({
    panelOpen: false,
    pendingPreset: null,
    openPanel: () => set({ panelOpen: true }),
    closePanel: () => set({ panelOpen: false }),
    togglePanel: () => set((state) => ({ panelOpen: !state.panelOpen })),
    openWithPreset: (preset) =>
      set({ panelOpen: true, pendingPreset: preset }),
    consumePendingPreset: () => {
      const preset = get().pendingPreset;
      if (preset === null) return null;
      set({ pendingPreset: null });
      return preset;
    },
  }),
);
