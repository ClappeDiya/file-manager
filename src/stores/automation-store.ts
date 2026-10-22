/**
 * Automation Store (Quickflows)
 *
 * Zustand store for automation rule management.
 * Handles CRUD, enable/disable, manual triggers, and NL creation.
 */
import { create } from "zustand";
import { persist } from "zustand/middleware";
import { tauriInvoke } from "@/hooks/use-tauri";

// ── Types (mirrors Rust enums) ──

export type RuleTrigger =
  | { type: "file_appears"; watch_path: string; recursive: boolean }
  | { type: "file_modified"; watch_path: string; recursive: boolean }
  | { type: "file_deleted"; watch_path: string; recursive: boolean }
  | { type: "schedule"; cron_expr: string; cron_human?: string | null }
  | { type: "manual" }
  | { type: "webhook" };

export type RuleCondition =
  | { type: "extension_is"; extensions: string[] }
  | { type: "name_matches"; pattern: string }
  | { type: "size_greater_than"; bytes: number }
  | { type: "size_less_than"; bytes: number }
  | { type: "created_within"; seconds: number }
  | { type: "path_contains"; substring: string }
  | { type: "older_than"; seconds: number }
  | { type: "modified_within"; seconds: number }
  | { type: "name_regex"; pattern: string };

/** AND/OR/NOT tree of conditions (mirrors `ConditionNode`). */
export type ConditionNode =
  | { type: "all"; nodes: ConditionNode[] }
  | { type: "any"; nodes: ConditionNode[] }
  | { type: "not"; node: ConditionNode }
  | { type: "condition"; condition: RuleCondition };

/** A backup-style notification channel (mirrors `notify::Notification`);
 *  its secret lives in the credential store under `credential_id`. */
export type MessageNotification = {
  credential_id: string;
  send_on?: string[];
  template?: string;
} & (
  | { type: "email"; host: string; port: number | null; tls: "start_tls" | "implicit" | "none"; username: string; from: string; to: string[]; subject?: string }
  | { type: "slack" }
  | { type: "teams" }
  | { type: "telegram"; chat_id: string; api_base?: string }
);

export type RuleAction =
  | { type: "move_file"; dest_path: string }
  | { type: "copy_file"; dest_path: string }
  | { type: "rename_file"; pattern: string }
  | { type: "delete_file"; to_trash: boolean }
  | { type: "compress_folder"; dest_path: string; format: string }
  | { type: "run_sync"; sync_pair_id: string }
  | { type: "transfer_to_connector"; connector_protocol: string; connection_id: string; remote_path: string }
  | { type: "execute_custom_command"; command_id: string }
  | { type: "notify"; title: string; body: string }
  | { type: "create_folder"; path: string }
  | { type: "extract_archive"; dest_path: string; max_entries: number; delete_archive: boolean }
  | { type: "run_command"; program: string; args: string[]; working_dir: string | null; timeout_secs: number }
  | { type: "send_message"; notification: MessageNotification; title: string; body: string }
  /** Replay a captured set of file operations exactly as the user once
   *  performed them. Created by the **Operation Pin** flow from a single
   *  ledger event — carries its own source list so the trigger doesn't
   *  need to provide a file. Mirrors `RuleAction::ReplayOp` in the Rust
   *  automation engine. */
  | { type: "replay_op"; kind: string; source_paths: string[]; dest_path: string }
  /** Download a URL into a folder (templates allowed in url, dest_path, file_name, sha256). */
  | {
      type: "download_url";
      url: string;
      dest_path: string;
      file_name: string | null;
      overwrite: boolean;
      max_mb: number;
      sha256: string | null;
      allow_private: boolean;
    };

export type OverwritePolicy =
  | "rename"
  | "skip_always"
  | "overwrite_always"
  | "overwrite_if_newer"
  | "overwrite_if_larger"
  | "ask_each_time";

export interface RetryPolicy {
  retries: number;
  delay_ms: number;
  exponential: boolean;
  max_delay_ms: number;
}

export interface SourceSelection {
  path: string;
  recursive: boolean;
  newest: number | null;
  include_dirs: boolean;
}

