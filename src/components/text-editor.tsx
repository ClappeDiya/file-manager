/**
 * Built-in Text Editor Component (Phase 2)
 *
 * Minimal in-app editor for files <1MB.
 * Features:
 * - Syntax-aware textarea (upgradeable to CodeMirror 6)
 * - Line numbers
 * - Find/replace (Cmd+F / Cmd+H)
 * - Save back to remote (Cmd+S)
 * - Max 1MB file size limit
 * - NOT a full IDE - no file tree, no extensions, no terminal
 */
import { useState, useEffect, useRef, useCallback } from "react";

const MAX_FILE_SIZE = 1024 * 1024; // 1MB

interface TextEditorProps {
  /** Initial file content */
  content: string;
  /** File name for display */
  fileName: string;
  /** File size in bytes */
  fileSize: number;
  /** Language/syntax hint */
  language?: string;
  /** Whether the file is read-only */
  readOnly?: boolean;
  /** Called when user saves (Cmd+S). Awaited so the Saving… state
   *  stays visible until the write completes (iter 32). */
  onSave: (content: string) => void | Promise<void>;
  /** Called when user closes the editor */
  onClose: () => void;
  /** Iter 34: fired whenever the internal dirty flag flips. Lets
   *  the hosting modal block accidental close when there are
   *  unsaved edits. Optional so direct consumers (if any ever
   *  exist outside `TextEditorModal`) don't break. */
  onDirtyChange?: (isDirty: boolean) => void;
}

