/**
 * Terminal Panel (T-046)
 *
 * Built-in terminal with xterm.js frontend:
 * - Local shell (bash/zsh macOS/Linux, PowerShell Windows)
 * - Remote SSH terminal using saved SFTP/SSH credentials
 * - Split terminal (horizontal/vertical)
 * - Drag file from browser pastes escaped path
 * - Terminal sessions persist across tab switches
 *
 * xterm.js renders each session; the Rust PTY backend streams output via
 * `terminal-output` events and receives input through `terminal_write`.
 */
import { useState, useCallback, useRef, useEffect } from "react";
import { useTerminalStore, type TerminalSession } from "@/stores/terminal-store";
import { XtermView } from "./xterm-view";
import { TerminalHelpDrawer } from "./terminal-help-drawer";
import { promptText } from "@/lib/prompt-dialog";
import { cn } from "@ufop/ui-components";
import {
  Terminal as TerminalIcon,
  Plus,
  X,
  SplitSquareHorizontal,
  SplitSquareVertical,
  Minimize2,
  Globe,
  Monitor,
  ChevronUp,
  GripHorizontal,
  CircleHelp,
} from "lucide-react";

// ──────────────────────────────────────────────
// Terminal Panel - Main Component
// ──────────────────────────────────────────────

interface TerminalPanelProps {
  className?: string;
}