export interface RuleOptions {
  overwrite: OverwritePolicy;
  retry: RetryPolicy;
  priority: number;
  allow_overlap: boolean;
  settle_secs: number;
  source?: SourceSelection | null;
  /** Set only by `approve_automation_command` (read-only for the UI). */
  command_approval?: string | null;
  webhook_secret_hash?: string | null;
}

export interface ActionStep {
  action: RuleAction;
  continue_on_error: boolean;
}

export interface AutomationRule {
  id: string;
  name: string;
  enabled: boolean;
  trigger: RuleTrigger;
  conditions: RuleCondition[];
  action: RuleAction;
  created_at: string;
  updated_at: string;
  last_triggered: string | null;
  trigger_count: number;
  error_count: number;
  last_error: string | null;
  condition_tree?: ConditionNode | null;
  steps?: ActionStep[];
  options?: RuleOptions;
  /** When set, the rule runs this graph instead of `action` + `steps`. */
  workflow?: Workflow | null;
  /** Organization that owns the rule (managed settings); read-only here. */
  managed_by?: string | null;
}

/** `automation_engine::workflow`: a rule's actions as a DAG. */
export type EdgeOn = "success" | "failure" | "always" | "true" | "false";

export type WorkflowNodeKind =
  | { kind: "action"; action: RuleAction }
  | { kind: "condition"; condition: ConditionNode }
  | { kind: "delay"; seconds: number }
  | { kind: "join"; all: boolean }
  | { kind: "sub_rule"; rule_id: string };

export type WorkflowNode = WorkflowNodeKind & { id: string; label: string; x: number; y: number };

export interface WorkflowEdge {
  from: string;
  to: string;
  on: EdgeOn;
}

export interface Workflow {
  nodes: WorkflowNode[];
  edges: WorkflowEdge[];
}

export interface RuleRevision {
  revision: number;
  saved_at: string;
  name: string;
  workflow: boolean;
  actions: number;
  rule: AutomationRule;
}

export interface StepResult {
  step: number;
  action: string;
  file: string | null;
  ok: boolean;
  attempts: number;
  message: string | null;
  /** Workflow step id (workflow runs). */
  node?: string | null;
}

export interface AutomationLog {
  id: string;
  rule_id: string;
  rule_name: string;
  triggered_at: string;
  completed_at: string | null;
  trigger_event: string;
  status: "success" | "failed" | "skipped" | "running";
  files_affected: string[];
  error_message: string | null;
  attempts?: number;
  steps?: StepResult[];
}

/** URL for a webhook-triggered rule (the token is shown only once). */
export interface WebhookInfo {
  url: string;
  token: string;
  listening: boolean;
}

/**
 * A pattern surfaced by the Quickflow Suggester. Mirrors
 * `automation_engine::suggester::QuickflowSuggestion` on the Rust side.
 */
export interface QuickflowSuggestion {
  id: string;
  kind: "copy" | "move";
  source_dir: string;
  dest_dir: string;
  hit_count: number;
  last_seen: string;
  sample_files: string[];
  suggested_name: string;
}

// ── Store ──

interface AutomationState {
  // UI state (persisted)
  panelOpen: boolean;
  selectedRuleId: string | null;
  editorOpen: boolean;

  // Data (not persisted - loaded from backend)
  rules: AutomationRule[];
  logs: AutomationLog[];
  suggestions: QuickflowSuggestion[];
  loading: boolean;
  nlParsing: boolean;
  editorRule: Partial<AutomationRule> | null;

  // UI Actions
  togglePanel: () => void;
  openEditor: (rule?: Partial<AutomationRule>) => void;
  closeEditor: () => void;
  selectRule: (id: string | null) => void;