export function TextEditor({
  content: initialContent,
  fileName,
  fileSize,
  language,
  readOnly = false,
  onSave,
  onClose,
  onDirtyChange,
}: TextEditorProps) {
  const [content, setContent] = useState(initialContent);
  const [isDirty, setIsDirty] = useState(false);
  const [saving, setSaving] = useState(false);
  const [showFind, setShowFind] = useState(false);
  const [findQuery, setFindQuery] = useState("");
  const [replaceQuery, setReplaceQuery] = useState("");
  const [matchCount, setMatchCount] = useState(0);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const findInputRef = useRef<HTMLInputElement>(null);

  const tooLarge = fileSize > MAX_FILE_SIZE;

  const handleSave = useCallback(async () => {
    if (readOnly || saving || tooLarge) return;
    setSaving(true);
    try {
      await onSave(content);
      setIsDirty(false);
    } catch {
      // Iter 34: host either blocked the save (external-edit
      // conflict from iter 33) or the write itself failed.
      // Either way, keep `isDirty=true` so the "Modified"
      // indicator stays visible, the close-confirm guard fires,
      // and the user knows their edits are not yet persisted.
      // The host component (TextEditorModal) is responsible for
      // surfacing the reason via its own banner state.
    } finally {
      setSaving(false);
    }
  }, [content, readOnly, saving, tooLarge, onSave]);

  const handleReplace = useCallback(() => {
    if (!findQuery || readOnly || tooLarge) return;
    const escaped = findQuery.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    const regex = new RegExp(escaped, "g");
    setContent(content.replace(regex, replaceQuery));
    setIsDirty(true);
  }, [findQuery, replaceQuery, content, readOnly, tooLarge]);

  // Handle keyboard shortcuts
  useEffect(() => {
    if (tooLarge) return;
    const handler = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "s") {
        e.preventDefault();
        if (!readOnly && isDirty) {
          handleSave();
        }
      }
      if ((e.metaKey || e.ctrlKey) && e.key === "f") {
        e.preventDefault();
        setShowFind(true);
        requestAnimationFrame(() => findInputRef.current?.focus());
      }
      if (e.key === "Escape" && showFind) {
        setShowFind(false);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [readOnly, isDirty, showFind, tooLarge, handleSave]);

  // Iter 34: bubble dirty-state changes to the host so it can
  // block accidental close with an unsaved-changes warning.
  // Fires on every flip (clean→dirty on first edit, dirty→clean
  // after save or reload). The host hooks this via the
  // `onDirtyChange` callback prop.
  useEffect(() => {
    onDirtyChange?.(isDirty);
  }, [isDirty, onDirtyChange]);

  // Update match count when find query changes
  useEffect(() => {
    if (!findQuery) {
      setMatchCount(0);
      return;
    }
    try {
      const regex = new RegExp(findQuery.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), "gi");
      const matches = content.match(regex);
      setMatchCount(matches?.length || 0);
    } catch {
      setMatchCount(0);
    }
  }, [findQuery, content]);

  // File too large guard - render after all hooks
  if (tooLarge) {
    return (
      <div className="flex flex-col items-center justify-center h-full text-text-tertiary text-sm gap-3 p-6">
        <svg className="w-12 h-12 opacity-40" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5">
          <circle cx="12" cy="12" r="10" />
          <path d="M12 8v4M12 16h.01" />
        </svg>
        <p className="font-medium">File too large for built-in editor</p>
        <p className="text-xs">Maximum size: 1 MB. This file is {(fileSize / 1024 / 1024).toFixed(1)} MB.</p>
        <p className="text-xs">Use an external editor for larger files.</p>
        <button
          onClick={onClose}
          className="mt-2 text-sm px-4 py-1.5 rounded bg-blue-500 text-white hover:bg-blue-600"
        >
          Close
        </button>
      </div>
    );
  }

  const lineCount = content.split("\n").length;

  return (
    <div className="flex flex-col h-full bg-surface-primary">
      {/* Toolbar */}
      <div className="flex items-center justify-between px-3 py-2 border-b border-border-primary bg-surface-secondary text-xs">
        <div className="flex items-center gap-2">
          <span className="font-medium">{fileName}</span>
          {language && <span className="text-text-tertiary px-1.5 py-0.5 bg-surface-primary rounded">{language}</span>}
          {isDirty && <span className="text-amber-500 font-medium">Modified</span>}
          {readOnly && <span className="text-text-tertiary italic">Read-only</span>}
        </div>
        <div className="flex items-center gap-2">
          <span className="text-text-tertiary">{lineCount} lines</span>
          {!readOnly && (
            <button
              onClick={handleSave}
              disabled={!isDirty || saving}
              className="px-2 py-0.5 rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-40"
            >
              {saving ? "Saving..." : "Save"}
            </button>
          )}
          <button
            onClick={onClose}
            className="p-0.5 rounded text-text-tertiary hover:bg-surface-hover"
            title="Close editor"
          >
            <svg className="w-4 h-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M18 6L6 18M6 6l12 12" />
            </svg>
          </button>
        </div>
      </div>

      {/* Find/Replace bar */}
      {showFind && (
        <div className="flex items-center gap-2 px-3 py-1.5 border-b border-border-primary bg-surface-secondary">
          <input
            ref={findInputRef}
            type="text"
            value={findQuery}
            onChange={(e) => setFindQuery(e.target.value)}
            placeholder="Find..."
            className="flex-1 text-xs px-2 py-1 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
          />
          {!readOnly && (
            <>
              <input
                type="text"
                value={replaceQuery}
                onChange={(e) => setReplaceQuery(e.target.value)}
                placeholder="Replace..."
                className="flex-1 text-xs px-2 py-1 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
              />
              <button
                onClick={handleReplace}
                disabled={!findQuery}
                className="text-xs px-2 py-1 rounded border border-gray-300 dark:border-gray-600 hover:bg-gray-100 dark:hover:bg-gray-800 disabled:opacity-40"
              >
                Replace All
              </button>
            </>
          )}
          <span className="text-xs text-text-tertiary">{matchCount} matches</span>
          <button
            onClick={() => setShowFind(false)}
            className="text-xs text-text-tertiary hover:text-text-primary"
          >
            Close
          </button>
        </div>
      )}

      {/* Editor area */}
      <div className="flex-1 overflow-auto relative">
        <div className="flex min-h-full">
          {/* Line numbers */}
          <div className="sticky left-0 select-none text-right pr-3 pl-2 pt-2 text-[10px] font-mono text-text-tertiary bg-surface-secondary border-r border-border-primary shrink-0">
            {Array.from({ length: lineCount }, (_, i) => (
              <div key={i + 1} className="leading-5">{i + 1}</div>
            ))}
          </div>

          {/* Text area */}
          <textarea
            ref={textareaRef}
            value={content}
            onChange={(e) => {
              setContent(e.target.value);
              setIsDirty(true);
            }}
            readOnly={readOnly}
            spellCheck={false}
            className="flex-1 min-w-0 p-2 text-xs leading-5 font-mono bg-transparent resize-none outline-none text-text-primary whitespace-pre overflow-x-auto"
            style={{ tabSize: 2 }}
          />
        </div>
      </div>
    </div>
  );
}