export function TerminalPanel({ className }: TerminalPanelProps) {
  const panelOpen = useTerminalStore((s) => s.panelOpen);
  const panelHeight = useTerminalStore((s) => s.panelHeight);
  const setPanelHeight = useTerminalStore((s) => s.setPanelHeight);
  const togglePanel = useTerminalStore((s) => s.togglePanel);
  const sessions = useTerminalStore((s) => s.sessions);
  const activeSessionId = useTerminalStore((s) => s.activeSessionId);
  const setActiveSession = useTerminalStore((s) => s.setActiveSession);
  const createLocalSession = useTerminalStore((s) => s.createLocalSession);
  const createRemoteSession = useTerminalStore((s) => s.createRemoteSession);
  const closeSession = useTerminalStore((s) => s.closeSession);
  const layout = useTerminalStore((s) => s.layout);
  const splitTerminal = useTerminalStore((s) => s.splitTerminal);

  const [isDragging, setIsDragging] = useState(false);
  const [showNewMenu, setShowNewMenu] = useState(false);
  const [helpOpen, setHelpOpen] = useState(false);
  const panelRef = useRef<HTMLDivElement>(null);

  // Handle resize drag
  const handleResizeStart = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      setIsDragging(true);
      const startY = e.clientY;
      const startHeight = panelHeight;

      const onMouseMove = (ev: MouseEvent) => {
        const delta = startY - ev.clientY;
        setPanelHeight(startHeight + delta);
      };

      const onMouseUp = () => {
        setIsDragging(false);
        window.removeEventListener("mousemove", onMouseMove);
        window.removeEventListener("mouseup", onMouseUp);
      };

      window.addEventListener("mousemove", onMouseMove);
      window.addEventListener("mouseup", onMouseUp);
    },
    [panelHeight, setPanelHeight],
  );

  // Handle file drop to paste path
  const handleDrop = useCallback(
    async (e: React.DragEvent) => {
      e.preventDefault();
      const path = e.dataTransfer.getData("text/path");
      if (path && activeSessionId) {
        const { escapePath, writeToSession } = useTerminalStore.getState();
        const escaped = await escapePath(path);
        await writeToSession(activeSessionId, escaped);
      }
    },
    [activeSessionId],
  );

  const handleDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.dataTransfer.dropEffect = "copy";
  }, []);

  // Command help: a click types the command at the prompt, without running it.
  const insertCommand = useCallback(
    (text: string) => {
      const id = activeSessionId ?? sessions[0]?.id;
      if (id) void useTerminalStore.getState().writeToSession(id, text);
    },
    [activeSessionId, sessions],
  );

  // Handle split
  const handleSplit = useCallback(
    async (orientation: "horizontal" | "vertical") => {
      if (sessions.length < 2) {
        // Create a second session first
        const newSession = await createLocalSession();
        if (activeSessionId) {
          await splitTerminal([activeSessionId, newSession.id], orientation);
        }
      } else if (activeSessionId) {
        const otherSession = sessions.find((s) => s.id !== activeSessionId);
        if (otherSession) {
          await splitTerminal([activeSessionId, otherSession.id], orientation);
        }
      }
    },
    [sessions, activeSessionId, createLocalSession, splitTerminal],
  );

  if (!panelOpen) {
    return (
      <TerminalToggleBar onToggle={togglePanel} sessionCount={sessions.length} />
    );
  }

  return (
    <div
      ref={panelRef}
      className={cn(
        "flex flex-col border-t border-[var(--color-border)]",
        "bg-[var(--color-bg)]",
        isDragging && "select-none",
        className,
      )}
      style={{ height: panelHeight }}
      onDrop={handleDrop}
      onDragOver={handleDragOver}
      data-testid="terminal-panel"
      role="region"
      aria-label="Terminal"
    >
      {/* Resize handle */}
      <div
        className={cn(
          "h-1 w-full cursor-row-resize flex items-center justify-center",
          "bg-[var(--color-border)] hover:bg-[var(--color-primary)]",
          "transition-theme",
          isDragging && "bg-[var(--color-primary)]",
        )}
        onMouseDown={handleResizeStart}
        role="separator"
        aria-orientation="horizontal"
        aria-label="Resize terminal panel"
      >
        <GripHorizontal className="h-3 w-3 text-[color:var(--color-text-tertiary)]" />
      </div>

      {/* Terminal toolbar */}
      <div className="flex items-center justify-between px-2 py-1 border-b border-[var(--color-border)] bg-[var(--color-bg-secondary)]">
        <div className="flex items-center gap-1 overflow-x-auto">
          {/* Session tabs */}
          {sessions.map((session) => (
            <SessionTab
              key={session.id}
              session={session}
              active={session.id === activeSessionId}
              onClick={() => setActiveSession(session.id)}
              onClose={() => closeSession(session.id)}
            />
          ))}

          {/* New terminal button */}
          <div className="relative">
            <button
              className={cn(
                "h-6 w-6 flex items-center justify-center rounded-sm",
                "text-[color:var(--color-text-tertiary)] hover:text-[color:var(--color-text)]",
                "hover:bg-[var(--color-hover-bg)] transition-theme",
              )}
              onClick={() => setShowNewMenu(!showNewMenu)}
              aria-label="New terminal"
              title="New terminal"
              data-testid="new-terminal-btn"
            >
              <Plus className="h-3.5 w-3.5" />
            </button>

            {showNewMenu && (
              <NewTerminalMenu
                onCreateLocal={() => {
                  createLocalSession();
                  setShowNewMenu(false);
                }}
                onCreateRemote={async () => {
                  setShowNewMenu(false);
                  const target = await promptText("SSH target (user@host or user@host:port):", "");
                  const m = target?.trim().match(/^([^@\s]+)@([^:\s]+)(?::(\d{1,5}))?$/);
                  if (!m) return;
                  try {
                    await createRemoteSession({ host: m[2], port: m[3] ? Number(m[3]) : 22, username: m[1] });
                  } catch { /* surfaced through the store's error state */ }
                }}
                onClose={() => setShowNewMenu(false)}
              />
            )}
          </div>
        </div>

        {/* Toolbar actions */}
        <div className="flex items-center gap-1">
          <button
            className={cn(
              "h-6 w-6 flex items-center justify-center rounded-sm",
              "text-[color:var(--color-text-tertiary)] hover:text-[color:var(--color-text)]",
              "hover:bg-[var(--color-hover-bg)] transition-theme",
              helpOpen && "bg-[var(--color-hover-bg)] text-[color:var(--color-text)]",
            )}
            onClick={() => setHelpOpen((o) => !o)}
            aria-label="Command help"
            aria-pressed={helpOpen}
            title="Command help: ufop and rclone commands"
            data-testid="terminal-help-btn"
          >
            <CircleHelp className="h-3.5 w-3.5" />
          </button>
          <button
            className={cn(
              "h-6 w-6 flex items-center justify-center rounded-sm",
              "text-[color:var(--color-text-tertiary)] hover:text-[color:var(--color-text)]",
              "hover:bg-[var(--color-hover-bg)] transition-theme",
            )}
            onClick={() => handleSplit("horizontal")}
            aria-label="Split terminal horizontally"
            title="Split horizontal"
            data-testid="split-horizontal-btn"
          >
            <SplitSquareHorizontal className="h-3.5 w-3.5" />
          </button>
          <button
            className={cn(
              "h-6 w-6 flex items-center justify-center rounded-sm",
              "text-[color:var(--color-text-tertiary)] hover:text-[color:var(--color-text)]",
              "hover:bg-[var(--color-hover-bg)] transition-theme",
            )}
            onClick={() => handleSplit("vertical")}
            aria-label="Split terminal vertically"
            title="Split vertical"
            data-testid="split-vertical-btn"
          >
            <SplitSquareVertical className="h-3.5 w-3.5" />
          </button>
          <button
            className={cn(
              "h-6 w-6 flex items-center justify-center rounded-sm",
              "text-[color:var(--color-text-tertiary)] hover:text-[color:var(--color-text)]",
              "hover:bg-[var(--color-hover-bg)] transition-theme",
            )}
            onClick={togglePanel}
            aria-label="Minimize terminal"
            title="Minimize"
          >
            <Minimize2 className="h-3.5 w-3.5" />
          </button>
        </div>
      </div>

      {/* Terminal content area */}
      <div className="flex-1 overflow-hidden flex">
        <div className="flex-1 min-w-0 overflow-hidden">
          {sessions.length === 0 ? (
            <EmptyTerminal onCreateLocal={createLocalSession} />
          ) : layout && layout.session_ids.length === 2 ? (
            <SplitTerminalView layout={layout} sessions={sessions} />
          ) : (
            <SingleTerminalView
              session={sessions.find((s) => s.id === activeSessionId) || sessions[0]}
            />
          )}
        </div>
        {helpOpen && <TerminalHelpDrawer onInsert={insertCommand} canInsert={sessions.length > 0} onClose={() => setHelpOpen(false)} />}
      </div>
    </div>
  );
}

