/**
 * Text Editor Modal (iter 32, hardened in iter 33)
 *
 * Thin wrapper that hosts the long-orphaned `TextEditor` (234 LOC)
 * inside the file-manager modal chrome and drives it with the two
 * IPCs added to `fs_commands.rs` in iter 32:
 *
 *   - `read_text_file_full(path)  -> String`
 *   - `write_text_file_full(path, content) -> ()`
 *
 * The raw `TextEditor` accepts `content` as a prop at mount time and
 * emits `onSave(content)` when the user hits Cmd+S — this wrapper
 * bridges that pure-UI contract to the filesystem via the IPC layer
 * so `TextEditor` itself stays unchanged (zero touch on the orphan).
 *
 * Iter 33 — External-edit conflict detection:
 *   When the file is loaded, the wrapper captures its mtime as a
 *   baseline. On every save attempt, the wrapper re-reads mtime via
 *   the existing `get_file_metadata` IPC and compares. If the file
 *   was modified externally (another editor, a script, a git pull,
 *   a sync engine) since the user opened it, the write is BLOCKED
 *   and a conflict banner appears offering two explicit resolutions:
 *     • "Overwrite anyway" — proceed with the user's content,
 *       losing the external changes. Baseline updates after.
 *     • "Reload from disk"  — discard the user's edits, re-read
 *       the external content, remount the editor fresh.
 *   Cancel is implicit (close the banner without acting). This
 *   hardening eliminates the "silent data loss" class of bug that
 *   iter 32's bare save path carried: without the check, an
 *   unattended editor could clobber a concurrent sync.
 *
 * Lifecycle:
 *   1. Parent opens the modal by setting `filePath` to a non-null value.
 *   2. This component fetches the file content + mtime in parallel.
 *   3. On success, `TextEditor` is mounted with the fetched content.
 *   4. User edits → Cmd+S → wrapper re-checks mtime → maybe blocks
 *      with a conflict banner, or writes and updates baseline.
 *   5. Parent closes the modal by setting `filePath` to null (unmounts).
 *
 * Errors during load fall through to a friendly message with a close
 * button. Errors during save stay visible inside the modal so the
 * user doesn't lose their unsaved work.
 */
import { useState, useEffect, useCallback, useRef } from "react";
import { tauriInvoke, isTauriAvailable } from "@/hooks/use-tauri";
import { dispatchRefresh } from "@/lib/refresh-affected";
import { TextEditor } from "./text-editor";

/** Shape of the `get_file_metadata` IPC response that this wrapper
 *  actually uses. The real `FileEntry` type carries more fields, but
 *  structurally-typed destructuring lets us stay decoupled from the
 *  full schema. */
interface FileMetaPartial {
  modified: string | null;
}

interface TextEditorModalProps {
  /** Absolute path to the file being edited. Non-null when open. */
  filePath: string;
  /** File size in bytes as reported by the file listing. Used to
   *  render the "too large" guard inside TextEditor without a
   *  round-trip to the backend. */
  fileSize: number;
  /** Close the modal — parent sets `filePath` state to null. */
  onClose: () => void;
}

/** Map a file extension to a language hint for TextEditor's syntax
 *  badge. The component itself is extension-agnostic; this is purely
 *  cosmetic ("JSON", "Markdown", etc.). Mirrors the same extension
 *  whitelist file-manager.tsx uses to decide when to open this modal. */
function detectLanguage(path: string): string {
  const lower = path.toLowerCase();
  const ext = lower.split(".").pop() || "";
  switch (ext) {
    case "md":
    case "markdown":
      return "Markdown";
    case "json":
      return "JSON";
    case "yaml":
    case "yml":
      return "YAML";
    case "toml":
      return "TOML";
    case "xml":
      return "XML";
    case "csv":
      return "CSV";
    case "log":
      return "Log";
    case "conf":
    case "ini":
      return "Config";
    case "env":
      return "Env";
    case "txt":
    default:
      return "Text";
  }
}