  // Data Actions
  loadRules: () => Promise<void>;
  /** Resolve to the saved rule; reject with the backend's message. */
  createRule: (rule: AutomationRule) => Promise<AutomationRule>;
  updateRule: (rule: AutomationRule) => Promise<AutomationRule>;
  cloneRule: (id: string) => Promise<AutomationRule | null>;
  approveCommand: (id: string) => Promise<AutomationRule>;
  rotateWebhook: (id: string) => Promise<WebhookInfo>;
  deleteRule: (id: string) => Promise<void>;
  toggleRule: (id: string, enabled: boolean) => Promise<void>;
  runRule: (id: string) => Promise<AutomationLog | null>;
  testRule: (rule: AutomationRule) => Promise<AutomationLog | null>;
  loadLogs: (ruleId?: string) => Promise<void>;
  clearLogs: (ruleId?: string) => Promise<void>;
  parseNaturalLanguage: (input: string) => Promise<AutomationRule | null>;

  // Quickflow Suggester (ledger → automation bridge)
  loadSuggestions: () => Promise<void>;
  acceptSuggestion: (id: string) => Promise<AutomationRule | null>;
  dismissSuggestion: (id: string) => Promise<void>;

  /** Operation Pin — turn a single past ledger event (identified by its
   *  correlation_id) into a saved manual-trigger Quickflow that can be
   *  re-run on demand. Returns the new (disabled-by-default) rule, or
   *  null on backend rejection. The created rule is also pushed into
   *  `rules[]` so the automation panel re-renders without a refetch. */
  pinLedgerEvent: (correlationId: string) => Promise<AutomationRule | null>;

  /** Operation Retry — failure-side mirror of Pin. Re-attempts a failed
   *  (or cancelled) fs.copy / fs.move event identified by its
   *  correlation_id, without persisting a saved rule. Returns the
   *  executor's `AutomationLog` (success/failed + files affected +
   *  error message) so the caller can render a precise toast, or null
   *  on backend rejection. The retry attempt is recorded to the unified
   *  ledger so it appears on the activity timeline next to its cause. */
  retryFailedEvent: (correlationId: string) => Promise<AutomationLog | null>;
}

