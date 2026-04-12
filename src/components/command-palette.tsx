import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useFileManagerStore } from "@/stores/file-manager-store";
import { useAiStore, type AiExecutionResult, type ParsedJobConfig } from "@/stores/ai-store";
import {
  applyMruOrdering,
  readCommandUsage,
  recordCommandUsage,
} from "@/lib/command-mru";
import { cn } from "@ufop/ui-components";
import {
  Search,
  FolderPlus,
  FilePlus,
  Copy,
  Scissors,
  Trash2,
  Pencil,
  Columns2,
  PanelLeftClose,
  Settings,
  Sun,
  Moon,
  RefreshCw,
  Star,
  Undo2,
  Layout,
  List,
  Grid3X3,
  AlignJustify,
  CopyPlus,
  Eye,
  Clipboard,
  Globe,
  FileEdit,
  Zap,
  Activity,
  Layers,
  RotateCcw,
  Bookmark,
  BookmarkPlus,
  Loader2,
  Sparkles,
  ArrowRight,
  ArrowLeftRight,
  Shield,
  Filter,
  FolderSync,
  Upload,
  History,
  ShieldCheck,
} from "lucide-react";

// ──────────────────────────────────────────────
// Types
// ──────────────────────────────────────────────

/** Iter 48: secondary shortcut entry. Rendered in the keyboard
 *  cheat sheet below the primary `shortcut` so multi-gesture
 *  features (Jump Ring forward/reverse, future copy/move
 *  modifier pairs, …) can surface every binding without needing
 *  separate palette entries that would clutter ⌘K. Not shown in
 *  the ⌘K palette itself — that surface stays one-row-per-action. */
export interface AdditionalShortcut {
  /** Keyboard glyph string, e.g. "\u2318\u21E7\u2325L". */
  keys: string;
  /** Short hint (≤40 chars) describing what the variant does. */
  hint: string;
}

export interface CommandItem {
  id: string;
  label: string;
  description?: string;
  icon?: React.ReactNode;
  shortcut?: string;
  /** Iter 48: secondary shortcuts surfaced in the keyboard cheat
   *  sheet only. Undefined or empty array = single shortcut,
   *  which is the pre-iter-48 behaviour. */
  additionalShortcuts?: AdditionalShortcut[];
  category: string;
  action: () => void;
  keywords?: string[];
}

interface CommandPaletteProps {
  commands: CommandItem[];
  isOpen: boolean;
  onClose: () => void;
  onExecutionResult?: (result: AiExecutionResult) => void;
}

// ──────────────────────────────────────────────
// Natural language detection
// ──────────────────────────────────────────────

const NL_INTENT_WORDS = new Set([
  "sync", "backup", "move", "copy", "transfer", "upload", "download",
  "encrypt", "find", "show", "when", "every", "schedule", "watch",
  "rename", "archive", "decrypt", "filter", "navigate", "connect",
  "compress", "delete", "clean", "organize",
]);

function isNaturalLanguage(query: string, commands: CommandItem[]): boolean {
  const trimmed = query.trim();
  if (trimmed.length < 16 || !trimmed.includes(" ")) return false;
  if (trimmed.startsWith("/")) return false;

  // If query exactly matches a command label, it's a command
  const queryLower = trimmed.toLowerCase();
  if (commands.some((cmd) => cmd.label.toLowerCase() === queryLower)) return false;

  // Check for intent words
  const words = queryLower.split(/\s+/);
  return words.some((w) => NL_INTENT_WORDS.has(w));
}

// ──────────────────────────────────────────────
// Intent preview card
// ──────────────────────────────────────────────

const INTENT_ICONS: Record<string, React.ReactNode> = {
  sync: <FolderSync className="h-5 w-5" />,
  backup: <FolderSync className="h-5 w-5" />,
  transfer: <Upload className="h-5 w-5" />,
  automation: <Zap className="h-5 w-5" />,
  filter: <Filter className="h-5 w-5" />,
  navigate: <ArrowRight className="h-5 w-5" />,
  vault: <Shield className="h-5 w-5" />,
  rename: <Pencil className="h-5 w-5" />,
  exclusion: <Filter className="h-5 w-5" />,
};

