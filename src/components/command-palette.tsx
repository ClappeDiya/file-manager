import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useFileManagerStore } from "@/stores/file-manager-store";
import { useAiStore, type AiExecutionResult, type ParsedJobConfig } from "@/stores/ai-store";
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
  Loader2,
  Sparkles,
  ArrowRight,
  Shield,
  Filter,
  FolderSync,
  Upload,
} from "lucide-react";

// ──────────────────────────────────────────────
// Types
// ──────────────────────────────────────────────

export interface CommandItem {
  id: string;
  label: string;
  description?: string;
  icon?: React.ReactNode;
  shortcut?: string;
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

  // Filter and sort by fuzzy match score
  const filteredCommands = useMemo(() => {
    if (!query.trim()) return commands;

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
  }, [commands, query]);

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
    // Settings
    {
      id: "open-settings",
      label: "Open Settings",
      icon: <Settings className="h-4 w-4" />,
      category: "Navigation",
      action: actions.onOpenSettings || (() => {}),
      keywords: ["settings", "preferences", "config"],
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
  ];

  return commands;
}