export const useAutomationStore = create<AutomationState>()(
  persist(
    (set, get) => ({
      // UI state
      panelOpen: false,
      selectedRuleId: null,
      editorOpen: false,

      // Data
      rules: [],
      logs: [],
      suggestions: [],
      loading: false,
      nlParsing: false,
      editorRule: null,

      // UI Actions
      togglePanel: () => set((s) => ({ panelOpen: !s.panelOpen })),

      openEditor: (rule) =>
        set({ editorOpen: true, editorRule: rule || null }),

      closeEditor: () =>
        set({ editorOpen: false, editorRule: null }),

      selectRule: (id) => set({ selectedRuleId: id }),

      // Data Actions
      loadRules: async () => {
        set({ loading: true });
        try {
          const rules = await tauriInvoke<AutomationRule[]>("list_automation_rules");
          set({ rules, loading: false });
        } catch (e) {
          console.error("Failed to load automation rules:", e);
          set({ loading: false });
        }
      },

      createRule: async (rule) => {
        const saved = await tauriInvoke<AutomationRule>("create_automation_rule", { rule });
        await get().loadRules();
        return saved;
      },

      updateRule: async (rule) => {
        const saved = await tauriInvoke<AutomationRule>("update_automation_rule", { rule });
        await get().loadRules();
        return saved;
      },

      cloneRule: async (id) => {
        try {
          const copy = await tauriInvoke<AutomationRule>("clone_automation_rule", { ruleId: id });
          await get().loadRules();
          return copy;
        } catch (e) {
          console.error("Failed to clone automation rule:", e);
          return null;
        }
      },

      approveCommand: async (id) => {
        const rule = await tauriInvoke<AutomationRule>("approve_automation_command", { ruleId: id });
        await get().loadRules();
        return rule;
      },

      rotateWebhook: async (id) => {
        const info = await tauriInvoke<WebhookInfo>("rotate_automation_webhook", { ruleId: id });
        await get().loadRules();
        return info;
      },

      deleteRule: async (id) => {
        try {
          await tauriInvoke("delete_automation_rule", { ruleId: id });
          set((s) => ({
            rules: s.rules.filter((r) => r.id !== id),
            selectedRuleId: s.selectedRuleId === id ? null : s.selectedRuleId,
          }));
        } catch (e) {
          console.error("Failed to delete automation rule:", e);
        }
      },

      toggleRule: async (id, enabled) => {
        try {
          await tauriInvoke("enable_automation_rule", { ruleId: id, enabled });
          set((s) => ({
            rules: s.rules.map((r) =>
              r.id === id ? { ...r, enabled } : r
            ),
          }));
        } catch (e) {
          console.error("Failed to toggle automation rule:", e);
        }
      },

      runRule: async (id) => {
        try {
          const log = await tauriInvoke<AutomationLog>("run_automation_rule", { ruleId: id });
          await get().loadRules();
          await get().loadLogs(id);
          return log;
        } catch (e) {
          console.error("Failed to run automation rule:", e);
          return null;
        }
      },

      testRule: async (rule) => {
        try {
          const log = await tauriInvoke<AutomationLog>("test_automation_rule", { rule });
          return log;
        } catch (e) {
          console.error("Failed to test automation rule:", e);
          return null;
        }
      },

      loadLogs: async (ruleId) => {
        try {
          const logs = await tauriInvoke<AutomationLog[]>("list_automation_logs", {
            ruleId: ruleId || null,
            limit: 50,
          });
          set({ logs });
        } catch (e) {
          console.error("Failed to load automation logs:", e);
        }
      },

      clearLogs: async (ruleId) => {
        try {
          await tauriInvoke("clear_automation_logs", { ruleId: ruleId || null });
          set({ logs: [] });
        } catch (e) {
          console.error("Failed to clear automation logs:", e);
        }
      },

      parseNaturalLanguage: async (input) => {
        set({ nlParsing: true });
        try {
          const rule = await tauriInvoke<AutomationRule>("parse_automation_nl", { input });
          set({ nlParsing: false, editorOpen: true, editorRule: rule });
          return rule;
        } catch (e) {
          console.error("Failed to parse automation NL:", e);
          set({ nlParsing: false });
          return null;
        }
      },

      loadSuggestions: async () => {
        try {
          const suggestions = await tauriInvoke<QuickflowSuggestion[]>(
            "quickflow_suggestions_list",
            undefined,
            [],
          );
          set({ suggestions });
        } catch (e) {
          console.error("Failed to load Quickflow suggestions:", e);
        }
      },

      acceptSuggestion: async (id) => {
        try {
          const rule = await tauriInvoke<AutomationRule>(
            "quickflow_suggestion_accept",
            { id },
          );
          set((s) => ({
            suggestions: s.suggestions.filter((sg) => sg.id !== id),
          }));
          await get().loadRules();
          return rule;
        } catch (e) {
          console.error("Failed to accept Quickflow suggestion:", e);
          return null;
        }
      },

      dismissSuggestion: async (id) => {
        try {
          await tauriInvoke("quickflow_suggestion_dismiss", { id });
          set((s) => ({
            suggestions: s.suggestions.filter((sg) => sg.id !== id),
          }));
        } catch (e) {
          console.error("Failed to dismiss Quickflow suggestion:", e);
        }
      },

      pinLedgerEvent: async (correlationId) => {
        try {
          const rule = await tauriInvoke<AutomationRule>("pin_ledger_event", {
            correlationId,
          });
          // Push the new rule into the in-memory list so the automation
          // panel re-renders without a round-trip refetch. The backend has
          // already persisted it to SQLite via `mgr.save_rule`.
          set((s) => ({ rules: [rule, ...s.rules] }));
          return rule;
        } catch (e) {
          console.error("Failed to pin ledger event:", e);
          return null;
        }
      },

      retryFailedEvent: async (correlationId) => {
        try {
          // Ephemeral execution — the backend does NOT persist a rule.
          // It records a `kind="retry"` row to the unified ledger, so
          // the timeline auto-refresh will surface the attempt on the
          // next ledger-tail poll.
          const log = await tauriInvoke<AutomationLog>("retry_failed_event", {
            correlationId,
          });
          return log;
        } catch (e) {
          console.error("Failed to retry ledger event:", e);
          return null;
        }
      },
    }),
    {
      name: "ufop-automation",
      partialize: (state) => ({
        panelOpen: state.panelOpen,
      }),
    },
  ),
);