// ──────────────────────────────────────────────
// Terminal Toggle Bar (collapsed state)
// ──────────────────────────────────────────────

function TerminalToggleBar({
  onToggle,
  sessionCount,
}: {
  onToggle: () => void;
  sessionCount: number;
}) {
  return (
    <button
      className={cn(
        "flex items-center gap-2 w-full px-4 py-1",
        "border-t border-[var(--color-border)]",
        "bg-[var(--color-bg-secondary)]",
        "text-[length:var(--font-size-xs)] text-[color:var(--color-text-secondary)]",
        "hover:bg-[var(--color-hover-bg)] transition-theme",
      )}
      onClick={onToggle}
      aria-label="Open terminal"
      data-testid="terminal-toggle-bar"
    >
      <TerminalIcon className="h-3.5 w-3.5" />
      <span>Terminal</span>
      {sessionCount > 0 && (
        <span className="px-1.5 py-0.5 rounded-full bg-[var(--color-primary)] text-[color:var(--color-primary-foreground)] text-[10px] font-medium">
          {sessionCount}
        </span>
      )}
      <ChevronUp className="h-3 w-3 ml-auto" />
    </button>
  );
}

// ──────────────────────────────────────────────
// Session Tab
// ──────────────────────────────────────────────

function SessionTab({
  session,
  active,
  onClick,
  onClose,
}: {
  session: TerminalSession;
  active: boolean;
  onClick: () => void;
  onClose: () => void;
}) {
  return (
    <div
      className={cn(
        "group flex items-center gap-1 px-2 py-1 rounded-t-[var(--radius-sm)]",
        "text-[length:var(--font-size-xs)] cursor-pointer transition-theme",
        active
          ? "bg-[var(--color-bg)] text-[color:var(--color-text)] border border-b-0 border-[var(--color-border)]"
          : "text-[color:var(--color-text-secondary)] hover:text-[color:var(--color-text)] hover:bg-[var(--color-hover-bg)]",
      )}
      onClick={onClick}
      role="tab"
      aria-selected={active}
      data-testid={`terminal-tab-${session.id}`}
    >
      {session.session_type === "remote" ? (
        <Globe className="h-3 w-3 shrink-0" />
      ) : (
        <Monitor className="h-3 w-3 shrink-0" />
      )}
      <span className="truncate max-w-[120px]">{session.title}</span>
      <button
        className={cn(
          "h-4 w-4 flex items-center justify-center rounded-sm shrink-0",
          "opacity-0 group-hover:opacity-100",
          "text-[color:var(--color-text-tertiary)] hover:text-[color:var(--color-error)]",
          "transition-theme",
        )}
        onClick={(e) => {
          e.stopPropagation();
          onClose();
        }}
        aria-label={`Close ${session.title}`}
      >
        <X className="h-3 w-3" />
      </button>
    </div>
  );
}

// ──────────────────────────────────────────────
// New Terminal Menu
// ──────────────────────────────────────────────