function IntentPreviewCard({ config }: { config: ParsedJobConfig }) {
  const icon = INTENT_ICONS[config.intent] || <Sparkles className="h-5 w-5" />;

  const intentLabel: Record<string, string> = {
    sync: "Create a sync pair",
    backup: "Set up a backup",
    transfer: "Start a transfer",
    automation: "Create an automation rule",
    filter: "Filter files",
    navigate: "Navigate to location",
    vault: "Open encrypted vault",
    rename: "Batch rename",
    exclusion: "Add exclusion patterns",
  };

  const title = intentLabel[config.intent] || `Execute: ${config.intent}`;

  return (
    <div className="px-4 py-4 space-y-3">
      <div className="flex items-start gap-3">
        <div className="mt-0.5 text-[color:var(--color-accent)]">{icon}</div>
        <div className="flex-1 min-w-0 space-y-1">
          <div className="text-[length:var(--font-size-sm)] font-medium text-[color:var(--color-text)]">
            {title}
          </div>
          {(config.source_path || config.dest_path || config.dest_connector) && (
            <div className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-secondary)] font-mono">
              {config.source_path || "..."}
              {(config.dest_path || config.dest_connector) && (
                <span>
                  {" → "}
                  {config.dest_connector || config.dest_path}
                </span>
              )}
            </div>
          )}
          {config.schedule_human && (
            <div className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-secondary)]">
              Schedule: {config.schedule_human}
            </div>
          )}
          {config.direction && (
            <div className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-secondary)]">
              Direction: {config.direction === "bidirectional" ? "Two-way" : "One-way"}
            </div>
          )}
          {config.filter_patterns.length > 0 && (
            <div className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-secondary)]">
              Filter: {config.filter_patterns.join(", ")}
            </div>
          )}
        </div>
      </div>

      {config.confidence < 0.9 && (
        <div className="flex items-center gap-2">
          <div className="flex-1 h-1.5 rounded-full bg-[var(--color-border)] overflow-hidden">
            <div
              className="h-full rounded-full bg-[var(--color-accent)] transition-all"
              style={{ width: `${Math.round(config.confidence * 100)}%` }}
            />
          </div>
          <span className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)] tabular-nums">
            {Math.round(config.confidence * 100)}%
          </span>
        </div>
      )}

      {config.clarification_questions.length > 0 && (
        <div className="space-y-1">
          {config.clarification_questions.map((q, i) => (
            <div
              key={i}
              className="text-[length:var(--font-size-xs)] text-[color:var(--color-warning)] italic"
            >
              {q}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

// ──────────────────────────────────────────────
// Fuzzy search
// ──────────────────────────────────────────────

function fuzzyMatch(query: string, text: string): { match: boolean; score: number } {
  if (!query) return { match: true, score: 0 };

  const queryLower = query.toLowerCase();
  const textLower = text.toLowerCase();

  // Exact substring match gets highest score
  if (textLower.includes(queryLower)) {
    const index = textLower.indexOf(queryLower);
    return { match: true, score: 100 - index };
  }

  // Fuzzy character matching
  let qi = 0;
  let score = 0;
  let lastMatchPos = -1;

  for (let ti = 0; ti < textLower.length && qi < queryLower.length; ti++) {
    if (textLower[ti] === queryLower[qi]) {
      qi++;
      // Consecutive matches score higher
      if (lastMatchPos === ti - 1) {
        score += 10;
      } else {
        score += 5;
      }
      // Word boundary matches score higher
      if (ti === 0 || textLower[ti - 1] === " " || textLower[ti - 1] === "-" || textLower[ti - 1] === "_") {
        score += 15;
      }
      lastMatchPos = ti;
    }
  }

  return { match: qi === queryLower.length, score };
}

// ──────────────────────────────────────────────
// CommandPalette component
// ──────────────────────────────────────────────

export function CommandPalette({ commands, isOpen, onClose, onExecutionResult }: CommandPaletteProps) {
  const [query, setQuery] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  // Intent Bar state from AI store
  const {
    ollamaAvailable,
    intentPhase,
    currentIntentPreview,
    probeOllama,
    submitIntent,
    confirmIntent,
    clearIntent,
  } = useAiStore();

  // Iter 24: read the MRU usage map fresh on every palette open.
  // Using `useState` (rather than `useRef` + version counter)
  // keeps the memo deps honest: when the map changes, React
  // triggers a re-render AND the memo's dep array picks it up
  // naturally via the `mruUsage` reference. The one extra
  // render per open is trivial compared to a clean dep list
  // that doesn't need a lint suppression.
  const [mruUsage, setMruUsage] = useState(() => readCommandUsage());
  useEffect(() => {
    if (isOpen) {
      setMruUsage(readCommandUsage());
    }
  }, [isOpen]);

  // Filter and sort by fuzzy match score
  const filteredCommands = useMemo(() => {
    // Iter 24: empty query \u2192 frecency-sorted list instead of the
    // raw declaration order. Commands the user has executed before
    // rise to the top (sorted by useCount DESC, lastUsedAt DESC);
    // everything else keeps its original declaration order below.
    // On first use (no localStorage record), this is a no-op and
    // the palette behaves exactly like iter 23.
    if (!query.trim()) return applyMruOrdering(commands, mruUsage);

    const scored = commands
      .map((cmd) => {
        const labelMatch = fuzzyMatch(query, cmd.label);
        const descMatch = fuzzyMatch(query, cmd.description || "");
        const categoryMatch = fuzzyMatch(query, cmd.category);
        const keywordScores = (cmd.keywords || []).map((k) => fuzzyMatch(query, k));

        const bestKeyword = keywordScores.reduce(
          (best, s) => (s.score > best.score ? s : best),
          { match: false, score: 0 },
        );

        const matched = labelMatch.match || descMatch.match || categoryMatch.match || bestKeyword.match;
        const score = Math.max(labelMatch.score, descMatch.score * 0.7, categoryMatch.score * 0.5, bestKeyword.score * 0.8);

        return { cmd, matched, score };
      })
      .filter((s) => s.matched)
      .sort((a, b) => b.score - a.score);

    return scored.map((s) => s.cmd);
  }, [commands, query, mruUsage]);

  // Reset selection when query changes
  useEffect(() => {
    setSelectedIndex(0);
  }, [query]);

  // Focus input when opened, probe Ollama
  useEffect(() => {
    if (isOpen) {
      setQuery("");
      setSelectedIndex(0);
      clearIntent();
      // Probe Ollama availability (cached after first call)
      if (ollamaAvailable === null) {
        probeOllama();
      }
      // Small delay to ensure DOM is ready
      requestAnimationFrame(() => {
        inputRef.current?.focus();
      });
    }
  }, [isOpen, ollamaAvailable, probeOllama, clearIntent]);

  // Global shortcut
  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      // Cmd+K or Cmd+P to open
      if ((e.metaKey || e.ctrlKey) && (e.key === "k" || e.key === "p")) {
        e.preventDefault();
        if (isOpen) {
          onClose();
        } else {
          useFileManagerStore.getState().setCommandPaletteOpen(true);
        }
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [isOpen, onClose]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      switch (e.key) {
        case "ArrowDown":
          if (intentPhase === "idle") {
            e.preventDefault();
            setSelectedIndex((i) => Math.min(i + 1, filteredCommands.length - 1));
          }
          break;
        case "ArrowUp":
          if (intentPhase === "idle") {
            e.preventDefault();
            setSelectedIndex((i) => Math.max(i - 1, 0));
          }
          break;
        case "Enter": {
          e.preventDefault();
          if (intentPhase === "preview") {
            // Confirm the intent
            confirmIntent().then((result: AiExecutionResult | null) => {
              if (result) {
                onExecutionResult?.(result);
              }
              onClose();
            });
          } else if (intentPhase === "clarify") {
            // Re-submit with additional context
            const clarification = query.trim();
            if (clarification) {
              const original = currentIntentPreview
                ? `${JSON.stringify(currentIntentPreview)}. Additional: ${clarification}`
                : clarification;
              submitIntent(original);
              setQuery("");
            }
          } else if (intentPhase === "idle") {
            const q = query.trim();
            if (q && isNaturalLanguage(q, commands)) {
              // Natural language mode
              submitIntent(q);
            } else {
              // Standard command execution
              const cmd = filteredCommands[selectedIndex];
              if (cmd) {
                // Iter 24: record the usage so the next palette
                // open surfaces this command at the top. Fire
                // before `action()` so a long-running async
                // action doesn't block the write.
                recordCommandUsage(cmd.id);
                cmd.action();
                onClose();
              }
            }
          }
          break;
        }
        case "Escape":
          e.preventDefault();
          if (intentPhase !== "idle") {
            clearIntent();
            inputRef.current?.focus();
          } else {
            onClose();
          }
          break;
      }
    },
    [filteredCommands, selectedIndex, onClose, intentPhase, query, commands, currentIntentPreview, submitIntent, confirmIntent, clearIntent, onExecutionResult],
  );

  // Scroll selected item into view
  useEffect(() => {
    const item = listRef.current?.children[selectedIndex] as HTMLElement;
    if (item && typeof item.scrollIntoView === "function") {
      item.scrollIntoView({ block: "nearest" });
    }
  }, [selectedIndex]);

  if (!isOpen) return null;

  // Group commands by category
  const grouped = new Map<string, CommandItem[]>();
  for (const cmd of filteredCommands) {
    const existing = grouped.get(cmd.category) || [];
    existing.push(cmd);
    grouped.set(cmd.category, existing);
  }

  let flatIndex = 0;

  return (
    <>
      {/* Backdrop */}
      <div
        className="fixed inset-0 z-50 bg-black/40"
        onClick={onClose}
        aria-hidden="true"
        data-testid="command-palette-backdrop"
      />

      {/* Palette */}
      <div
        className={cn(
          "fixed z-50 top-[15%] left-1/2 -translate-x-1/2",
          "w-[90vw] max-w-[560px]",
          "bg-[var(--color-bg-elevated)] border border-[var(--color-border)]",
          "rounded-[var(--radius-lg)] shadow-xl",
          "animate-scale-in overflow-hidden",
        )}
        role="dialog"
        aria-label="Command palette"
        data-testid="command-palette"
      >
        {/* Search input */}
        <div className="flex items-center gap-2 px-4 border-b border-[var(--color-border)]">
          <Search className="h-4 w-4 text-[color:var(--color-text-tertiary)] shrink-0" aria-hidden="true" />
          <input
            ref={inputRef}
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={
              ollamaAvailable
                ? "Type a command or describe what you want..."
                : "Type a command..."
            }
            className={cn(
              "flex-1 h-12 bg-transparent border-none outline-none",
              "text-[length:var(--font-size-md)] text-[color:var(--color-text)]",
              "placeholder:text-[color:var(--color-text-tertiary)]",
            )}
            aria-label="Search commands"
            data-testid="command-palette-input"
          />
          <span className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]">
            ESC
          </span>
        </div>

        {/* Content area — phase-aware */}
        {intentPhase === "loading" ? (
          /* Loading phase */
          <div className="flex flex-col items-center justify-center py-12 gap-3">
            <Loader2 className="h-6 w-6 animate-spin text-[color:var(--color-accent)]" />
            <span className="text-[length:var(--font-size-sm)] text-[color:var(--color-text-secondary)]">
              Understanding your request...
            </span>
          </div>
        ) : intentPhase === "preview" && currentIntentPreview ? (
          /* Preview phase */
          <div>
            <div className="px-4 py-1 text-[length:var(--font-size-xs)] font-semibold text-[color:var(--color-text-tertiary)] uppercase tracking-wider flex items-center gap-1.5">
              <Sparkles className="h-3 w-3" />
              I&apos;ll do this
            </div>
            <IntentPreviewCard config={currentIntentPreview} />
          </div>
        ) : intentPhase === "clarify" && currentIntentPreview ? (
          /* Clarify phase */
          <div className="px-4 py-4 space-y-3">
            <div className="text-[length:var(--font-size-sm)] text-[color:var(--color-text)]">
              I need a bit more information:
            </div>
            {currentIntentPreview.clarification_questions.map((q: string, i: number) => (
              <div
                key={i}
                className="text-[length:var(--font-size-sm)] text-[color:var(--color-text-secondary)]"
              >
                {q}
              </div>
            ))}
            <div className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]">
              Type your answer above and press Enter
            </div>
          </div>
        ) : (
          /* Default: command list */
          <div
            ref={listRef}
            className="max-h-[60vh] overflow-auto py-2"
            role="listbox"
            aria-label="Commands"
          >
            {filteredCommands.length === 0 ? (
              <div className="px-4 py-8 text-center text-[length:var(--font-size-sm)] text-[color:var(--color-text-tertiary)]">
                No matching commands found
              </div>
            ) : (
              Array.from(grouped.entries()).map(([category, cmds]) => (
                <div key={category}>
                  <div className="px-4 py-1 text-[length:var(--font-size-xs)] font-semibold text-[color:var(--color-text-tertiary)] uppercase tracking-wider">
                    {category}
                  </div>
                  {cmds.map((cmd) => {
                    const currentIndex = flatIndex++;
                    const isSelected = currentIndex === selectedIndex;

                    return (
                      <button
                        key={cmd.id}
                        role="option"
                        aria-selected={isSelected}
                        className={cn(
                          "flex items-center gap-3 w-full px-4 py-2 text-left",
                          "transition-theme",
                          isSelected
                            ? "bg-[var(--color-selection-bg)] text-[color:var(--color-selection-text)]"
                            : "text-[color:var(--color-text)] hover:bg-[var(--color-hover-bg)]",
                        )}
                        onClick={() => {
                          // Iter 24: record usage for MRU ordering.
                          recordCommandUsage(cmd.id);
                          cmd.action();
                          onClose();
                        }}
                        onMouseEnter={() => setSelectedIndex(currentIndex)}
                        data-testid={`command-${cmd.id}`}
                      >
                        {cmd.icon && (
                          <span className="w-5 h-5 flex items-center justify-center shrink-0">
                            {cmd.icon}
                          </span>
                        )}
                        <div className="flex-1 min-w-0">
                          <span className="text-[length:var(--font-size-sm)] block truncate">
                            {cmd.label}
                          </span>
                          {cmd.description && (
                            <span className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-secondary)] block truncate">
                              {cmd.description}
                            </span>
                          )}
                        </div>
                        {cmd.shortcut && (
                          <span className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)] shrink-0">
                            {cmd.shortcut}
                          </span>
                        )}
                      </button>
                    );
                  })}
                </div>
              ))
            )}
          </div>
        )}

        {/* Footer — phase-aware */}
        <div className="flex items-center justify-between px-4 py-2 border-t border-[var(--color-border)] text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]">
          {intentPhase === "preview" ? (
            <>
              <span className="flex items-center gap-1">
                <Sparkles className="h-3 w-3" /> AI-powered
              </span>
              <div className="flex items-center gap-3">
                <span>Enter to confirm</span>
                <span>Esc to cancel</span>
              </div>
            </>
          ) : intentPhase === "loading" ? (
            <>
              <span>Analyzing...</span>
              <span>Esc to cancel</span>
            </>
          ) : intentPhase === "clarify" ? (
            <>
              <span>Type your answer</span>
              <div className="flex items-center gap-3">
                <span>Enter to submit</span>
                <span>Esc to cancel</span>
              </div>
            </>
          ) : (
            <>
              <span>
                {filteredCommands.length} command{filteredCommands.length !== 1 ? "s" : ""}
                {ollamaAvailable && (
                  <span className="ml-2 opacity-60">· or describe in plain language</span>
                )}
              </span>
              <div className="flex items-center gap-3">
                <span>Navigate</span>
                <span>Enter to run</span>
              </div>
            </>
          )}
        </div>
      </div>
    </>
  );
}

