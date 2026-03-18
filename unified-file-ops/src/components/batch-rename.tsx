/**
 * Batch Rename Engine Component (T-060)
 *
 * Features:
 * - Live preview of all changes as user types
 * - Token substitution: {name}, {ext}, {num}, {date}, {parent}, {counter}
 * - Find/Replace with regex (Advanced) and literal (Simple)
 * - Sequential numbering with start/step/zero-padding
 * - Case transformation: UPPER, lower, Title, Sentence
 * - Undo renames back to originals
 * - Preview uses compatibility engine for warnings
 */
import { useState, useCallback, useEffect, useRef } from "react";
import { tauriInvoke } from "@/hooks/use-tauri";

// ── Types ──

interface RenamePreviewEntry {
  original_path: string;
  original_name: string;
  new_name: string;
  changed: boolean;
  warnings: string[];
}

interface RenameParams {
  files: string[];
  pattern: string | null;
  find: string | null;
  replace: string | null;
  use_regex: boolean;
  case_transform: string | null;
  counter_start: number;
  counter_step: number;
  counter_pad: number;
}

interface RenameResult {
  success_count: number;
  fail_count: number;
  undo_id: string;
  errors: string[];
}

type RenameMode = "pattern" | "find-replace" | "case";

interface BatchRenameProps {
  files: string[];
  onClose?: () => void;
  onComplete?: (result: RenameResult) => void;
}

// ── Component ──