function NewTerminalMenu({
  onCreateLocal,
  onCreateRemote,
  onClose,
}: {
  onCreateLocal: () => void;
  onCreateRemote: () => void;
  onClose: () => void;
}) {
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    function handleClickOutside(e: MouseEvent) {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        onClose();
      }
    }
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, [onClose]);

  return (
    <div
      ref={menuRef}
      className={cn(
        "absolute top-full left-0 mt-1 z-50",
        "bg-[var(--color-bg-elevated)] border border-[var(--color-border)]",
        "rounded-[var(--radius-md)] shadow-lg py-1 min-w-[180px]",
      )}
      role="menu"
      data-testid="new-terminal-menu"
    >
      <button
        className={cn(
          "flex items-center gap-2 w-full px-3 py-1.5 text-left",
          "text-[length:var(--font-size-xs)] text-[color:var(--color-text)]",
          "hover:bg-[var(--color-hover-bg)] transition-theme",
        )}
        onClick={onCreateLocal}
        role="menuitem"
      >
        <Monitor className="h-3.5 w-3.5 text-[color:var(--color-text-secondary)]" />
        Local Terminal
      </button>
      <button
        className={cn(
          "flex items-center gap-2 w-full px-3 py-1.5 text-left",
          "text-[length:var(--font-size-xs)] text-[color:var(--color-text)]",
          "hover:bg-[var(--color-hover-bg)] transition-theme",
        )}
        onClick={onCreateRemote}
        role="menuitem"
      >
        <Globe className="h-3.5 w-3.5 text-[color:var(--color-text-secondary)]" />
        SSH Remote Terminal
      </button>
    </div>
  );
}

// ──────────────────────────────────────────────
// Empty Terminal State
// ──────────────────────────────────────────────

function EmptyTerminal({ onCreateLocal }: { onCreateLocal: () => void }) {
  return (
    <div className="flex flex-col items-center justify-center h-full bg-[#1e1e1e] text-[#cccccc]">
      <TerminalIcon className="h-10 w-10 mb-3 opacity-30" />
      <p className="text-sm mb-1">No terminal sessions</p>
      <p className="text-xs text-[#888888] mb-4">
        Create a new terminal to get started
      </p>
      <button
        className={cn(
          "px-4 py-2 rounded-[var(--radius-md)]",
          "bg-[#007acc] text-white text-xs font-medium",
          "hover:bg-[#1a8adb] transition-colors",
        )}
        onClick={() => onCreateLocal()}
        data-testid="create-first-terminal"
      >
        New Terminal
      </button>
    </div>
  );
}

// ──────────────────────────────────────────────
// Single Terminal View
// ──────────────────────────────────────────────

function SingleTerminalView({ session }: { session: TerminalSession }) {
  return (
    <div
      className="flex flex-col h-full bg-[#1e1e1e] text-[#cccccc] font-mono"
      data-testid={`terminal-view-${session.id}`}
    >
      <div className="flex items-center gap-2 px-3 py-1 text-[10px] text-[#666666] border-b border-[#333333]">
        <span>{session.shell}</span>
        <span>|</span>
        <span>{session.cwd}</span>
      </div>
      <div className="flex-1 min-h-0">
        <XtermView sessionId={session.id} />
      </div>
    </div>
  );
}

// ──────────────────────────────────────────────
// Split Terminal View
// ──────────────────────────────────────────────

function SplitTerminalView({
  layout,
  sessions,
}: {
  layout: { session_ids: string[]; orientation: string; split_ratio: number };
  sessions: TerminalSession[];
}) {
  const session1 = sessions.find((s) => s.id === layout.session_ids[0]);
  const session2 = sessions.find((s) => s.id === layout.session_ids[1]);

  if (!session1 || !session2) {
    return session1 ? <SingleTerminalView session={session1} /> : null;
  }

  const isHorizontal = layout.orientation === "horizontal";
  const ratio = layout.split_ratio * 100;

  return (
    <div
      className={cn(
        "flex h-full",
        isHorizontal ? "flex-col" : "flex-row",
      )}
      data-testid="split-terminal-view"
    >
      <div
        style={
          isHorizontal
            ? { height: `${ratio}%` }
            : { width: `${ratio}%` }
        }
        className="overflow-hidden"
      >
        <SingleTerminalView session={session1} />
      </div>

      {/* Divider */}
      <div
        className={cn(
          isHorizontal
            ? "h-px w-full bg-[#444444]"
            : "w-px h-full bg-[#444444]",
        )}
      />

      <div
        style={
          isHorizontal
            ? { height: `${100 - ratio}%` }
            : { width: `${100 - ratio}%` }
        }
        className="overflow-hidden"
      >
        <SingleTerminalView session={session2} />
      </div>
    </div>
  );
}