function basename(path: string): string {
  return path.split("/").filter(Boolean).pop() || path;
}

export function TextEditorModal({ filePath, fileSize, onClose }: TextEditorModalProps) {
  const [content, setContent] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  // Iter 33: baseline mtime captured when the file is first loaded
  // (and re-captured after every successful save or reload). Used
  // by `handleSave` to detect external modifications before
  // committing the user's write.
  const [baselineMtime, setBaselineMtime] = useState<string | null>(null);
  // Iter 33: conflict-banner state. When non-null the banner is
  // showing and `pendingContent` holds the user's typed content
  // that they were trying to save at the moment the conflict was
  // detected. The "Overwrite anyway" action feeds this back into
  // the write path.
  const [pendingContent, setPendingContent] = useState<string | null>(null);
  // Iter 33: bumped on every "Reload from disk" action. The
  // `TextEditor` child uses this as part of its React key so the
  // child unmounts and remounts with a fresh `initialContent`
  // prop. Save-success does NOT bump this, so the cursor and
  // dirty state are preserved on ordinary saves.
  const [reloadCounter, setReloadCounter] = useState(0);
  // Iter 34: mirror of the `TextEditor` child's internal dirty
  // flag, bubbled up via the `onDirtyChange` callback the orphan
  // was extended to support. Drives `handleClose` below so
  // clicking the X (or anything else that calls onClose) surfaces
  // an explicit "discard unsaved edits?" confirm instead of
  // silently dropping the user's work. A ref mirror lets the
  // latest handlers read the current value without adding
  // `dirty` to their dependency arrays (which would churn the
  // callbacks on every keystroke and invalidate the `key`
  // stability that iter 33 depends on).
  const [dirty, setDirty] = useState(false);
  const dirtyRef = useRef(false);
  useEffect(() => {
    dirtyRef.current = dirty;
  }, [dirty]);

  // Load file content on mount. Re-runs when `filePath` changes so the
  // parent can re-use the same modal instance for a different file if
  // that pattern ever emerges.
  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    setContent(null);
    setBaselineMtime(null);
    setPendingContent(null);
    setSaveError(null);

    if (!isTauriAvailable()) {
      // Browser-mode fallback: render a placeholder so component
      // tests and the in-browser dev harness don't crash. The real
      // desktop app always goes through the IPC path.
      if (!cancelled) {
        setContent(`// ${basename(filePath)}\n// (browser preview — content is not loaded)`);
        setLoading(false);
      }
      return () => {
        cancelled = true;
      };
    }

    // Iter 33: fetch content AND mtime in parallel so the baseline
    // for conflict detection is captured at the exact same moment
    // as the content snapshot. If the metadata call fails the
    // editor still opens — we just lose conflict protection for
    // this session and log a console warning.
    Promise.all([
      tauriInvoke<string>("read_text_file_full", { path: filePath }),
      tauriInvoke<FileMetaPartial>("get_file_metadata", { path: filePath }).catch((err) => {
        console.warn("[text-editor-modal] mtime baseline unavailable:", err);
        return { modified: null } as FileMetaPartial;
      }),
    ])
      .then(([text, meta]) => {
        if (!cancelled) {
          setContent(text);
          setBaselineMtime(meta.modified);
          setLoading(false);
        }
      })
      .catch((err) => {
        if (!cancelled) {
          const msg = err instanceof Error ? err.message : String(err);
          setError(msg);
          setLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [filePath]);

  // Iter 33: the actual filesystem write path, factored out so
  // both the happy-path save and the "Overwrite anyway" action can
  // share it without duplicating the IPC call + baseline refresh.
  // Iter 35: also fires `dispatchRefresh` for the saved path so
  // every pane that happens to be viewing the file's parent
  // directory picks up the new mtime INSTANTLY instead of waiting
  // for the ~20s ledger tail poll. The Rust side also records the
  // save in the unified OperationLedger (kind="edit_text"), so
  // the activity-timeline panel shows the event and the tail poll
  // will idempotently fire a second dispatchRefresh; the duplicate
  // is a no-op because `shouldRefreshPane` is a pure function.
  const writeAndRefreshBaseline = useCallback(
    async (nextContent: string) => {
      await tauriInvoke<void>("write_text_file_full", {
        path: filePath,
        content: nextContent,
      });
      setContent(nextContent);
      // Re-capture mtime immediately after write so subsequent
      // saves in the same session don't trip the conflict check
      // just because we ourselves just modified the file.
      try {
        const meta = await tauriInvoke<FileMetaPartial>("get_file_metadata", {
          path: filePath,
        });
        setBaselineMtime(meta.modified);
      } catch (err) {
        console.warn("[text-editor-modal] baseline refresh failed:", err);
      }
      // Iter 35: instant cross-pane refresh. Passing the saved
      // FILE path (not the parent) because `shouldRefreshPane`
      // matches child paths against the pane's viewed directory
      // via prefix logic — see iter-18/19 helper semantics.
      dispatchRefresh([filePath]);
    },
    [filePath],
  );

  const handleSave = useCallback(
    async (nextContent: string) => {
      setSaveError(null);
      if (!isTauriAvailable()) {
        setSaveError("Saving is disabled in browser preview mode.");
        // Iter 34: throw so the `TextEditor` child keeps its
        // internal dirty flag set and the "Modified" badge stays
        // visible. Without this throw, iter 33's conflict path
        // and every error path would silently mark the editor
        // clean even though the user's bytes were never written.
        throw new Error("Saving disabled in browser preview.");
      }
      try {
        // Iter 33: pre-write conflict check. If the file's on-disk
        // mtime differs from the baseline captured at load time,
        // block the write and show the conflict banner. A missing
        // baseline (metadata IPC failed on load) skips the check
        // so the editor still functions — we prefer a small risk
        // of silent overwrite over locking the user out entirely.
        if (baselineMtime !== null) {
          const meta = await tauriInvoke<FileMetaPartial>("get_file_metadata", {
            path: filePath,
          });
          if (meta.modified !== null && meta.modified !== baselineMtime) {
            setPendingContent(nextContent);
            // Iter 34: see comment above — rethrow so the child
            // stays dirty until the user resolves the banner.
            throw new Error("UFOP_CONFLICT_PENDING");
          }
        }
        await writeAndRefreshBaseline(nextContent);
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        // Iter 34: the conflict sentinel does NOT populate the
        // red saveError banner because the amber conflict banner
        // is already displayed and describes the situation. Real
        // errors still surface in the red banner for the user.
        if (msg !== "UFOP_CONFLICT_PENDING") {
          setSaveError(msg);
        }
        throw err;
      }
    },
    [filePath, baselineMtime, writeAndRefreshBaseline],
  );

  // Iter 33: user chose "Overwrite anyway" in the conflict banner.
  // Proceeds with the write using the content the user was trying
  // to save when the conflict was detected.
  const handleOverwriteAnyway = useCallback(async () => {
    if (pendingContent === null) return;
    setSaveError(null);
    try {
      await writeAndRefreshBaseline(pendingContent);
      setPendingContent(null);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setSaveError(msg);
    }
  }, [pendingContent, writeAndRefreshBaseline]);

  // Iter 33: user chose "Reload from disk" in the conflict banner.
  // Discards the in-flight pending content, re-reads the external
  // version, and bumps `reloadCounter` so the TextEditor child
  // remounts cleanly with the new initial content.
  const handleReloadFromDisk = useCallback(async () => {
    setSaveError(null);
    setPendingContent(null);
    if (!isTauriAvailable()) return;
    try {
      const [text, meta] = await Promise.all([
        tauriInvoke<string>("read_text_file_full", { path: filePath }),
        tauriInvoke<FileMetaPartial>("get_file_metadata", { path: filePath }).catch(
          () => ({ modified: null } as FileMetaPartial),
        ),
      ]);
      setContent(text);
      setBaselineMtime(meta.modified);
      setReloadCounter((n) => n + 1);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setSaveError(msg);
    }
  }, [filePath]);

  const handleDismissConflict = useCallback(() => {
    setPendingContent(null);
  }, []);

  // Iter 34: guard the close path. If the in-editor dirty flag
  // is set we prompt for explicit confirmation before propagating
  // onClose to the parent. Reads the latest dirty value from the
  // ref so this callback doesn't depend on `dirty` directly and
  // therefore stays stable across keystrokes — which is critical
  // because iter 33 gives it to `TextEditor` as `onClose` and we
  // don't want the editor to see a new close handler every time
  // the user types a character. `window.confirm` is a synchronous
  // native prompt; the codebase already uses it in `file-manager
  // .tsx` for new-folder conflict checks, so the pattern is
  // established.
  const handleClose = useCallback(() => {
    if (dirtyRef.current) {
      const ok = window.confirm(
        "You have unsaved changes. Close without saving?",
      );
      if (!ok) return;
    }
    onClose();
  }, [onClose]);

  if (loading) {
    return (
      <div className="flex flex-col items-center justify-center h-full text-xs text-[color:var(--color-text-tertiary)]">
        <div className="animate-pulse">Loading {basename(filePath)}…</div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex flex-col items-center justify-center h-full gap-3 p-6 text-xs">
        <p className="font-medium text-red-500">Cannot open file</p>
        <p className="text-[color:var(--color-text-tertiary)] text-center max-w-md">{error}</p>
        <button
          onClick={handleClose}
          className="mt-2 text-sm px-4 py-1.5 rounded bg-[var(--color-accent)] text-white"
        >
          Close
        </button>
      </div>
    );
  }

  if (content === null) {
    return null;
  }

  return (
    <div className="flex flex-col h-full">
      {pendingContent !== null && (
        /* Iter 33: external-modification conflict banner. Appears
           when a save was blocked because the file's mtime
           changed under us. Offers explicit resolution — never
           silently discards either side's changes. */
        <div
          className="px-3 py-2 text-xs bg-amber-500/10 text-amber-600 dark:text-amber-400 border-b border-amber-500/30 flex items-center gap-2 flex-wrap"
          role="alert"
          data-testid="text-editor-conflict-banner"
        >
          <span className="font-medium">
            ⚠ This file was changed outside the editor since you opened it.
          </span>
          <div className="flex items-center gap-1 ml-auto">
            <button
              onClick={handleOverwriteAnyway}
              className="px-2 py-0.5 rounded bg-amber-500 text-white hover:bg-amber-600"
              data-testid="text-editor-overwrite-anyway"
            >
              Overwrite anyway
            </button>
            <button
              onClick={handleReloadFromDisk}
              className="px-2 py-0.5 rounded border border-amber-500/50 hover:bg-amber-500/10"
              data-testid="text-editor-reload-from-disk"
            >
              Reload from disk
            </button>
            <button
              onClick={handleDismissConflict}
              className="px-2 py-0.5 rounded text-[color:var(--color-text-tertiary)] hover:bg-[var(--color-hover)]"
              aria-label="Dismiss"
            >
              ✕
            </button>
          </div>
        </div>
      )}
      {saveError && (
        <div className="px-3 py-2 text-xs bg-red-500/10 text-red-500 border-b border-red-500/30">
          Save failed: {saveError}
        </div>
      )}
      <div className="flex-1 min-h-0">
        <TextEditor
          key={`${filePath}#${reloadCounter}`}
          content={content}
          fileName={basename(filePath)}
          fileSize={fileSize}
          language={detectLanguage(filePath)}
          onSave={handleSave}
          onClose={handleClose}
          onDirtyChange={setDirty}
        />
      </div>
    </div>
  );
}