export function BatchRename({ files, onClose, onComplete }: BatchRenameProps) {
  const [mode, setMode] = useState<RenameMode>("pattern");
  const [pattern, setPattern] = useState("{name}.{ext}");
  const [findText, setFindText] = useState("");
  const [replaceText, setReplaceText] = useState("");
  const [useRegex, setUseRegex] = useState(false);
  const [caseTransform, setCaseTransform] = useState<string | null>(null);
  const [counterStart, setCounterStart] = useState(1);
  const [counterStep, setCounterStep] = useState(1);
  const [counterPad, setCounterPad] = useState(0);
  const [preview, setPreview] = useState<RenamePreviewEntry[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [isApplying, setIsApplying] = useState(false);
  const [lastUndoId, setLastUndoId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [successMessage, setSuccessMessage] = useState<string | null>(null);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Build params from current state
  const buildParams = useCallback((): RenameParams => {
    return {
      files,
      pattern: mode === "pattern" ? pattern : null,
      find: mode === "find-replace" ? findText || null : null,
      replace: mode === "find-replace" ? replaceText || null : null,
      use_regex: useRegex,
      case_transform: mode === "case" ? caseTransform : null,
      counter_start: counterStart,
      counter_step: counterStep,
      counter_pad: counterPad,
    };
  }, [files, mode, pattern, findText, replaceText, useRegex, caseTransform, counterStart, counterStep, counterPad]);

  // Fetch live preview
  const fetchPreview = useCallback(async () => {
    if (files.length === 0) return;
    setIsLoading(true);
    setError(null);
    try {
      const params = buildParams();
      const result = await tauriInvoke<RenamePreviewEntry[]>("batch_rename_preview", { params });
      setPreview(result);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setIsLoading(false);
    }
  }, [files, buildParams]);

  // Debounced preview update on any input change
  useEffect(() => {
    if (debounceRef.current) {
      clearTimeout(debounceRef.current);
    }
    debounceRef.current = setTimeout(() => {
      fetchPreview();
    }, 200);
    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
  }, [fetchPreview]);

  // Apply renames
  const handleApply = async () => {
    setIsApplying(true);
    setError(null);
    setSuccessMessage(null);
    try {
      const params = buildParams();
      const result = await tauriInvoke<RenameResult>("batch_rename_apply", { params });
      setLastUndoId(result.undo_id);
      setSuccessMessage(
        `Renamed ${result.success_count} file(s)${result.fail_count > 0 ? `, ${result.fail_count} failed` : ""}`
      );
      if (result.errors.length > 0) {
        setError(result.errors.join("\n"));
      }
      onComplete?.(result);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setIsApplying(false);
    }
  };

  // Undo renames
  const handleUndo = async () => {
    if (!lastUndoId) return;
    try {
      const result = await tauriInvoke<RenameResult>("batch_rename_undo", { undoId: lastUndoId });
      setSuccessMessage(`Undone: ${result.success_count} file(s) restored`);
      setLastUndoId(null);
      fetchPreview();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const changedCount = preview.filter((p) => p.changed).length;
  const warningCount = preview.filter((p) => p.warnings.length > 0).length;

  return (
    <div className="flex flex-col h-full bg-surface-primary text-text-primary">
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-3 border-b border-border-primary">
        <h2 className="text-lg font-semibold">Batch Rename</h2>
        <div className="flex items-center gap-2">
          <span className="text-sm text-text-secondary">
            {files.length} file(s) selected
          </span>
          {onClose && (
            <button
              onClick={onClose}
              className="p-1 rounded hover:bg-surface-hover text-text-secondary"
              title="Close"
            >
              <svg className="w-5 h-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <path d="M18 6L6 18M6 6l12 12" />
              </svg>
            </button>
          )}
        </div>
      </div>

      {/* Mode tabs */}
      <div className="flex px-4 py-2 gap-1 border-b border-border-primary bg-surface-secondary">
        {([
          ["pattern", "Pattern"],
          ["find-replace", "Find & Replace"],
          ["case", "Case"],
        ] as [RenameMode, string][]).map(([m, label]) => (
          <button
            key={m}
            onClick={() => setMode(m)}
            className={`px-3 py-1.5 text-sm rounded-md transition-colors ${
              mode === m
                ? "bg-accent-primary text-white"
                : "text-text-secondary hover:bg-surface-hover"
            }`}
          >
            {label}
          </button>
        ))}
      </div>

      {/* Controls */}
      <div className="px-4 py-3 space-y-3 border-b border-border-primary">
        {mode === "pattern" && (
          <div className="space-y-2">
            <label className="block text-sm font-medium">
              Pattern
              <span className="ml-2 text-xs text-text-tertiary">
                Tokens: {"{name}"} {"{ext}"} {"{num}"} {"{date}"} {"{parent}"} {"{counter}"}
              </span>
            </label>
            <input
              type="text"
              value={pattern}
              onChange={(e) => setPattern(e.target.value)}
              className="w-full px-3 py-2 text-sm border rounded-md bg-surface-primary border-border-primary focus:outline-none focus:ring-2 focus:ring-accent-primary"
              placeholder="{name}_{counter}.{ext}"
            />
            <div className="flex gap-4">
              <div className="flex items-center gap-2">
                <label className="text-xs text-text-secondary">Start:</label>
                <input
                  type="number"
                  value={counterStart}
                  onChange={(e) => setCounterStart(parseInt(e.target.value) || 1)}
                  className="w-16 px-2 py-1 text-xs border rounded bg-surface-primary border-border-primary"
                />
              </div>
              <div className="flex items-center gap-2">
                <label className="text-xs text-text-secondary">Step:</label>
                <input
                  type="number"
                  value={counterStep}
                  onChange={(e) => setCounterStep(parseInt(e.target.value) || 1)}
                  className="w-16 px-2 py-1 text-xs border rounded bg-surface-primary border-border-primary"
                />
              </div>
              <div className="flex items-center gap-2">
                <label className="text-xs text-text-secondary">Pad:</label>
                <input
                  type="number"
                  value={counterPad}
                  onChange={(e) => setCounterPad(parseInt(e.target.value) || 0)}
                  className="w-16 px-2 py-1 text-xs border rounded bg-surface-primary border-border-primary"
                  min={0}
                  max={10}
                />
              </div>
            </div>
          </div>
        )}

        {mode === "find-replace" && (
          <div className="space-y-2">
            <div className="flex items-center gap-2">
              <label className="block text-sm font-medium flex-1">
                Find
                <input
                  type="text"
                  value={findText}
                  onChange={(e) => setFindText(e.target.value)}
                  className="mt-1 w-full px-3 py-2 text-sm border rounded-md bg-surface-primary border-border-primary focus:outline-none focus:ring-2 focus:ring-accent-primary"
                  placeholder={useRegex ? "\\d+" : "old text"}
                />
              </label>
              <label className="block text-sm font-medium flex-1">
                Replace
                <input
                  type="text"
                  value={replaceText}
                  onChange={(e) => setReplaceText(e.target.value)}
                  className="mt-1 w-full px-3 py-2 text-sm border rounded-md bg-surface-primary border-border-primary focus:outline-none focus:ring-2 focus:ring-accent-primary"
                  placeholder="new text"
                />
              </label>
            </div>
            <label className="flex items-center gap-2 text-sm">
              <input
                type="checkbox"
                checked={useRegex}
                onChange={(e) => setUseRegex(e.target.checked)}
                className="rounded border-border-primary"
              />
              Use regular expression (Advanced)
            </label>
          </div>
        )}

        {mode === "case" && (
          <div className="space-y-2">
            <label className="block text-sm font-medium">Case Transformation</label>
            <div className="flex gap-2">
              {[
                ["upper", "UPPER"],
                ["lower", "lower"],
                ["title", "Title Case"],
                ["sentence", "Sentence case"],
              ].map(([value, label]) => (
                <button
                  key={value}
                  onClick={() => setCaseTransform(caseTransform === value ? null : value)}
                  className={`px-3 py-1.5 text-sm rounded-md border transition-colors ${
                    caseTransform === value
                      ? "bg-accent-primary text-white border-accent-primary"
                      : "border-border-primary text-text-secondary hover:bg-surface-hover"
                  }`}
                >
                  {label}
                </button>
              ))}
            </div>
          </div>
        )}
      </div>

      {/* Preview list */}
      <div className="flex-1 overflow-auto px-4 py-2">
        <div className="flex items-center justify-between mb-2">
          <span className="text-sm text-text-secondary">
            Preview ({changedCount} changed, {warningCount} warnings)
          </span>
          {isLoading && (
            <span className="text-xs text-text-tertiary animate-pulse">Updating...</span>
          )}
        </div>

        <div className="space-y-1">
          {preview.map((entry, i) => (
            <div
              key={i}
              className={`flex items-center gap-2 px-3 py-1.5 rounded text-sm ${
                entry.changed
                  ? "bg-green-50 dark:bg-green-900/20"
                  : "bg-surface-secondary"
              }`}
            >
              <span className="text-text-secondary truncate flex-1" title={entry.original_name}>
                {entry.original_name}
              </span>
              {entry.changed && (
                <>
                  <svg className="w-4 h-4 text-text-tertiary shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                    <path d="M5 12h14M12 5l7 7-7 7" />
                  </svg>
                  <span className="text-green-700 dark:text-green-400 truncate flex-1 font-medium" title={entry.new_name}>
                    {entry.new_name}
                  </span>
                </>
              )}
              {!entry.changed && (
                <span className="text-text-tertiary text-xs italic">unchanged</span>
              )}
              {entry.warnings.length > 0 && (
                <span
                  className="text-amber-500 shrink-0"
                  title={entry.warnings.join("\n")}
                >
                  <svg className="w-4 h-4" viewBox="0 0 24 24" fill="currentColor">
                    <path d="M1 21h22L12 2 1 21zm12-3h-2v-2h2v2zm0-4h-2v-4h2v4z" />
                  </svg>
                </span>
              )}
            </div>
          ))}
        </div>
      </div>

      {/* Error / Success messages */}
      {error && (
        <div className="px-4 py-2 bg-red-50 dark:bg-red-900/30 text-red-700 dark:text-red-400 text-sm border-t border-red-200 dark:border-red-800">
          {error}
        </div>
      )}
      {successMessage && (
        <div className="px-4 py-2 bg-green-50 dark:bg-green-900/30 text-green-700 dark:text-green-400 text-sm border-t border-green-200 dark:border-green-800">
          {successMessage}
        </div>
      )}

      {/* Actions */}
      <div className="flex items-center justify-between px-4 py-3 border-t border-border-primary bg-surface-secondary">
        <div className="flex gap-2">
          {lastUndoId && (
            <button
              onClick={handleUndo}
              className="px-3 py-1.5 text-sm rounded-md border border-border-primary text-text-secondary hover:bg-surface-hover"
            >
              Undo Last Rename
            </button>
          )}
        </div>
        <div className="flex gap-2">
          {onClose && (
            <button
              onClick={onClose}
              className="px-4 py-1.5 text-sm rounded-md border border-border-primary text-text-secondary hover:bg-surface-hover"
            >
              Cancel
            </button>
          )}
          <button
            onClick={handleApply}
            disabled={isApplying || changedCount === 0}
            className="px-4 py-1.5 text-sm rounded-md bg-accent-primary text-white hover:bg-accent-primary/90 disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {isApplying ? "Renaming..." : `Rename ${changedCount} file(s)`}
          </button>
        </div>
      </div>
    </div>
  );
}
