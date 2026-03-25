/**
 * Automation Panel (Quickflows)
 *
 * Right-side panel for creating and managing automation rules.
 * Features: NL rule creation, rule list with toggles, rule editor, execution logs.
 */
import { useCallback, useEffect, useState } from "react";
import {
  useAutomationStore,
  type AutomationRule,
  type RuleTrigger,
  type RuleAction,
  type RuleCondition,
  type AutomationLog,
} from "@/stores/automation-store";
import { cn } from "@ufop/ui-components";
import { Button, Badge, ScrollArea } from "@ufop/ui-components";
import {
  X,
  Plus,
  Play,
  Trash2,
  ChevronDown,
  ChevronRight,
  Zap,
  FolderInput,
  Clock,
  Hand,
  AlertTriangle,
  CheckCircle2,
  XCircle,
  Loader2,
  Send,
  Pencil,
  ToggleLeft,
  ToggleRight,
} from "lucide-react";

// ── NL Input Bar ──

function NLInputBar() {
  const [input, setInput] = useState("");
  const parseNaturalLanguage = useAutomationStore((s) => s.parseNaturalLanguage);
  const nlParsing = useAutomationStore((s) => s.nlParsing);

  const handleSubmit = useCallback(async () => {
    if (!input.trim() || nlParsing) return;
    await parseNaturalLanguage(input.trim());
    setInput("");
  }, [input, nlParsing, parseNaturalLanguage]);

  return (
    <div className="p-3 border-b border-[var(--color-border)]">
      <div className="flex gap-2">
        <input
          type="text"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              handleSubmit();
            }
          }}
          placeholder='Describe a rule... (e.g., "when PDFs appear in Downloads, move to Documents")'
          className="flex-1 text-xs bg-[var(--color-bg)] border border-[var(--color-border)] rounded px-2 py-1.5 text-[color:var(--color-text)] placeholder:text-[var(--color-text-muted)] focus:outline-none focus:ring-1 focus:ring-[var(--color-primary)]"
          disabled={nlParsing}
          aria-label="Describe an automation rule in natural language"
        />
        <Button
          variant="ghost"
          size="icon"
          onClick={handleSubmit}
          disabled={!input.trim() || nlParsing}
          title="Create rule from description"
        >
          {nlParsing ? (
            <Loader2 className="h-4 w-4 animate-spin" aria-hidden="true" />
          ) : (
            <Send className="h-4 w-4" aria-hidden="true" />
          )}
        </Button>
      </div>
    </div>
  );
}

// ── Trigger Label ──

function triggerLabel(trigger: RuleTrigger): string {
  switch (trigger.type) {
    case "file_appears":
      return `File appears in ${shortenPath(trigger.watch_path)}`;
    case "file_modified":
      return `File modified in ${shortenPath(trigger.watch_path)}`;
    case "file_deleted":
      return `File deleted in ${shortenPath(trigger.watch_path)}`;
    case "schedule":
      return trigger.cron_human || `Schedule: ${trigger.cron_expr}`;
    case "manual":
      return "Manual trigger";
  }
}

function actionLabel(action: RuleAction): string {
  switch (action.type) {
    case "move_file":
      return `Move to ${shortenPath(action.dest_path)}`;
    case "copy_file":
      return `Copy to ${shortenPath(action.dest_path)}`;
    case "rename_file":
      return `Rename (${action.pattern})`;
    case "delete_file":
      return action.to_trash ? "Move to trash" : "Delete permanently";
    case "compress_folder":
      return `Compress to ${action.format}`;
    case "run_sync":
      return `Run sync pair`;
    case "transfer_to_connector":
      return `Transfer via ${action.connector_protocol}`;
    case "execute_custom_command":
      return "Run custom command";
    case "notify":
      return `Notify: ${action.title}`;
  }
}