// ──────────────────────────────────────────────
// Default commands builder
// ──────────────────────────────────────────────

export function getDefaultCommands(actions: {
  onNewFolder?: () => void;
  onNewFile?: () => void;
  onCopy?: () => void;
  onCut?: () => void;
  onPaste?: () => void;
  onDelete?: () => void;
  onRename?: () => void;
  onDuplicate?: () => void;
  onUndo?: () => void;
  onRefresh?: () => void;
  onToggleDualPane?: () => void;
  onToggleSidebar?: () => void;
  onSetViewMode?: (mode: string) => void;
  onSetTheme?: (theme: string) => void;
  onAddToFavorites?: () => void;
  onTogglePreview?: () => void;
  onSelectAll?: () => void;
  onInvertSelection?: () => void;
  onSaveWorkspace?: () => void;
  onLoadWorkspace?: () => void;
  onSetPermissions?: () => void;
  onSetOwner?: () => void;
  onCreateSymlink?: () => void;
  onCopyAsScript?: () => void;
  onCopyPath?: () => void;
  onCopyRemotePath?: () => void;
  onCopyUrl?: () => void;
  onCopyRelativePath?: () => void;
  onOpenInEditor?: () => void;
  onOpenSettings?: () => void;
  onToggleAutomations?: () => void;
  onCreateAutomation?: () => void;
  onCreateSmartSpace?: () => void;
  onToggleActivityTimeline?: () => void;
  onReopenClosedTab?: () => void;
  onStashSelection?: () => void;
  onRecallLastStash?: () => void;
  /** Toggle a bookmark on the active pane's current directory. */
  onToggleBookmarkCurrentDir?: () => void;
  /** Live snapshot of the favorites ring so the palette can surface
   *  each bookmark as its own jump entry. Kept optional for callers
   *  that build the palette without the file-manager store wired in. */
  bookmarks?: Array<{ id: string; name: string; path: string }>;
  /** Jump the active pane to a bookmark by its slot index. */
  onJumpToBookmark?: (index: number) => void;
  /** Open the active pane's current path as a fresh tab in the OTHER
   *  pane and focus it. Auto-exits single-pane mode. */
  onOpenInOtherPane?: () => void;
  /** Swap the contents of the two panes \u2014 left becomes right and
   *  vice versa, taking selections, tabs, and per-pane history along
   *  with them. Auto-exits single-pane mode. */
  onSwapPanes?: () => void;
  /** Copy the active pane's filter text to the other pane so both
   *  sides filter for the same query. No-op when the source filter
   *  is empty. */
  onEchoFilterToOtherPane?: () => void;
  /** Copy the active pane's full view configuration (viewMode,
   *  sortBy, sortAsc, groupBy) to the other pane in one keystroke.
   *  Idempotent: silent no-op when both panes already match. */
  onMirrorViewToOtherPane?: () => void;
  /** Duplicate the active tab as a fresh tab in the SAME pane.
   *  The clone gets its own back/forward history starting from
   *  the current path. Pairs with Open in Other Pane (which
   *  targets the inactive pane) to give the user both same-pane
   *  and cross-pane "fork this view" gestures. */
  onDuplicateActiveTab?: () => void;
  /** Open the Batch Rename dialog on the active pane's current
   *  selection. Iter 25 surfaces the long-orphaned rename engine
   *  (token substitution, find/replace, regex, sequential
   *  numbering, case transformation, undo). No-op with a toast
   *  when the selection is empty. */
  onBatchRename?: () => void;
  /** Open the Connection Manager panel \u2014 saved profiles,
   *  quick-connect, connection test, third-party import, and
   *  the 17-protocol registry (SFTP, S3, Google Drive, WebDAV,
   *  SMB, FTP, Azure Blob, GCS, Dropbox, Box, OneDrive, iCloud
   *  Drive, Mega, pCloud, Nextcloud, OpenStack Swift, MinIO). */
  onOpenConnections?: () => void;
  /** Open the Sync Panel \u2014 sync-pair health indicators,
   *  create-pair wizard, dry-run preview, conflict resolution,
   *  reports, and rollback. Iter 28 surfaces the long-orphaned
   *  1454-LOC component. */
  onOpenSync?: () => void;
  /** Open the Transfer History (Journal) panel \u2014 searchable
   *  table of every historical transfer with status, checksum
   *  verification, retry count, duration, and speed; CSV/JSON
   *  export; retention cleanup. Iter 30 surfaces the long-
   *  orphaned 284-LOC component that complements the transfer
   *  engine's journal + crash-recovery layer. */
  onOpenTransferHistory?: () => void;
  /** Open the Integrity Tools panel — four tabs covering
   *  checksum computation (MD5 / SHA-1 / SHA-256), checksum
   *  verification, duplicate detection, tags and labels, and
   *  Smart Folder saved queries. Iter 36 surfaces the last
   *  clean 1071-LOC orphan (T-063) with five pre-registered
   *  backend IPCs. */
  onOpenIntegrityTools?: () => void;
  /** Open the Preview Pane on the active pane's first selected
   *  file. Supports images (EXIF), PDFs, syntax-highlighted
   *  code (30+ languages), rendered Markdown, audio waveforms,
   *  video thumbnails, archive contents, and plain metadata
   *  for large files. Sandboxed \u2014 no script / network / write.
   *  Iter 29 surfaces the long-orphaned 844-LOC component.
   *  Shows a structured-error toast when selection is empty. */
  onOpenPreview?: () => void;
  /** Copy every selected path (one per line) to the system
   *  clipboard, or the current directory path when nothing is
   *  selected. Different from the single-path `onCopyPath` entry,
   *  which silently drops every path but the first from a
   *  multi-file selection. */
  onCopyAllPaths?: () => void;
  /** Iter 42: "Jump to Last Touched" — teleport directly to the
   *  most recently touched ledger path without opening Instant
   *  Jump. Routes through the same shared dispatch helper as
   *  Instant Jump, so archives/text/media/folders all land on
   *  the right surface. No-op when the ledger is empty. */
  onJumpLastTouched?: () => void;
}): CommandItem[] {
  const isMac = typeof navigator !== "undefined" && navigator.platform.startsWith("Mac");
  const cmd = isMac ? "\u2318" : "Ctrl+";

  const commands: CommandItem[] = [
    // File operations
    {
      id: "new-folder",
      label: "New Folder",
      icon: <FolderPlus className="h-4 w-4" />,
      shortcut: `${cmd}Shift+N`,
      category: "File",
      action: actions.onNewFolder || (() => {}),
      keywords: ["create", "directory", "mkdir"],
    },
    {
      id: "new-file",
      label: "New File",
      icon: <FilePlus className="h-4 w-4" />,
      category: "File",
      action: actions.onNewFile || (() => {}),
      keywords: ["create", "touch"],
    },
    {
      id: "copy",
      label: "Copy",
      icon: <Copy className="h-4 w-4" />,
      shortcut: `${cmd}C`,
      category: "Edit",
      action: actions.onCopy || (() => {}),
    },
    {
      id: "cut",
      label: "Cut",
      icon: <Scissors className="h-4 w-4" />,
      shortcut: `${cmd}X`,
      category: "Edit",
      action: actions.onCut || (() => {}),
    },
    {
      id: "paste",
      label: "Paste",
      shortcut: `${cmd}V`,
      category: "Edit",
      action: actions.onPaste || (() => {}),
    },
    {
      id: "duplicate",
      label: "Duplicate",
      icon: <CopyPlus className="h-4 w-4" />,
      shortcut: `${cmd}D`,
      category: "Edit",
      action: actions.onDuplicate || (() => {}),
    },
    {
      id: "rename",
      label: "Rename",
      icon: <Pencil className="h-4 w-4" />,
      shortcut: "F2",
      category: "Edit",
      action: actions.onRename || (() => {}),
    },
    {
      id: "delete",
      label: "Move to Trash",
      icon: <Trash2 className="h-4 w-4" />,
      shortcut: `${cmd}Backspace`,
      category: "Edit",
      action: actions.onDelete || (() => {}),
      keywords: ["remove", "trash"],
    },
    {
      id: "undo",
      label: "Undo",
      icon: <Undo2 className="h-4 w-4" />,
      shortcut: `${cmd}Z`,
      category: "Edit",
      action: actions.onUndo || (() => {}),
    },

    // Selection
    {
      id: "select-all",
      label: "Select All",
      shortcut: `${cmd}A`,
      category: "Selection",
      action: actions.onSelectAll || (() => {}),
    },
    {
      id: "invert-selection",
      label: "Invert Selection",
      category: "Selection",
      action: actions.onInvertSelection || (() => {}),
    },

    // View
    {
      id: "view-list",
      label: "List View",
      icon: <List className="h-4 w-4" />,
      category: "View",
      action: () => actions.onSetViewMode?.("list"),
      keywords: ["list", "view"],
    },
    {
      id: "view-detail",
      label: "Detail View",
      icon: <AlignJustify className="h-4 w-4" />,
      category: "View",
      action: () => actions.onSetViewMode?.("detail"),
      keywords: ["detail", "columns", "view"],
    },
    {
      id: "view-grid",
      label: "Grid View",
      icon: <Grid3X3 className="h-4 w-4" />,
      category: "View",
      action: () => actions.onSetViewMode?.("grid"),
      keywords: ["grid", "icons", "view"],
    },
    {
      id: "view-compact",
      label: "Compact View",
      icon: <Layout className="h-4 w-4" />,
      category: "View",
      action: () => actions.onSetViewMode?.("compact"),
      keywords: ["compact", "dense", "view"],
    },
    {
      id: "toggle-dual-pane",
      label: "Toggle Dual Pane",
      icon: <Columns2 className="h-4 w-4" />,
      shortcut: `${cmd}Shift+D`,
      category: "View",
      action: actions.onToggleDualPane || (() => {}),
      keywords: ["split", "dual", "pane"],
    },
    {
      id: "toggle-sidebar",
      label: "Toggle Sidebar",
      icon: <PanelLeftClose className="h-4 w-4" />,
      shortcut: `${cmd}B`,
      category: "View",
      action: actions.onToggleSidebar || (() => {}),
    },
    {
      id: "toggle-preview",
      label: "Toggle Preview Panel",
      icon: <Eye className="h-4 w-4" />,
      category: "View",
      action: actions.onTogglePreview || (() => {}),
      keywords: ["preview", "panel"],
    },
    {
      id: "refresh",
      label: "Refresh",
      icon: <RefreshCw className="h-4 w-4" />,
      shortcut: `${cmd}R`,
      category: "View",
      action: actions.onRefresh || (() => {}),
      keywords: ["reload"],
    },

    // Bookmarks
    {
      id: "add-favorite",
      label: "Add to Favorites",
      icon: <Star className="h-4 w-4" />,
      category: "Navigation",
      action: actions.onAddToFavorites || (() => {}),
      keywords: ["bookmark", "favorite", "star"],
    },

    // Theme
    {
      id: "theme-light",
      label: "Light Theme",
      icon: <Sun className="h-4 w-4" />,
      category: "Appearance",
      action: () => actions.onSetTheme?.("light"),
      keywords: ["theme", "light"],
    },
    {
      id: "theme-dark",
      label: "Dark Theme",
      icon: <Moon className="h-4 w-4" />,
      category: "Appearance",
      action: () => actions.onSetTheme?.("dark"),
      keywords: ["theme", "dark"],
    },
    {
      id: "theme-system",
      label: "System Theme",
      icon: <Settings className="h-4 w-4" />,
      category: "Appearance",
      action: () => actions.onSetTheme?.("system"),
      keywords: ["theme", "system", "auto"],
    },
    // Workspace commands
    {
      id: "save-workspace",
      label: "Save Workspace",
      shortcut: `${cmd}Shift+S`,
      category: "File",
      action: actions.onSaveWorkspace || (() => {}),
      keywords: ["workspace", "save", "layout", "tabs"],
    },
    {
      id: "load-workspace",
      label: "Load Workspace",
      category: "File",
      action: actions.onLoadWorkspace || (() => {}),
      keywords: ["workspace", "restore", "open", "layout"],
    },
    // Permission commands
    {
      id: "set-permissions",
      label: "Set Permissions (chmod)",
      category: "File",
      action: actions.onSetPermissions || (() => {}),
      keywords: ["chmod", "permissions", "mode", "755", "644"],
    },
    {
      id: "set-owner",
      label: "Set Owner (chown)",
      category: "File",
      action: actions.onSetOwner || (() => {}),
      keywords: ["chown", "owner", "group", "uid", "gid"],
    },
    {
      id: "create-symlink",
      label: "Create Symbolic Link",
      category: "File",
      action: actions.onCreateSymlink || (() => {}),
      keywords: ["symlink", "link", "symbolic"],
    },
    // Script generation
    {
      id: "copy-as-script",
      label: "Copy as Script",
      category: "Edit",
      action: actions.onCopyAsScript || (() => {}),
      keywords: ["script", "bash", "shell", "generate", "automation"],
    },
    // Copy Path commands (Transmit parity)
    {
      id: "copy-path",
      label: "Copy Path",
      icon: <Clipboard className="h-4 w-4" />,
      shortcut: `${cmd}Shift+C`,
      category: "Copy Path",
      action: actions.onCopyPath || (() => {}),
      keywords: ["path", "copy", "absolute", "filepath"],
    },
    {
      id: "copy-remote-path",
      label: "Copy Remote Path",
      icon: <Clipboard className="h-4 w-4" />,
      category: "Copy Path",
      action: actions.onCopyRemotePath || (() => {}),
      keywords: ["path", "copy", "remote", "server"],
    },
    {
      id: "copy-url",
      label: "Copy URL",
      icon: <Globe className="h-4 w-4" />,
      category: "Copy Path",
      action: actions.onCopyUrl || (() => {}),
      keywords: ["url", "copy", "link", "protocol"],
    },
    {
      id: "copy-relative-path",
      label: "Copy Relative Path",
      icon: <Clipboard className="h-4 w-4" />,
      category: "Copy Path",
      action: actions.onCopyRelativePath || (() => {}),
      keywords: ["path", "copy", "relative"],
    },
    // Editor
    {
      id: "open-in-editor",
      label: "Open in External Editor",
      icon: <FileEdit className="h-4 w-4" />,
      category: "File",
      action: actions.onOpenInEditor || (() => {}),
      keywords: ["editor", "external", "vscode", "open", "edit"],
    },
    // Preview \u2014 iter 29 surfaced the 844-LOC orphaned
    // PreviewPane. Images (with EXIF), PDFs, syntax-highlighted
    // code (30+ languages), rendered Markdown, audio waveforms,
    // video thumbnails, archive contents, plain metadata for
    // huge files. All sandboxed. 4 IPC commands (preview_file,
    // preview_get_exif, set_permissions, set_owner) were
    // pre-registered in the Rust backend; the component just
    // had zero importers until iter 29.
    {
      id: "open-preview",
      label: "Preview File\u2026",
      description:
        "Open a sandboxed preview (images, PDF, code, Markdown, audio, video, archive, EXIF) for the selected file",
      icon: <Eye className="h-4 w-4" />,
      shortcut: `${cmd}\u21E7P`,
      category: "File Operations",
      action: actions.onOpenPreview || (() => {}),
      keywords: [
        "preview",
        "peek",
        "quick look",
        "view",
        "show",
        "inspect",
        "file preview",
        "pdf",
        "image",
        "exif",
        "markdown",
        "syntax",
        "code",
        "thumbnail",
        "metadata",
      ],
    },
    // Sync Panel \u2014 iter 28 surfaced the 1454-LOC orphaned
    // sync-pair manager. Health indicators, create-pair
    // wizard, dry-run preview, conflict resolution, reports,
    // and rollback. All 14 sync IPC commands were already
    // registered in the Rust backend; the component just had
    // zero importers until iter 28.
    {
      id: "open-sync",
      label: "Sync Pairs\u2026",
      description:
        "Manage sync pairs \u2014 health indicators, dry-run preview, conflict resolution, reports, rollback",
      icon: <FolderSync className="h-4 w-4" />,
      shortcut: `${cmd}\u21E7H`,
      category: "Navigation",
      action: actions.onOpenSync || (() => {}),
      keywords: [
        "sync",
        "sync pair",
        "sync pairs",
        "mirror",
        "replicate",
        "dry run",
        "preview",
        "conflict",
        "rollback",
        "health",
        "scheduled",
        "one-way",
        "two-way",
        "backup",
      ],
    },
    // Transfer History (Journal) \u2014 iter 30 surfaced the 284-LOC
    // `TransferHistoryPanel` orphan that complements the transfer
    // engine's journal + crash-recovery layer. Shows every
    // historical transfer with status, checksum verification,
    // retry count, duration, speed; supports CSV/JSON export and
    // retention cleanup. Three backend IPCs (search_transfer_history,
    // export_transfer_history, cleanup_transfer_history) were
    // already registered; the component just had zero importers
    // until iter 30. Cmd+Shift+J mnemonic = "Journal" \u2014 the only
    // still-free letter that carried semantic meaning after the
    // iter 12\u201329 accumulation.
    {
      id: "open-transfer-history",
      label: "Transfer History\u2026",
      description:
        "Searchable journal of every transfer \u2014 status, checksum, retries, speed, duration; CSV/JSON export",
      icon: <History className="h-4 w-4" />,
      shortcut: `${cmd}\u21E7J`,
      category: "Navigation",
      action: actions.onOpenTransferHistory || (() => {}),
      keywords: [
        "transfer",
        "transfers",
        "history",
        "journal",
        "log",
        "completed",
        "failed",
        "retry",
        "retries",
        "checksum",
        "verified",
        "speed",
        "bandwidth",
        "throughput",
        "duration",
        "export",
        "csv",
        "json",
        "audit",
        "report",
        "past",
        "old",
      ],
    },
    // Integrity Tools — iter 36 surfaced the last clean 1071-LOC
    // orphan (T-063). Four tabs covering MD5/SHA-1/SHA-256
    // checksums, duplicate detection, tags/labels, and Smart
    // Folder saved queries. Five pre-registered backend IPCs
    // (integrity_checksum, integrity_verify, compute_checksum,
    // integrity_find_duplicates, integrity_get_file_info).
    // Cmd+Shift+I mnemonic = "Integrity".
    {
      id: "open-integrity-tools",
      label: "Integrity Tools…",
      description:
        "Checksums (MD5/SHA-1/SHA-256), duplicate detection, tags and labels, Smart Folder saved queries",
      icon: <ShieldCheck className="h-4 w-4" />,
      shortcut: `${cmd}\u21E7I`,
      category: "Navigation",
      action: actions.onOpenIntegrityTools || (() => {}),
      keywords: [
        "integrity",
        "checksum",
        "hash",
        "md5",
        "sha1",
        "sha256",
        "verify",
        "duplicate",
        "duplicates",
        "dedupe",
        "tag",
        "tags",
        "label",
        "labels",
        "smart folder",
        "smart folders",
        "saved query",
        "filter",
      ],
    },
    // Iter 42/43/45: Jump to Last Touched. One-keystroke teleport
    // to a recently touched ledger path. Complements Instant Jump
    // (⌘⇧O) — that one opens a picker, this one is zero-UI: it
    // fetches top `ledger_recent_paths` rows and routes through
    // the shared dispatch helper so archives land in the browser,
    // text in the editor, media in the preview pane, folders in
    // the active tab. Iter 43 upgraded the handler to a "Jump
    // Ring" (rapid re-press cycles forward, the current tab path
    // is always filtered out). Iter 45 added reverse cycling on
    // ⌘⇧⌥L and extracted the decision logic into a pure,
    // unit-tested `pickJumpRingTarget` function. "L" mnemonic = "Last".
    {
      id: "jump-last-touched",
      label: "Jump to Last Touched",
      description:
        "Teleport to a recently touched ledger path — \u2318\u21E7L cycles forward, \u2318\u21E7\u2325L cycles back, the current folder is always skipped",
      icon: <History className="h-4 w-4" />,
      shortcut: `${cmd}\u21E7L`,
      // Iter 48: surface the reverse cycling gesture in the
      // keyboard cheat sheet (?). Secondary shortcuts don't
      // show up in ⌘K itself — the palette stays one row per
      // action — but they DO appear in the cheat sheet so
      // power users discover the Alt modifier convention.
      additionalShortcuts: [
        {
          keys: `${cmd}\u21E7\u2325L`,
          hint: "Cycle back through the ring",
        },
      ],
      category: "Navigation",
      action: actions.onJumpLastTouched || (() => {}),
      keywords: [
        "jump",
        "last",
        "recent",
        "most recent",
        "teleport",
        "back",
        "where",
        "left off",
        "resume",
        "return",
        "previous",
        "ledger",
        "history",
        "ring",
        "cycle",
        "alt tab",
      ],
    },
    // Connection Manager \u2014 iter 27 surfaced the 1941-LOC
    // `ConnectionPanel` orphan that manages profiles across the
    // 17 connector protocols. All 17 Tauri commands it uses
    // were already registered in the Rust backend; the
    // component just had zero importers until iter 27.
    {
      id: "open-connections",
      label: "Connection Manager\u2026",
      description:
        "Manage saved connections (SFTP, S3, Google Drive, WebDAV, SMB, FTP, Azure, GCS, Dropbox, \u2026)",
      icon: <Globe className="h-4 w-4" />,
      shortcut: `${cmd}\u21E7N`,
      category: "Navigation",
      action: actions.onOpenConnections || (() => {}),
      keywords: [
        "connection",
        "connections",
        "connect",
        "remote",
        "server",
        "sftp",
        "s3",
        "google drive",
        "webdav",
        "smb",
        "ftp",
        "azure",
        "gcs",
        "dropbox",
        "protocol",
        "manager",
        "new connection",
      ],
    },
    // Settings \u2014 iter 26 wired the long-orphaned `SettingsPanel`
    // (1354 LOC with 9+ backend commands already registered) to
    // this entry. The palette stub existed from a prior iter but
    // `actions.onOpenSettings` was never passed, so the entry did
    // nothing until now. Iter 26 enriches the entry with the
    // universal Cmd+, shortcut + richer keywords.
    {
      id: "open-settings",
      label: "Settings\u2026",
      description:
        "Export/import, sync across installations, notification prefs, privacy, logs",
      icon: <Settings className="h-4 w-4" />,
      shortcut: `${cmd},`,
      category: "Navigation",
      action: actions.onOpenSettings || (() => {}),
      keywords: [
        "settings",
        "preferences",
        "options",
        "config",
        "configure",
        "export",
        "import",
        "notifications",
        "privacy",
        "logs",
      ],
    },
    // Automations (Quickflows)
    {
      id: "toggle-automations",
      label: "Toggle Quickflows Panel",
      icon: <Zap className="h-4 w-4" />,
      category: "Navigation",
      action: actions.onToggleAutomations || (() => {}),
      keywords: ["automation", "quickflow", "rules", "workflow", "watcher"],
    },
    {
      id: "create-automation",
      label: "Create Automation Rule",
      icon: <Zap className="h-4 w-4" />,
      category: "File",
      action: actions.onCreateAutomation || (() => {}),
      keywords: ["automation", "quickflow", "rule", "new", "create", "watcher", "trigger"],
    },
    // Smart Spaces
    {
      id: "create-smart-space",
      label: "Create Smart Space...",
      icon: <Layers className="h-4 w-4" />,
      category: "File",
      action: actions.onCreateSmartSpace || (() => {}),
      keywords: ["space", "workspace", "folder", "connection", "sync", "bundle"],
    },
    // Selection Stash — capture the active pane's selection into the
    // global persistent stash ring. Pairs with "Recall Last Stash" so
    // the user can navigate away, do something else, and come back to
    // the same working set without rebuilding it manually.
    {
      id: "stash-selection",
      label: "Stash Selection",
      description: "Save the current selection so you can recall it later",
      icon: <Bookmark className="h-4 w-4" />,
      shortcut: `${cmd}\u21E7M`,
      category: "Selection",
      action: actions.onStashSelection || (() => {}),
      keywords: [
        "stash",
        "save",
        "selection",
        "mark",
        "remember",
        "bookmark selection",
        "memory",
        "set aside",
      ],
    },
    {
      id: "recall-last-stash",
      label: "Recall Last Stash",
      description:
        "Restore the most recent stashed selection into the active pane",
      icon: <Bookmark className="h-4 w-4" />,
      category: "Selection",
      action: actions.onRecallLastStash || (() => {}),
      keywords: [
        "recall",
        "restore",
        "stash",
        "last",
        "selection",
        "bring back",
        "resume",
      ],
    },
    // Reopen the most recently closed tab in the active pane —
    // browser-muscle-memory shortcut. Fully no-op when the ring buffer
    // is empty, so the entry is always safe to surface.
    {
      id: "reopen-closed-tab",
      label: "Reopen Closed Tab",
      description: "Revive the most recently closed tab in the active pane",
      icon: <RotateCcw className="h-4 w-4" />,
      shortcut: `${cmd}\u21E7T`,
      category: "Navigation",
      action: actions.onReopenClosedTab || (() => {}),
      keywords: [
        "reopen",
        "restore",
        "tab",
        "closed",
        "undo close",
        "bring back",
      ],
    },
    // Activity Timeline (unified ledger viewer)
    {
      id: "toggle-activity-timeline",
      label: "Toggle Activity Timeline",
      description: "View recent activity across all engines",
      icon: <Activity className="h-4 w-4" />,
      category: "Navigation",
      shortcut: `${cmd}\u21E7Y`,
      action: actions.onToggleActivityTimeline || (() => {}),
      keywords: [
        "activity",
        "timeline",
        "history",
        "audit",
        "ledger",
        "recent",
        "events",
        "what happened",
      ],
    },
    // Open in Other Pane — opens the active pane's current path as a
    // fresh tab in the OTHER pane and focuses it. Auto-exits single-pane
    // mode (revealing the second pane is part of the gesture).
    {
      id: "open-in-other-pane",
      label: "Open in Other Pane",
      description:
        "Mirror the current location into the other pane as a new tab",
      icon: <Columns2 className="h-4 w-4" />,
      shortcut: `${cmd}\u21E7E`,
      category: "Navigation",
      action: actions.onOpenInOtherPane || (() => {}),
      keywords: [
        "pane",
        "split",
        "side by side",
        "mirror",
        "echo",
        "twin",
        "compare",
        "open here",
        "dual",
      ],
    },
    // Copy All Paths — copies every selected path to the clipboard
    // (one per line), or the current directory path when nothing
    // is selected. The existing "Copy Path" entry silently drops
    // everything but the first path, so this entry fills the
    // multi-select gap and also provides a keyboard shortcut.
    {
      id: "copy-all-paths",
      label: "Copy Selected Paths",
      description:
        "Copy every selected path to the clipboard \u2014 or the current directory when nothing is selected",
      icon: <Clipboard className="h-4 w-4" />,
      shortcut: `${cmd}\u21E7C`,
      category: "File Operations",
      action: actions.onCopyAllPaths || (() => {}),
      keywords: [
        "copy path",
        "copy paths",
        "clipboard",
        "multi copy",
        "paste",
        "absolute",
        "all paths",
        "selection paths",
        "export paths",
      ],
    },
    // Batch Rename \u2014 opens the rename engine on the active pane's
    // current selection. Iter 25 surfaced this long-orphaned
    // feature: token substitution ({name}, {ext}, {num}, {date},
    // {parent}, {counter}), find/replace with optional regex,
    // sequential numbering, case transformation, live preview,
    // undo, and compatibility-warning integration \u2014 all of it
    // was already built and tested in Rust + React, just never
    // wired into the UI.
    {
      id: "batch-rename",
      label: "Batch Rename\u2026",
      description:
        "Open the batch rename engine on the selected files (patterns, regex, numbering, case)",
      icon: <FileEdit className="h-4 w-4" />,
      shortcut: `${cmd}\u21E7R`,
      category: "File Operations",
      action: actions.onBatchRename || (() => {}),
      keywords: [
        "batch rename",
        "bulk rename",
        "rename",
        "regex rename",
        "find replace",
        "pattern rename",
        "case",
        "numbering",
        "multi rename",
        "mass rename",
      ],
    },
    // Duplicate Tab \u2014 clones the active tab as a fresh tab in the
    // SAME pane and focuses it. Different from Open in Other Pane
    // (which targets the inactive pane). The clone gets its own
    // back/forward history so navigating it does not disturb the
    // original.
    {
      id: "duplicate-active-tab",
      label: "Duplicate Tab",
      description:
        "Clone the active tab as a fresh tab in the same pane with its own history",
      icon: <CopyPlus className="h-4 w-4" />,
      shortcut: `${cmd}\u21E7K`,
      category: "Navigation",
      action: actions.onDuplicateActiveTab || (() => {}),
      keywords: [
        "duplicate",
        "clone",
        "fork",
        "tab",
        "copy tab",
        "split tab",
        "twin tab",
        "new tab here",
        "open here",
      ],
    },
    // Mirror View to Other Pane — copies the active pane's view
    // configuration (viewMode, sortBy, sortAsc, groupBy) to the
    // other pane in one keystroke. Replaces 3-4 menu clicks needed
    // to align two panes for side-by-side comparison. Idempotent
    // (silent no-op when both panes already match).
    {
      id: "mirror-view-to-other-pane",
      label: "Mirror View to Other Pane",
      description:
        "Copy view mode, sort column, sort direction, and grouping to the other pane",
      icon: <Layout className="h-4 w-4" />,
      shortcut: `${cmd}\u21E7V`,
      category: "Navigation",
      action: actions.onMirrorViewToOtherPane || (() => {}),
      keywords: [
        "view",
        "mirror view",
        "equalize",
        "align",
        "sort echo",
        "match view",
        "sync view",
        "compare layout",
        "twin view",
        "same sort",
      ],
    },
    // Echo Filter to Other Pane — copies the active pane's filter
    // text to the inactive pane so both sides filter for the same
    // query. No-op when the source filter is empty (silent: avoids
    // wiping the other pane's filter by surprise).
    {
      id: "echo-filter-to-other-pane",
      label: "Echo Filter to Other Pane",
      description:
        "Copy the active pane's filter text to the other pane so both sides filter for the same query",
      icon: <Filter className="h-4 w-4" />,
      shortcut: `${cmd}\u21E7F`,
      category: "Navigation",
      action: actions.onEchoFilterToOtherPane || (() => {}),
      keywords: [
        "filter",
        "echo",
        "mirror filter",
        "same query",
        "both panes",
        "sync filter",
        "copy filter",
        "twin filter",
        "apply filter",
      ],
    },
    // Swap Panes — flips the two panes' contents, taking tabs,
    // selections, and per-pane history with them. Auto-exits single-
    // pane mode. Pairs with Open in Other Pane: mirror, then swap if
    // the source/target sides are wrong.
    {
      id: "swap-panes",
      label: "Swap Panes",
      description:
        "Flip the left and right panes \u2014 contents, tabs, selections, and history all travel together",
      icon: <ArrowLeftRight className="h-4 w-4" />,
      shortcut: `${cmd}\u21E7X`,
      category: "Navigation",
      action: actions.onSwapPanes || (() => {}),
      keywords: [
        "swap",
        "flip",
        "exchange",
        "switch",
        "sides",
        "left right",
        "rotate",
        "trade",
        "panes",
        "reverse",
      ],
    },
    // Bookmark Current Folder — toggles the active pane's directory in
    // the same `favorites` ring the sidebar drag-drop populates. Single
    // entry, two outcomes (add / remove): the action itself decides
    // based on whether the path is already bookmarked.
    {
      id: "bookmark-current-folder",
      label: "Bookmark Current Folder",
      description:
        "Add (or remove) the active pane's directory from your bookmarks",
      icon: <BookmarkPlus className="h-4 w-4" />,
      shortcut: `${cmd}\u21E7B`,
      category: "Navigation",
      action: actions.onToggleBookmarkCurrentDir || (() => {}),
      keywords: [
        "bookmark",
        "favorite",
        "star",
        "save",
        "pin",
        "remember",
        "add to favorites",
      ],
    },
  ];

  // Inject the first nine bookmarks as their own palette entries so the
  // user can fuzzy-search them by name without remembering slot numbers.
  // Slot 1..9 maps directly to ⌘1..⌘9 muscle memory.
  const bookmarks = actions.bookmarks ?? [];
  const onJump = actions.onJumpToBookmark;
  for (let i = 0; i < Math.min(bookmarks.length, 9); i += 1) {
    const fav = bookmarks[i];
    commands.push({
      id: `jump-bookmark-${fav.id}`,
      label: `Jump to ${fav.name}`,
      description: fav.path,
      icon: <Bookmark className="h-4 w-4" />,
      shortcut: `${cmd}${i + 1}`,
      category: "Bookmarks",
      action: onJump ? () => onJump(i) : () => {},
      keywords: ["bookmark", "favorite", "jump", "navigate", fav.name],
    });
  }

  return commands;
}