function triggerIcon(trigger: RuleTrigger) {
  switch (trigger.type) {
    case "file_appears":
    case "file_modified":
    case "file_deleted":
      return <FolderInput className="h-3.5 w-3.5" aria-hidden="true" />;
    case "schedule":
      return <Clock className="h-3.5 w-3.5" aria-hidden="true" />;
    case "manual":
      return <Hand className="h-3.5 w-3.5" aria-hidden="true" />;
  }
}

function shortenPath(path: string): string {
  if (path.length <= 30) return path;
  const parts = path.split("/");
  if (parts.length <= 2) return path;
  return `.../${parts.slice(-2).join("/")}`;
}

// ── Rule Card ──

interface RuleCardProps {
  rule: AutomationRule;
}

function RuleCard({ rule }: RuleCardProps) {
  const toggleRule = useAutomationStore((s) => s.toggleRule);
  const runRule = useAutomationStore((s) => s.runRule);
  const deleteRule = useAutomationStore((s) => s.deleteRule);
  const openEditor = useAutomationStore((s) => s.openEditor);
  const selectRule = useAutomationStore((s) => s.selectRule);
  const selectedRuleId = useAutomationStore((s) => s.selectedRuleId);
  const [expanded, setExpanded] = useState(false);
  const [running, setRunning] = useState(false);

  const isSelected = selectedRuleId === rule.id;

  const handleRun = useCallback(async () => {
    setRunning(true);
    await runRule(rule.id);
    setRunning(false);
  }, [rule.id, runRule]);

  return (
    <div
      className={cn(
        "border border-[var(--color-border)] rounded-lg p-3 transition-colors",
        isSelected ? "bg-[var(--color-bg-elevated)] ring-1 ring-[var(--color-primary)]" : "bg-[var(--color-bg)]",
        !rule.enabled && "opacity-60",
      )}
      onClick={() => selectRule(isSelected ? null : rule.id)}
      role="button"
      tabIndex={0}
      aria-label={`Automation rule: ${rule.name}`}
      onKeyDown={(e) => {
        if (e.key === "Enter") selectRule(isSelected ? null : rule.id);
      }}
    >
      {/* Header */}
      <div className="flex items-start gap-2">
        <button
          onClick={(e) => {
            e.stopPropagation();
            setExpanded(!expanded);
          }}
          className="mt-0.5 text-[var(--color-text-muted)]"
          aria-label={expanded ? "Collapse" : "Expand"}
        >
          {expanded ? <ChevronDown className="h-3.5 w-3.5" /> : <ChevronRight className="h-3.5 w-3.5" />}
        </button>

        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <span className="text-xs font-medium text-[color:var(--color-text)] truncate">
              {rule.name}
            </span>
            {rule.error_count > 0 && (
              <Badge variant="destructive" className="text-[10px] px-1 py-0">
                {rule.error_count} err
              </Badge>
            )}
          </div>
          <div className="flex items-center gap-1.5 mt-1 text-[10px] text-[var(--color-text-muted)]">
            {triggerIcon(rule.trigger)}
            <span>{triggerLabel(rule.trigger)}</span>
            <span className="text-[var(--color-text-muted)]">→</span>
            <span>{actionLabel(rule.action)}</span>
          </div>
        </div>

        {/* Toggle */}
        <button
          onClick={(e) => {
            e.stopPropagation();
            toggleRule(rule.id, !rule.enabled);
          }}
          className={cn(
            "text-lg",
            rule.enabled ? "text-green-500" : "text-[var(--color-text-muted)]",
          )}
          title={rule.enabled ? "Disable rule" : "Enable rule"}
          aria-label={rule.enabled ? "Disable rule" : "Enable rule"}
        >
          {rule.enabled ? (
            <ToggleRight className="h-5 w-5" />
          ) : (
            <ToggleLeft className="h-5 w-5" />
          )}
        </button>
      </div>

      {/* Expanded details */}
      {expanded && (
        <div className="mt-2 pt-2 border-t border-[var(--color-border)]">
          {/* Stats */}
          <div className="flex items-center gap-3 text-[10px] text-[var(--color-text-muted)] mb-2">
            <span>Triggered: {rule.trigger_count}x</span>
            {rule.last_triggered && (
              <span>Last: {new Date(rule.last_triggered).toLocaleString()}</span>
            )}
          </div>

          {/* Conditions */}
          {rule.conditions.length > 0 && (
            <div className="text-[10px] text-[var(--color-text-muted)] mb-2">
              <span className="font-medium">Conditions: </span>
              {rule.conditions.map((c, i) => (
                <span key={i}>
                  {i > 0 && " AND "}
                  {conditionLabel(c)}
                </span>
              ))}
            </div>
          )}

          {/* Last error */}
          {rule.last_error && (
            <div className="flex items-start gap-1 text-[10px] text-red-400 mb-2">
              <AlertTriangle className="h-3 w-3 mt-0.5 shrink-0" />
              <span className="truncate">{rule.last_error}</span>
            </div>
          )}

          {/* Actions */}
          <div className="flex items-center gap-1">
            <Button
              variant="ghost"
              size="sm"
              onClick={(e) => {
                e.stopPropagation();
                handleRun();
              }}
              disabled={running}
              className="h-6 text-[10px]"
            >
              {running ? <Loader2 className="h-3 w-3 animate-spin mr-1" /> : <Play className="h-3 w-3 mr-1" />}
              Run
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={(e) => {
                e.stopPropagation();
                openEditor(rule);
              }}
              className="h-6 text-[10px]"
            >
              <Pencil className="h-3 w-3 mr-1" />
              Edit
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={(e) => {
                e.stopPropagation();
                if (confirm(`Delete rule "${rule.name}"?`)) {
                  deleteRule(rule.id);
                }
              }}
              className="h-6 text-[10px] text-red-400 hover:text-red-500"
            >
              <Trash2 className="h-3 w-3 mr-1" />
              Delete
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}

function conditionLabel(condition: RuleCondition): string {
  switch (condition.type) {
    case "extension_is":
      return `ext = ${condition.extensions.join(", ")}`;
    case "name_matches":
      return `name ~ ${condition.pattern}`;
    case "size_greater_than":
      return `size > ${formatBytes(condition.bytes)}`;
    case "size_less_than":
      return `size < ${formatBytes(condition.bytes)}`;
    case "created_within":
      return `age < ${condition.seconds}s`;
    case "path_contains":
      return `path contains "${condition.substring}"`;
  }
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1048576) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1073741824) return `${(bytes / 1048576).toFixed(1)} MB`;
  return `${(bytes / 1073741824).toFixed(1)} GB`;
}

// ── Log Entry ──

function LogEntry({ log }: { log: AutomationLog }) {
  const statusIcon = {
    success: <CheckCircle2 className="h-3 w-3 text-green-500" />,
    failed: <XCircle className="h-3 w-3 text-red-500" />,
    skipped: <XCircle className="h-3 w-3 text-yellow-500" />,
    running: <Loader2 className="h-3 w-3 text-blue-500 animate-spin" />,
  };

  return (
    <div className="flex items-start gap-2 py-1.5 border-b border-[var(--color-border)] last:border-0">
      {statusIcon[log.status]}
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2 text-[10px]">
          <span className="font-medium text-[color:var(--color-text)]">{log.rule_name}</span>
          <span className="text-[var(--color-text-muted)]">
            {new Date(log.triggered_at).toLocaleTimeString()}
          </span>
        </div>
        {log.error_message && (
          <div className="text-[10px] text-red-400 truncate">{log.error_message}</div>
        )}
        {log.files_affected.length > 0 && (
          <div className="text-[10px] text-[var(--color-text-muted)] truncate">
            {log.files_affected.length} file(s) affected
          </div>
        )}
      </div>
    </div>
  );
}

// ── Main Panel ──

export function AutomationPanel() {
  const panelOpen = useAutomationStore((s) => s.panelOpen);
  const togglePanel = useAutomationStore((s) => s.togglePanel);
  const rules = useAutomationStore((s) => s.rules);
  const logs = useAutomationStore((s) => s.logs);
  const loading = useAutomationStore((s) => s.loading);
  const loadRules = useAutomationStore((s) => s.loadRules);
  const loadLogs = useAutomationStore((s) => s.loadLogs);
  const openEditor = useAutomationStore((s) => s.openEditor);

  const [activeTab, setActiveTab] = useState<"rules" | "logs">("rules");

  // Load rules on mount
  useEffect(() => {
    if (panelOpen) {
      loadRules();
      loadLogs();
    }
  }, [panelOpen, loadRules, loadLogs]);

  if (!panelOpen) return null;

  return (
    <aside
      className="flex flex-col border-l border-[var(--color-border)] bg-[var(--color-bg-secondary)]"
      style={{ width: 340 }}
      role="complementary"
      aria-label="Automation panel"
    >
      {/* Header */}
      <div className="flex items-center justify-between px-3 py-2 border-b border-[var(--color-border)]">
        <div className="flex items-center gap-2">
          <Zap className="h-4 w-4 text-yellow-500" aria-hidden="true" />
          <span className="text-sm font-semibold text-[color:var(--color-text)]">Quickflows</span>
          <Badge variant="secondary" className="text-[10px]">
            {rules.length}
          </Badge>
        </div>
        <div className="flex items-center gap-1">
          <Button
            variant="ghost"
            size="icon"
            onClick={() => openEditor()}
            title="Create new rule"
            aria-label="Create new automation rule"
          >
            <Plus className="h-4 w-4" />
          </Button>
          <Button
            variant="ghost"
            size="icon"
            onClick={togglePanel}
            title="Close panel"
            aria-label="Close automation panel"
          >
            <X className="h-4 w-4" />
          </Button>
        </div>
      </div>

      {/* NL Input */}
      <NLInputBar />

      {/* Tabs */}
      <div className="flex border-b border-[var(--color-border)]">
        <button
          className={cn(
            "flex-1 text-xs py-2 font-medium transition-colors",
            activeTab === "rules"
              ? "text-[var(--color-primary)] border-b-2 border-[var(--color-primary)]"
              : "text-[var(--color-text-muted)] hover:text-[var(--color-text)]",
          )}
          onClick={() => setActiveTab("rules")}
        >
          Rules ({rules.length})
        </button>
        <button
          className={cn(
            "flex-1 text-xs py-2 font-medium transition-colors",
            activeTab === "logs"
              ? "text-[var(--color-primary)] border-b-2 border-[var(--color-primary)]"
              : "text-[var(--color-text-muted)] hover:text-[var(--color-text)]",
          )}
          onClick={() => setActiveTab("logs")}
        >
          Logs ({logs.length})
        </button>
      </div>

      {/* Content */}
      <ScrollArea className="flex-1">
        <div className="p-3 space-y-2">
          {activeTab === "rules" && (
            <>
              {loading && (
                <div className="flex items-center justify-center py-8 text-[var(--color-text-muted)]">
                  <Loader2 className="h-5 w-5 animate-spin" />
                </div>
              )}
              {!loading && rules.length === 0 && (
                <div className="text-center py-8 text-xs text-[var(--color-text-muted)]">
                  <Zap className="h-8 w-8 mx-auto mb-2 opacity-30" />
                  <p className="font-medium">No automation rules yet</p>
                  <p className="mt-1">
                    Describe a rule above or click{" "}
                    <button
                      className="text-[var(--color-primary)] underline"
                      onClick={() => openEditor()}
                    >
                      + New Rule
                    </button>
                  </p>
                </div>
              )}
              {!loading &&
                rules.map((rule) => <RuleCard key={rule.id} rule={rule} />)}
            </>
          )}

          {activeTab === "logs" && (
            <>
              {logs.length === 0 && (
                <div className="text-center py-8 text-xs text-[var(--color-text-muted)]">
                  <p>No execution logs yet.</p>
                  <p className="mt-1">Run a rule to see results here.</p>
                </div>
              )}
              {logs.map((log) => (
                <LogEntry key={log.id} log={log} />
              ))}
            </>
          )}
        </div>
      </ScrollArea>
    </aside>
  );
}
